#!/usr/bin/env bash
# Barrière de preuve de l'usine.
# Usage : scripts/verify-factory.sh [--comment N] [--release-size] <issue> [sha] [fichier]
#
# Imprime un résumé markdown : le SHA vérifié (git rev-parse HEAD), une ligne
# par étape (code de sortie, résultat et durée en secondes), puis le journal complet
# replié dans <details>. Le même texte est écrit dans
# target/verify-factory/<issue>-<sha>.log. Si un SHA est donné, HEAD doit
# être exactement ce commit. Rust stable requis : 1.99.1.
#
# --comment N publie ce résumé sur le ticket ou la PR N, ou met à jour le
# commentaire qui contient déjà <!-- factory-proof -->. GitHub refuse un
# commentaire de plus de 65536 caractères : dans ce cas le commentaire garde
# le résumé, le début et la fin du journal. Le fichier local reste complet.
#
# --release-size construit aussi grokbot-wrapper en release et consigne sa taille
# en octets. Les mesures sont comparées au dernier journal local dont le SHA
# appartient à origin/main ; une comparaison indisponible est notée « — ».
# Aucune mesure n'impose de seuil de réussite : un échec de ce build release
# est consigné (taille « — ») sans changer le code de sortie.
#
# Enchaîne ensuite, depuis la racine du dépôt :
#   cargo build --workspace --locked
#   cargo fmt --check
#   cargo test --workspace --locked
#   cargo clippy --workspace --all-targets --locked -- -D warnings
#   gitleaks detect --redact --log-opts=origin/main..HEAD
#
# Le corps du ticket peut contenir un bloc ```verify, une commande par ligne.
# Il est lu par `gh issue view` lorsque gh est disponible, sinon par le fichier
# passé en argument. Si gh répond sans bloc et qu'un fichier est donné, ce
# fichier est utilisé. Chaque commande est exécutée et journalisée.
#
# L'arbre de travail doit être propre (fichiers suivis, indexés ou non) : les
# vérifications portent sur l'arbre de travail, pas sur le seul commit.
#
# Sortie 0 seulement si toute la barrière passe et, le cas échéant, si le
# commentaire a été publié. Ne fusionne rien.
set -euo pipefail

export CARGO_TERM_COLOR=never

usage() {
  printf '%s\n' 'Usage: scripts/verify-factory.sh [--comment N] [--release-size] <issue> [sha] [fichier]' >&2
  exit 2
}

comment_pr=""
release_size=0
positional=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --comment)
      if [[ -n "$comment_pr" || $# -lt 2 || ! "$2" =~ ^[0-9]+$ ]]; then
        usage
      fi
      comment_pr=$2
      shift 2
      ;;
    --release-size)
      release_size=1
      shift
      ;;
    --)
      shift
      positional+=("$@")
      break
      ;;
    -*)
      usage
      ;;
    *)
      positional+=("$1")
      shift
      ;;
  esac
done

