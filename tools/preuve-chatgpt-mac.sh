#!/bin/bash
# Preuve réelle ChatGPT sur le Mac du propriétaire.
# Un HOME de test, un clic sur « Se connecter » seulement si aucune session
# de cette copie n’est déjà utilisable, puis des contrôles sans jeton.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
real_home="${HOME}"
work=""
app_pid=""
keep=0
verdict_printed=0
base=""
port=""

note() { printf '%s\n' "$*"; }
die() { printf '%s\n' "$*" >&2; verdict_printed=1; exit 1; }

# N'imprime jamais la valeur capturée : seulement un code de sortie.
forbid_secrets() {
  local file="$1"
  python3 - "$file" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
if re.search(r"eyJ[A-Za-z0-9_-]{8,}|Bearer\s+\S+|refresh_token|access_token|id_token", text, re.I):
    sys.exit(3)
PY
}

journal_ok() {
  python3 - "$1" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
if re.search(r"eyJ[A-Za-z0-9_-]{8,}|Bearer\s+\S+|refresh_token|access_token|id_token", text, re.I):
    print("fuite")
    sys.exit(3)
needed = {
    "completed": r"status=200 code=completed",
    "model_effort": r"OpenAI request model=gpt-6\.1-sol effort=high\b",
    "refresh": r"status=200 code=token_refresh_saved",
}
missing = [name for name, pattern in needed.items() if not re.search(pattern, text)]
if missing:
    print("manque:" + ",".join(missing))
    sys.exit(1)
print("journal_ok")
PY
}

tokens_absent() {
  python3 - "$1" <<'PY'
import json, os, stat, sys
path = sys.argv[1]
data = json.load(open(path, encoding="utf-8"))
for key in ("access_token", "refresh_token", "id_token"):
    if data.get(key) not in (None, ""):
        print(key + "=present")
        sys.exit(1)
mode = stat.S_IMODE(os.stat(path).st_mode)
print(f"tokens_absent mode={mode:04o}")
PY
}

# Même en-têtes que app.js api() pour un POST, plus l’Origin que le WebView ajoute.
# Host vient de l’URL, une seule fois : un second Host est refusé par le garde.
call() {
  local method="$1" path="$2" body="${3:-}" dest="$4"
  local code
  local -a args=(-sS --max-redirs 0 --max-time 30 -o "$dest" -w '%{http_code}' -X "$method")
  if [[ "$method" != "GET" && "$method" != "HEAD" ]]; then
    args+=(
      -H "Content-Type: application/json"
      -H "X-Grokbot-Request: local"
      -H "Origin: ${base}"
    )
  fi
  if [[ -n "$body" ]]; then
    args+=(--data-binary "$body")
  fi
  code="$(curl "${args[@]}" "${base}${path}")"
  forbid_secrets "$dest" || die "Réponse locale refusée : motif de jeton."
  printf '%s' "$code"
}

session_has_grant() {
  python3 - "$1" <<'PY'
import json, sys
try:
    data = json.load(open(sys.argv[1], encoding="utf-8"))
except Exception:
    sys.exit(1)
signed = any(isinstance(data.get(key), str) and data.get(key) for key in ("access_token", "refresh_token"))
sys.exit(0 if signed else 1)
PY
}

