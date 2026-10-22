#!/usr/bin/env bash
# Barrière de preuve de l'usine.
# Usage : scripts/verify-factory.sh <issue> [sha] [fichier]
#
# Imprime le SHA vérifié (git rev-parse HEAD). Si un SHA est donné, HEAD doit
# être exactement ce commit. Rust stable requis : 1.99.1.
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
# Journal : target/verify-factory/<issue>-<sha>.log
# Sortie 0 seulement si toute la barrière passe. Ne fusionne rien.
set -euo pipefail

export CARGO_TERM_COLOR=never

usage() {
  printf '%s\n' 'Usage: scripts/verify-factory.sh <issue> [sha] [fichier]' >&2
  exit 2
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
fi

issue=$1
sha_arg=""
criteria_file=""

if [[ ! "$issue" =~ ^[0-9]+$ ]]; then
  usage
fi

if [[ $# -ge 2 ]]; then
  if [[ "$2" =~ ^[0-9a-fA-F]{7,40}$ ]]; then
    sha_arg=$2
    if [[ $# -eq 3 ]]; then
      criteria_file=$3
    fi
  elif [[ $# -eq 2 ]]; then
    criteria_file=$2
  else
    usage
  fi
fi

LOG=""
body_file=""
verdict=1
fail=0
criteria_source=""

finish() {
  local status=$?
  trap - EXIT
  set +e
  if [[ -n "$LOG" && -f "$LOG" ]]; then
    if [[ "$verdict" -eq 0 ]]; then
      printf '%s\n' 'verify-factory: ok' | tee -a "$LOG"
    else
      printf '%s\n' 'verify-factory: échec' | tee -a "$LOG"
    fi
  fi
  if [[ -n "$body_file" ]]; then
    rm -f "$body_file"
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
: >"$LOG"

emit() {
  printf '%s\n' "$1" | tee -a "$LOG"
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
normalize_body() {
  sed -i 's/\r$//' "$1"
}

if command -v gh >/dev/null 2>&1; then
  gh_err=$(mktemp)
  set +e
  gh issue view "$issue" --json body --jq .body >"$body_file" 2>"$gh_err"
  gh_status=$?
  set -e
  if [[ "$gh_status" -ne 0 ]]; then
    if [[ -s "$gh_err" ]]; then
      tee -a "$LOG" <"$gh_err"
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

run_strict() {
  local display="" arg status
  for arg in "$@"; do
    display+="${arg} "
  done
  display=${display% }
  emit "+ ${display}"
  set +e
  "$@" 2>&1 | tee -a "$LOG"
  status=${PIPESTATUS[0]}
  set -e
  emit "exit:${status}"
  if [[ "$status" -ne 0 ]]; then
    exit 1
  fi
}

run_shell() {
  local cmd=$1 status
  emit "+ ${cmd}"
  set +e
  env -u BASH_ENV bash --noprofile --norc -c "$cmd" 2>&1 | tee -a "$LOG"
  status=${PIPESTATUS[0]}
  set -e
  emit "exit:${status}"
  if [[ "$status" -ne 0 ]]; then
    fail=1
  fi
}

run_strict cargo build --workspace --locked
run_strict cargo fmt --check
run_strict cargo test --workspace --locked
run_strict cargo clippy --workspace --all-targets --locked -- -D warnings
run_strict gitleaks detect --redact --log-opts=origin/main..HEAD

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