if [[ ${#positional[@]} -lt 1 || ${#positional[@]} -gt 3 ]]; then
  usage
fi

issue=${positional[0]}
sha_arg=""
criteria_file=""

if [[ ! "$issue" =~ ^[0-9]+$ ]]; then
  usage
fi

if [[ ${#positional[@]} -ge 2 ]]; then
  if [[ "${positional[1]}" =~ ^[0-9a-fA-F]{7,40}$ ]]; then
    sha_arg=${positional[1]}
    if [[ ${#positional[@]} -eq 3 ]]; then
      criteria_file=${positional[2]}
    fi
  elif [[ ${#positional[@]} -eq 2 ]]; then
    criteria_file=${positional[1]}
  else
    usage
  fi
fi

LOG=""
RAW=""
body_file=""
verdict=1
fail=0
criteria_source=""
head_sha=""
step_cmd=()
step_code=()
step_result=()
step_duration=()
comparison_cmd=()
comparison_duration=()
comparison_bytes="—"
release_bytes="—"

finish() {
  local status=$?
  trap - EXIT
  set +e
  if [[ -n "$RAW" && -f "$RAW" ]]; then
    if [[ "$verdict" -eq 0 ]]; then
      printf '\n%s\n' 'verify-factory: ok' >>"$RAW"
    else
      printf '\n%s\n' 'verify-factory: échec' >>"$RAW"
    fi
    if ! render_markdown "$RAW" "$LOG"; then
      status=1
    fi
    cat "$LOG"
    if [[ -n "$comment_pr" ]]; then
      if ! publish_comment "$LOG"; then
        if [[ "$status" -eq 0 ]]; then
          status=1
        fi
      fi
    fi
  fi
  if [[ -n "$body_file" ]]; then
    rm -f "$body_file"
  fi
  if [[ -n "$RAW" ]]; then
    rm -f "$RAW"
  fi
  exit "$status"
}
trap finish EXIT

if [[ -n "$criteria_file" ]]; then
  if [[ ! -f "$criteria_file" ]]; then
    printf 'fichier introuvable: %s\n' "$criteria_file" >&2
    exit 2
  fi
  criteria_dir=$(cd -- "$(dirname -- "$criteria_file")" && pwd)
  criteria_file="${criteria_dir}/$(basename -- "$criteria_file")"
fi

root=$(git rev-parse --show-toplevel)
cd "$root"

head_sha=$(git rev-parse HEAD)
mkdir -p "$root/target/verify-factory"
LOG="$root/target/verify-factory/${issue}-${head_sha}.log"
RAW=$(mktemp)
: >"$RAW"

emit() {
  printf '%s\n' "$1" >>"$RAW"
}

# Longueurs maximales d'une commande et d'un résultat dans le résumé, pour
# que celui-ci reste loin de la limite de 65536 caractères d'un commentaire.
CMD_MAX=200
RESULT_MAX=300

shorten() {
  local text=$1 max=$2
  if [[ ${#text} -gt $max ]]; then
    text="${text:0:$max}…"
  fi
  printf '%s' "$text"
}

record_step() {
  local cmd=$1 code=$2 result=$3 duration=$4
  cmd=${cmd//\`/\'}
  result=${result//$'\n'/ }
  result=${result//\`/\'}
  cmd=$(shorten "$cmd" "$CMD_MAX")
  result=$(shorten "$result" "$RESULT_MAX")
  step_cmd+=("$cmd")
  step_code+=("$code")
  step_result+=("$result")
  step_duration+=("$duration")
}

# Ne lit que le résumé, jamais le journal brut (qui peut contenir de faux
# en-têtes). Toute erreur de lecture ou de comparaison reste non bloquante.
load_comparison() {
  local duration cmd
  while IFS=$'\t' read -r duration cmd; do
    if [[ "$duration" == size ]]; then
      comparison_bytes="${cmd} octets"
    else
      comparison_cmd+=("$cmd")
      comparison_duration+=("$duration")
    fi
  done < <(python3 - "$root/target/verify-factory" <<'PY' 2>/dev/null || true
from pathlib import Path
import re
import subprocess
import sys

try:
    main = set(subprocess.check_output(["git", "rev-list", "origin/main"], text=True).splitlines())
    candidates = []
    for path in Path(sys.argv[1]).glob("*.log"):
        match = re.fullmatch(r"[0-9]+-([0-9a-f]{40})\.log", path.name)
        if match and match[1] in main:
            try:
                candidates.append((path.stat().st_mtime_ns, path, match[1]))
            except OSError:
                pass
    for _, path, sha in sorted(candidates, reverse=True):
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError):
            continue
        if not text.startswith("<!-- factory-proof -->\n\nSHA `%s`\n" % sha):
            continue
        summary = text.partition("\n<details>\n")[0]
        for line in summary.splitlines():
            step = re.fullmatch(r"- `([^\t]*)` — exit [0-9]+ — .* — ([0-9]+) s — origin/main : (?:[0-9]+ s|—)", line)
            size = re.fullmatch(r"- binaire release : ([0-9]+) octets — origin/main : (?:[0-9]+ octets|—)", line)
            if step:
                print("%s\t%s" % (step[2], step[1]))
            elif size:
                print("size\t%s" % size[1])
        break
except (OSError, subprocess.SubprocessError):
    pass
PY
  )
}

previous_duration() {
  local cmd=$1 i
  for i in "${!comparison_cmd[@]}"; do
    if [[ "${comparison_cmd[$i]}" == "$cmd" ]]; then
      printf '%s s' "${comparison_duration[$i]}"
      return
    fi
  done
  printf '%s' '—'
}

result_line() {
  local file=$1 status=$2 line
  if line=$(awk '
    /^test result: / {
      n++
      if ($0 ~ /test result: FAILED/) bad = 1
      if (match($0, /[0-9]+ passed/)) {
        s = substr($0, RSTART, RLENGTH)
        split(s, a, " ")
        passed += a[1]
      }
      if (match($0, /[0-9]+ failed/)) {
        s = substr($0, RSTART, RLENGTH)
        split(s, a, " ")
        failed += a[1]
      }
      if (match($0, /[0-9]+ ignored/)) {
        s = substr($0, RSTART, RLENGTH)
        split(s, a, " ")
        ignored += a[1]
      }
      if (match($0, /[0-9]+ measured/)) {
        s = substr($0, RSTART, RLENGTH)
        split(s, a, " ")
        measured += a[1]
      }
    }
    END {
      if (n == 0) exit 2
      status = bad ? "FAILED" : "ok"
      printf "test result: %s. %d passed; %d failed; %d ignored; %d measured", status, passed + 0, failed + 0, ignored + 0, measured + 0
    }
  ' "$file"); then
    printf '%s' "$line"
    return 0
  fi
  line=$(grep -F 'Finished `' "$file" | tail -n 1 || true)
  if [[ -n "$line" ]]; then
    printf '%s' "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
    return 0
  fi
  if grep -q 'no leaks found' "$file"; then
    printf '%s' 'no leaks found'
    return 0
  fi
  if grep -q 'leaks found' "$file"; then
    printf '%s' 'leaks found'
    return 0
  fi
  line=$(grep -v '^[[:space:]]*$' "$file" | tail -n 1 || true)
  if [[ -n "$line" ]]; then
    printf '%s' "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
    return 0
  fi
  if [[ "$status" -eq 0 ]]; then
    printf '%s' 'ok'
  else
    printf '%s' 'échec'
  fi
}

render_markdown() {
  local raw=$1 dest=$2 fence='```' i
  while grep -F -q -- "$fence" "$raw"; do
    fence="${fence}\`"
  done
  {
    printf '%s\n\n' '<!-- factory-proof -->'
    printf 'SHA `%s`\n' "$head_sha"
    if [[ ${#step_cmd[@]} -gt 0 ]]; then
      printf '\n'
      for i in "${!step_cmd[@]}"; do
        printf -- '- `%s` — exit %s — %s — %s s — origin/main : %s\n' "${step_cmd[$i]}" "${step_code[$i]}" "${step_result[$i]}" "${step_duration[$i]}" "$(previous_duration "${step_cmd[$i]}")"
      done
    fi
    if [[ "$release_size" -eq 1 ]]; then
      printf -- '- binaire release : %s — origin/main : %s\n' "$release_bytes" "$comparison_bytes"
    fi
    printf '\n<details>\n<summary>Journal complet</summary>\n\n%s\n' "$fence"
    cat "$raw"
    printf '%s\n</details>\n' "$fence"
  } >"$dest"
}

publish_comment() {
  local md=$1 remote spec owner repo list json existing method url
  if ! command -v gh >/dev/null 2>&1; then
    printf '%s\n' 'gh absent: --comment impossible' >&2
    return 1
  fi
  if ! remote=$(git remote get-url origin 2>/dev/null); then
    printf '%s\n' 'origin introuvable: --comment impossible' >&2
    return 1
  fi
  spec=$(printf '%s\n' "$remote" | sed -E 's#[^ ]*github.com[:/]##; s#\.git$##; s#^/+##; s#/$##')
  owner=${spec%%/*}
  repo=${spec#*/}
  if [[ -z "$owner" || -z "$repo" || "$owner" == "$repo" || "$repo" == */* || "$owner" == *[@:]* ]]; then
    printf '%s\n' 'dépôt GitHub illisible' >&2
    return 1
  fi
  json=$(mktemp)
  list=$(mktemp)
  if ! python3 - "$md" "$json" <<'PY'
import json, sys
src, dest = sys.argv[1], sys.argv[2]
text = open(src, encoding="utf-8").read()
limit = 65536
sep = "\n<details>\n<summary>Journal complet</summary>\n\n"

def emit(body, truncated):
    if len(body) > limit:
        body = body[:limit]
        truncated = True
    open(dest, "w", encoding="utf-8").write(json.dumps({"body": body}, ensure_ascii=False))
    if truncated:
        print(
            "commentaire tronqué à %s caractères (limite GitHub 65536)" % len(body),
            file=sys.stderr,
        )
    raise SystemExit(0)

if len(text) <= limit:
    emit(text, False)

idx = text.find(sep)
if idx == -1:
    emit(text, True)

summary = text[:idx]
summary_cap = 30000
if len(summary) > summary_cap:
    kept = []
    size = 0
    for line in summary.splitlines(keepends=True):
        if size + len(line) > summary_cap - 200:
            break
        kept.append(line)
        size += len(line)
    summary = "".join(kept) + "\n[… étapes suivantes omises du résumé …]\n"
rest = text[idx + len(sep):]
fence, newline, after = rest.partition("\n")
closer = "\n" + fence + "\n</details>\n"
if newline != "\n" or not after.endswith(closer):
    emit(summary + sep + after[: limit - len(summary) - len(sep)], True)
raw = after[: -len(closer)]
notice = (
    "Journal tronqué : GitHub limite un commentaire à 65536 caractères. "
    "Journal complet : %s caractères. Début et fin conservés.\n\n" % len(raw)
)
omission = "\n[… milieu omis …]\n"
line_cap = 2000
lines = []
for line in raw.splitlines(keepends=True):
    if len(line) > line_cap:
        line = line[:line_cap] + "[… ligne tronquée …]\n"
    lines.append(line)

def assemble(head_count, tail_count):
    if head_count + tail_count >= len(lines):
        part = "".join(lines)
    elif tail_count == 0:
        part = "".join(lines[:head_count]) + omission
    else:
        part = "".join(lines[:head_count]) + omission + "".join(lines[-tail_count:])
    if part and not part.endswith("\n"):
        part += "\n"
    return summary + sep + notice + fence + "\n" + part + fence + "\n</details>\n"

if not lines:
    emit(assemble(0, 0), True)

tail_count = min(40, len(lines))
while tail_count > 0 and len(assemble(0, tail_count)) > limit:
    tail_count -= 1
best_head = 0
low, high = 0, max(0, len(lines) - tail_count)
while low <= high:
    mid = (low + high) // 2
    if len(assemble(mid, tail_count)) <= limit:
        best_head = mid
        low = mid + 1
    else:
        high = mid - 1
emit(assemble(best_head, tail_count), True)
PY
  then
    printf '%s\n' 'corps du commentaire illisible' >&2
    rm -f "$json" "$list"
    return 1
  fi
  if ! gh api --paginate "repos/${owner}/${repo}/issues/${comment_pr}/comments" >"$list"; then
    printf '%s\n' 'lecture des commentaires impossible' >&2
    rm -f "$json" "$list"
    return 1
  fi
  existing=$(python3 -c '
import json, sys
raw = open(sys.argv[1], encoding="utf-8").read().strip()
if not raw:
    raise SystemExit(0)
decoder = json.JSONDecoder()
idx = 0
n = len(raw)
while idx < n:
    while idx < n and raw[idx].isspace():
        idx += 1
    if idx >= n:
        break
    obj, end = decoder.raw_decode(raw, idx)
    idx = end
    items = obj if isinstance(obj, list) else [obj]
    for comment in items:
        if not isinstance(comment, dict):
            continue
        if "<!-- factory-proof -->" in (comment.get("body") or ""):
            print(comment.get("id", ""))
            raise SystemExit(0)
' "$list" || true)
  if [[ -n "$existing" ]]; then
    method=PATCH
    url="repos/${owner}/${repo}/issues/comments/${existing}"
  else
    method=POST
    url="repos/${owner}/${repo}/issues/${comment_pr}/comments"
  fi
  if ! gh api --method "$method" --input "$json" "$url" >/dev/null; then
    printf '%s\n' 'publication du commentaire impossible' >&2
    rm -f "$json" "$list"
    return 1
  fi
  rm -f "$json" "$list"
  return 0
}

emit "$head_sha"
emit "horodatage=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
emit "issue=${issue}"

if [[ -n "$sha_arg" ]]; then
  if ! expected=$(git rev-parse --verify "${sha_arg}^{commit}" 2>/dev/null); then
    emit "SHA inconnu: ${sha_arg}"
    exit 1
  fi
  if [[ "$expected" != "$head_sha" ]]; then
    emit "HEAD ${head_sha} différent du SHA demandé ${expected}"
    exit 1
  fi
fi

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  emit "arbre de travail modifié: les vérifications ne porteraient pas sur le commit ${head_sha}"
  git status --short --untracked-files=no >>"$RAW"
  exit 1
fi

if ! rustc_version=$(rustc --version 2>&1); then
  emit "rustc introuvable"
  exit 1
fi
emit "$rustc_version"
if [[ ! "$rustc_version" =~ ^rustc\ 1\.99\.1(\ |$) ]]; then
  emit "rustc 1.99.1 stable requis"
  exit 1
fi

extract_verify() {
  awk '
    /^```verify[[:space:]]*$/ { capture = 1; next }
    capture && /^```[[:space:]]*$/ { capture = 0; next }
    capture && /^[[:space:]]*#/ { next }
    capture && /^[[:space:]]*$/ { next }
    capture { print }
  ' "$1"
}

has_verify_commands() {
  [[ -n "$(extract_verify "$1")" ]]
}

body_file=$(mktemp)
cr=$(printf '\r')
normalize_body() {
  local tmp
  tmp=$(mktemp)
  sed "s/${cr}\$//" "$1" >"$tmp"
  cat "$tmp" >"$1"
  rm -f "$tmp"
}

if command -v gh >/dev/null 2>&1; then
  gh_err=$(mktemp)
  set +e
  gh issue view "$issue" --json body --jq .body >"$body_file" 2>"$gh_err"
  gh_status=$?
  set -e
  if [[ "$gh_status" -ne 0 ]]; then
    if [[ -s "$gh_err" ]]; then
      cat "$gh_err" >>"$RAW"
    fi
    rm -f "$gh_err"
    emit "gh issue view ${issue} a échoué"
    exit 1
  fi
  rm -f "$gh_err"
  normalize_body "$body_file"
  criteria_source="gh issue view ${issue}"
  if ! has_verify_commands "$body_file" && [[ -n "$criteria_file" ]]; then
    cp "$criteria_file" "$body_file"
    normalize_body "$body_file"
    criteria_source="fichier ${criteria_file} (aucun bloc verify sur le ticket)"
  fi
elif [[ -n "$criteria_file" ]]; then
  cp "$criteria_file" "$body_file"
  normalize_body "$body_file"
  criteria_source="fichier ${criteria_file}"
else
  emit "gh absent: passer le corps du ticket en fichier"
  exit 2
fi

if ! git rev-parse --verify --quiet 'origin/main^{commit}' >/dev/null; then
  emit "origin/main introuvable"
  exit 1
fi

load_comparison
captured_status=0

run_captured() {
  local display=$1 consume=$2 status out line started duration
  shift 2
  emit "+ ${display}"
  out=$(mktemp)
  started=$SECONDS
  set +e
  "$@" >"$out" 2>&1
  status=$?
  set -e
  duration=$((SECONDS - started))
  cat "$out" >>"$RAW"
  emit "exit:${status}"
  line=$(result_line "$out" "$status")
  record_step "$display" "$status" "$line" "$duration"
  "$consume" "$out" "$status" || true
  rm -f "$out"
  captured_status=$status
}

run_strict() {
  local display="" arg
  for arg in "$@"; do
    display+="${arg} "
  done
  display=${display% }
  run_captured "$display" : "$@"
  if [[ "$captured_status" -ne 0 ]]; then
    exit 1
  fi
}

run_shell() {
  local cmd=$1
  run_captured "$cmd" : env -u BASH_ENV bash --noprofile --norc -o pipefail -c "$cmd"
  if [[ "$captured_status" -ne 0 ]]; then
    fail=1
  fi
}

run_strict cargo build --workspace --locked
run_strict cargo fmt --check
run_strict cargo test --workspace --locked
run_strict cargo clippy --workspace --all-targets --locked -- -D warnings
run_strict gitleaks detect --redact --log-opts=origin/main..HEAD

measure_release_size() {
  # L'artefact annoncé par ce build est la référence, même avec un répertoire
  # cible personnalisé ou une compilation croisée. Ne pas mesurer un ancien
  # target/release/grokbot-wrapper par accident.
  [[ "$2" -eq 0 ]] || return 0
  release_bytes=$(python3 - "$1" <<'PY' 2>/dev/null
import json
from pathlib import Path
import sys

size = "—"
try:
    for line in Path(sys.argv[1]).read_text(encoding="utf-8").splitlines():
        try:
            artifact = json.loads(line)
        except ValueError:
            continue
        if not isinstance(artifact, dict):
            continue
        target = artifact.get("target") or {}
        if (artifact.get("reason") == "compiler-artifact"
                and target.get("name") == "grokbot-wrapper"
                and "bin" in target.get("kind", [])
                and artifact.get("executable")):
            size = "%s octets" % Path(artifact["executable"]).stat().st_size
except (OSError, UnicodeError, TypeError, AttributeError):
    pass
print(size)
PY
  ) || release_bytes="—"
}

if [[ "$release_size" -eq 1 ]]; then
  run_captured 'cargo build --release --locked -p grokbot-wrapper --message-format=json' measure_release_size \
    cargo build --release --locked -p grokbot-wrapper --message-format=json
fi

emit "critères: ${criteria_source}"

cmds=()
while IFS= read -r line; do
  [[ -n "$line" ]] && cmds+=("$line")
done < <(extract_verify "$body_file")

if [[ ${#cmds[@]} -eq 0 ]]; then
  emit "critères: aucune commande"
else
  for cmd in "${cmds[@]}"; do
    run_shell "$cmd"
  done
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

verdict=0
exit 0