wrapper_bin() {
  local candidate target_dir
  if [[ -n "${EZERBOT_WRAPPER:-}" && -x "${EZERBOT_WRAPPER}" ]]; then
    printf '%s\n' "${EZERBOT_WRAPPER}"
    return
  fi
  candidate="${EZERBOT_APP:-$root/app/build/Ezerbot.app}/Contents/MacOS/grokbot-wrapper"
  if [[ -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    return 1
  fi
  target_dir="$(cargo metadata --locked --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  for candidate in "$target_dir/debug/grokbot-wrapper" "$target_dir/release/grokbot-wrapper"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return
    fi
  done
  cargo build --locked -p grokbot-wrapper --manifest-path "$root/Cargo.toml" >/dev/null
  candidate="$target_dir/debug/grokbot-wrapper"
  [[ -x "$candidate" ]] || return 1
  printf '%s\n' "$candidate"
}

# Rejoue les appels de la preuve contre le garde, sans compte OpenAI.
guard_calls() {
  local dir wrapper pid="" line code conversation body
  dir="$(mktemp -d "${TMPDIR:-/tmp}/ezerbot-preuve-guard.XXXXXX")"
  stop_guard() {
    if [[ -n "${pid}" ]]; then
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
      pid=""
    fi
    rm -rf "$dir"
  }
  trap stop_guard EXIT
  wrapper="$(wrapper_bin)" || die "binaire grokbot-wrapper introuvable pour le contrôle du garde"
  HOME="$dir" "$wrapper" --port 0 --db "$dir/workspace.sqlite" >"$dir/announce" 2>"$dir/engine.err" &
  pid=$!
  base=""
  for _ in $(seq 1 50); do
    if [[ -s "$dir/announce" ]]; then
      line="$(head -n 1 "$dir/announce" || true)"
      base="$(python3 - "$line" <<'PY' || true
import json, sys
try:
    url = json.loads(sys.argv[1]).get("url") or ""
except Exception:
    url = ""
if url.startswith("http://127.0.0.1:"):
    print(url)
PY
)"
      [[ -n "$base" ]] && break
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      die "Le moteur local s’est arrêté avant d’annoncer son port."
    fi
    sleep 0.1
  done
  [[ -n "$base" ]] || die "Annonce de port illisible."
  port="${base##*:}"
  code="$(call GET /api/state "" "$dir/state.json")"
  [[ "$code" == "200" ]] || die "État de contrôle HTTP $code"
  code="$(call POST /api/preferences '{"model":"gpt-6.1-sol","effort":"high"}' "$dir/prefs.json")"
  [[ "$code" == "200" ]] || die "Le garde refuse encore les préférences (HTTP $code)."
  code="$(call POST /api/conversations '{}' "$dir/conversation.json")"
  [[ "$code" == "200" ]] || die "Conversation de contrôle HTTP $code"
  conversation="$(python3 - "$dir/conversation.json" <<'PY'
import json, sys
print(json.load(open(sys.argv[1], encoding="utf-8"))["conversation"]["id"])
PY
)"
  body="$(python3 - "$conversation" <<'PY'
import json, sys
print(json.dumps({"conversation_id": sys.argv[1], "message_id": "preuve-1", "body": "ping"}))
PY
)"
  code="$(call POST /api/send "$body" "$dir/send.json")"
  [[ "$code" != "403" ]] || die "Le garde refuse l’envoi."
  code="$(call POST /api/provider/test '{"refresh":true}' "$dir/refresh.json")"
  [[ "$code" == "200" ]] || die "Test de connexion HTTP $code"
  code="$(call GET /api/usage "" "$dir/usage.json")"
  [[ "$code" == "200" ]] || die "Quota de contrôle HTTP $code"
  code="$(call POST /api/usage/refresh '{}' "$dir/usage.json")"
  [[ "$code" == "200" ]] || die "Rafraîchissement de quota HTTP $code"
  code="$(call POST /api/provider/signout '{}' "$dir/signout.json")"
  [[ "$code" == "200" ]] || die "Déconnexion de contrôle HTTP $code"
  code="$(curl -sS --max-redirs 0 --max-time 30 -o "$dir/denied.json" -w '%{http_code}' -X POST -H 'Content-Type: application/json' --data-binary '{"model":"gpt-6.1-sol","effort":"high"}' "${base}/api/preferences")"
  [[ "$code" == "403" ]] || die "Le garde a laissé passer un POST sans le marqueur de l’interface (HTTP $code)."
  grep -q "ne provient pas de l'application locale" "$dir/denied.json" || die "Le refus du garde a changé."
  forbid_secrets "$dir/denied.json" || die "Le refus du garde contient un motif de jeton."
  code="$(curl -sS --max-redirs 0 --max-time 30 -o "$dir/origin.json" -w '%{http_code}' -X POST -H 'Content-Type: application/json' -H 'X-Grokbot-Request: local' -H 'Origin: https://example.invalid' --data-binary '{}' "${base}/api/preferences")"
  [[ "$code" == "403" ]] || die "Le garde a laissé passer une autre origine (HTTP $code)."
  base=""
  port=""
  trap - EXIT
  stop_guard
}

self_check() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/ezerbot-preuve-check.XXXXXX")"
  local token bad clean auth
  token="$(printf '%s%s%s' 'ey' 'J' 'preuveXXXXXXXX')"
  bad="$dir/bad.log"
  clean="$dir/clean.log"
  auth="$dir/auth.json"
  printf 'OpenAI request model=gpt-6.1-sol effort=high\nstatus=200 code=completed\nstatus=200 code=token_refresh_saved\nsecret=%s\n' "$token" >"$bad"
  printf 'OpenAI request model=gpt-6.1-sol effort=high\n[1] upstream turn=0123456789abcdef status=200 code=completed\n[2] upstream turn=fedcba9876543210 status=200 code=token_refresh_saved\n' >"$clean"
  printf '%s\n' '{"access_token":null,"refresh_token":null,"id_token":null}' >"$auth"
  chmod 600 "$auth"
  journal_ok "$bad" >"$dir/bad.out" || true
  if grep -q "$token" "$dir/bad.out" 2>/dev/null; then
    rm -rf "$dir"
    die "la censure a affiché un jeton"
  fi
  grep -qx 'fuite' "$dir/bad.out"
  journal_ok "$clean" | grep -qx 'journal_ok'
  tokens_absent "$auth" | grep -qx 'tokens_absent mode=0600'
  rm -rf "$dir"
  guard_calls
  note "self-check ok"
}

cleanup() {
  local status=$?
  if [[ -n "$app_pid" ]]; then
    kill "$app_pid" >/dev/null 2>&1 || true
    wait "$app_pid" >/dev/null 2>&1 || true
  fi
  if [[ "$status" -ne 0 ]]; then
    if [[ "$verdict_printed" -eq 0 ]]; then
      printf '%s\n' "preuve interrompue (code ${status})." >&2
    fi
    if [[ -n "$work" && -d "$work" ]]; then
      note "Copie conservée pour la prochaine preuve : $work"
    fi
  elif [[ "$keep" -eq 0 && -n "$work" && "$work" != "$real_home" && -d "$work" ]]; then
    rm -rf "$work"
    note "copie jetable supprimée"
  elif [[ -n "$work" && "$keep" -eq 1 ]]; then
    note "copie conservée : supprimez ce dossier vous-même, il peut contenir une session"
    note "$work"
  fi
  exit "$status"
}

if [[ "${1:-}" == "--self-check" ]]; then
  self_check
  exit 0
fi
if [[ "${1:-}" == "--keep" ]]; then
  keep=1
  shift
fi
if [[ $# -ne 0 ]]; then
  die "Usage : tools/preuve-chatgpt-mac.sh [--self-check|--keep]"
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  note "La preuve réelle se lance sur le Mac du propriétaire. Ici : tools/preuve-chatgpt-mac.sh --self-check"
  exit 2
fi

app="${EZERBOT_APP:-$root/app/build/Ezerbot.app}"
binary="$app/Contents/MacOS/Ezerbot"
engine="$app/Contents/MacOS/grokbot-wrapper"
[[ -x "$binary" && -x "$engine" ]] || die "Bundle absent. Construisez-le avec EZERBOT_BUILD_DEBUG=1 tools/build-app.sh"
if ! grep -a -q "Le HOME jetable du test est invalide." "$binary"; then
  die "Ce bundle ignore EZERBOT_TEST_HOME. Relancez EZERBOT_BUILD_DEBUG=1 tools/build-app.sh"
fi

tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root%/}"
work="${tmp_root}/ezerbot-preuve-session"
[[ "$work" != "$real_home" && "$work" == "${tmp_root}/"* ]] || die "HOME de test refusé"
mkdir -p "$work"
trap cleanup EXIT

real_auth="$real_home/Library/Application Support/Grokbot/workspace.codex-auth.json"
real_stamp="$(stat -f %m "$real_auth" 2>/dev/null || printf 'absent')"
guard_real() {
  local now
  now="$(stat -f %m "$real_auth" 2>/dev/null || printf 'absent')"
  [[ "$now" == "$real_stamp" ]] || die "La copie a touché les données réelles. Arrêt sans lecture du fichier."
}

export HOME="$work"
export EZERBOT_TEST_HOME="$work"
support="$work/Library/Application Support/Grokbot"
auth_file="$support/workspace.codex-auth.json"
reuse=0
if [[ -f "$auth_file" ]] && session_has_grant "$auth_file"; then
  reuse=1
  note "Session de test déjà enregistrée : réutilisation, sans nouveau clic si elle répond."
else
  note "Cliquez une seule fois sur « Se connecter » dans la fenêtre de cette copie."
fi
note "N’ouvrez pas l’Ezerbot habituel pendant cette preuve."
"$binary" >/dev/null 2>"$work/shell.err" &
app_pid=$!

port=""
for _ in $(seq 1 60); do
  guard_real
  if ! kill -0 "$app_pid" >/dev/null 2>&1; then
    die "Ezerbot s’est arrêté avant d’annoncer son port."
  fi
  wrapper="$(pgrep -P "$app_pid" grokbot-wrapper | head -n 1 || true)"
  if [[ -n "$wrapper" ]]; then
    port="$(lsof -nP -a -p "$wrapper" -iTCP -sTCP:LISTEN 2>/dev/null | awk 'NR>1 {print $9}' | sed -n 's/.*://p' | head -n 1 || true)"
  fi
  [[ -n "$port" ]] && break
  sleep 1
done
[[ -n "$port" && "$port" =~ ^[0-9]+$ ]] || die "Port local introuvable."
base="http://127.0.0.1:${port}"

field() {
  python3 - "$1" "$2" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for part in sys.argv[2].split("."):
    if isinstance(data, dict):
        data = data.get(part)
    else:
        data = None
        break
if isinstance(data, bool):
    print("true" if data else "false")
elif data is None:
    print("")
elif isinstance(data, (dict, list)):
    sys.exit(2)
else:
    text = str(data)
    if any(piece in text.lower() for piece in ("eyj", "bearer", "refresh", "access_token")):
        sys.exit(3)
    print(text)
PY
}

if [[ "$reuse" -eq 1 ]]; then
  note "En attente de la session déjà enregistrée, puis de la sonde (10 min)."
else
  note "En attente du clic « Se connecter » et d’une sonde réussie (10 min)."
fi
ready=0
asked="$reuse"
for attempt in $(seq 1 300); do
  guard_real
  code="$(call GET /api/state "" "$work/state.json")"
  [[ "$code" == "200" ]] || die "État HTTP $code"
  signed="$(field "$work/state.json" provider.signed_in)"
  connected="$(field "$work/state.json" provider.connected)"
  if [[ "$signed" == "true" && "$connected" == "true" ]]; then
    ready=1
    break
  fi
  if [[ "$asked" -eq 1 && "$signed" != "true" && "$attempt" -eq 10 ]]; then
    note "Cette session ne répond plus. Cliquez une seule fois sur « Se connecter »."
    asked=0
  fi
  sleep 2
done
[[ "$ready" -eq 1 ]] || die "Connexion non confirmée. Un seul clic sur « Se connecter », puis attendez la sonde."

code="$(call POST /api/preferences '{"model":"gpt-6.1-sol","effort":"high"}' "$work/prefs.json")"
[[ "$code" == "200" ]] || die "Préférences HTTP $code"
model="$(field "$work/prefs.json" preferences.model)"
effort="$(field "$work/prefs.json" preferences.effort)"
[[ "$model" == "gpt-6.1-sol" && "$effort" == "high" ]] || die "GPT-6.1 Sol ou l’effort high n’est pas retenu par le compte."

code="$(call POST /api/conversations '{}' "$work/conversation.json")"
[[ "$code" == "200" ]] || die "Conversation HTTP $code"
conversation="$(field "$work/conversation.json" conversation.id)"
[[ -n "$conversation" ]] || die "Conversation sans identifiant."

prompt='Réponds uniquement par trois lignes, exactement : 1. Sept. 2. Sept. 3. Sept.'
body="$(python3 - "$conversation" "$prompt" <<'PY'
import json, sys
print(json.dumps({"conversation_id": sys.argv[1], "message_id": "preuve-1", "body": sys.argv[2]}, ensure_ascii=False))
PY
)"
code="$(call POST /api/send "$body" "$work/send.json")"
[[ "$code" == "200" ]] || die "Envoi HTTP $code"

complete=0
for _ in $(seq 1 90); do
  code="$(call GET /api/state "" "$work/state.json")"
  [[ "$code" == "200" ]] || die "État HTTP $code"
  reply_status=0
  python3 - "$work/state.json" <<'PY' || reply_status=$?
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
for conversation in state.get("conversations") or []:
    for message in conversation.get("messages") or []:
        if message.get("id") != "reply-preuve-1":
            continue
        if message.get("state") == "streaming":
            sys.exit(1)
        body = message.get("body") or ""
        ok = message.get("state") == "echoed" and not message.get("error") and all(part in body for part in ("1. Sept.", "2. Sept.", "3. Sept."))
        sys.exit(0 if ok else 4)
sys.exit(1)
PY
  if [[ "$reply_status" -eq 0 ]]; then
    complete=1
    break
  fi
  [[ "$reply_status" -eq 1 ]] || die "Réponse incomplète ou refusée."
  sleep 2
done
[[ "$complete" -eq 1 ]] || die "Réponse incomplète ou refusée."

code="$(call POST /api/provider/test '{"refresh":true}' "$work/refresh.json")"
[[ "$code" == "200" ]] || die "Rafraîchissement HTTP $code"
refreshed=0
for _ in $(seq 1 30); do
  code="$(call GET /api/state "" "$work/state.json")"
  [[ "$code" == "200" ]] || die "État HTTP $code"
  if [[ "$(field "$work/state.json" provider.connected)" == "true" ]] && journal_ok "$support/logs/engine.log" >/dev/null 2>&1; then
    refreshed=1
    break
  fi
  sleep 1
done
[[ "$refreshed" -eq 1 ]] || die "Le rafraîchissement du jeton ou le journal n’est pas confirmé."

quota=0
for _ in $(seq 1 20); do
  code="$(call GET /api/usage "" "$work/usage.json")"
  [[ "$code" == "200" ]] || die "Quota HTTP $code"
  if python3 - "$work/usage.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8")).get("codex") or {}
if data.get("status") != "ready":
    sys.exit(1)
quota = data.get("quota") or {}
windows = [quota.get("primary"), quota.get("secondary")]
percents = [window.get("used_percent") for window in windows if isinstance(window, dict)]
if not percents or any(not isinstance(value, (int, float)) or value < 0 or value > 100 for value in percents):
    sys.exit(4)
print("quota_ok")
PY
  then
    quota=1
    break
  fi
  code="$(call POST /api/usage/refresh '{}' "$work/usage.json")"
  [[ "$code" == "200" ]] || die "Quota HTTP $code"
  sleep 1
done
[[ "$quota" -eq 1 ]] || die "Quota WHAM indisponible."

code="$(call POST /api/provider/signout '{}' "$work/signout.json")"
[[ "$code" == "200" ]] || die "Déconnexion HTTP $code"
code="$(call GET /api/state "" "$work/state.json")"
[[ "$code" == "200" ]] || die "État HTTP $code"
[[ "$(field "$work/state.json" provider.signed_in)" == "false" ]] || die "La session reste affichée connectée."
[[ "$(field "$work/state.json" provider.connected)" == "false" ]] || die "La sonde reste affichée connectée."
tokens_absent "$support/workspace.codex-auth.json" | grep -q 'tokens_absent' || die "Des jetons restent dans la copie."
journal_ok "$support/logs/engine.log" | grep -qx 'journal_ok'
if [[ -s "$work/shell.err" ]]; then
  forbid_secrets "$work/shell.err" || die "Le journal du shell contient un motif de jeton."
fi
guard_real
note "preuve ok : 200, réponse complète, gpt-6.1-sol, effort high, jeton rafraîchi, quota, déconnexion"
