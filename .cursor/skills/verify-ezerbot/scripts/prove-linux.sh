#!/usr/bin/env bash
# Preuve Linux : build du commit extrait, wrapper isolé sous Xvfb privé (xvfb-run -a), trois entrées
# de la carte (accueil, réponse dans un fil, paramètres), captures, verdict PASS/FIX, nettoyage.
# Usage : bash .cursor/skills/verify-ezerbot/scripts/prove-linux.sh [--evidence <dossier-neuf>] [--sphere linux] [--patch <patch-id>]
set -euo pipefail
umask 077

fail() { printf 'verify-ezerbot: %s\n' "$*" >&2; exit 2; }
[[ $(uname -s) == Linux ]] || fail 'prove-linux.sh exige Linux ; sur le Mac, utiliser prove.sh.'
DIR=$(cd "$(dirname -- "$0")" && pwd)
root=$(git -C "$DIR" rev-parse --show-toplevel)
evidence=${EZERBOT_VERIFY_EVIDENCE_DIR:-}
patch=${EZERBOT_VERIFY_PATCH_ID:-}
while (($#)); do
  case $1 in
    --evidence) [[ $# -ge 2 ]] || fail '--evidence exige un dossier.'; evidence=$2; shift 2 ;;
    # `linux` nomme les trois entrées jouées ensemble ; les autres sphères passent par prove.py.
    --sphere) [[ $# -ge 2 && $2 == linux ]] || fail "--sphere : seul « linux » passe par ce chemin ; les autres sphères (dont all) passent par prove.sh."; shift 2 ;;
    --patch) [[ $# -ge 2 ]] || fail '--patch exige un patch-id.'; patch=$2; shift 2 ;;
    *) fail "argument inconnu : $1" ;;
  esac
done

[[ -z $patch || $patch =~ ^[0-9a-f]{40}$ ]] || fail 'patch-id invalide : 40 hexadécimaux minuscules attendus.'
patch_args=()
[[ -z $patch ]] || patch_args=(--patch "$patch")
sha=$(git -C "$root" rev-parse --verify 'HEAD^{commit}')
unix=$(git -C "$root" show -s --format=%ct "$sha")
[[ -n $evidence ]] || evidence="${TMPDIR:-/tmp}/verify-ezerbot-evidence-${sha:0:12}-$(date +%Y%m%dT%H%M%S)-$$"
evidence=$(realpath -m -- "$evidence")
case $evidence/ in "$root"/*) fail "les preuves doivent rester hors du dépôt : $evidence" ;; esac
# Un dossier neuf (absent ou vide) garantit que le verdict ne lit que les preuves de ce run.
if [[ -e $evidence ]] && [[ ! -d $evidence || -n $(ls -A -- "$evidence") ]]; then fail "dossier de preuves déjà utilisé : $evidence"; fi
mkdir -p "$evidence/captures"

# shellcheck source=linux_run_lib.sh
source "$DIR/linux_run_lib.sh"

printf 'verify-ezerbot: commit %s\nverify-ezerbot: preuves %s\n' "$sha" "$evidence"
if ! python3 "$DIR/doctor.py" --prerequisites >"$evidence/doctor.txt"; then
  cat "$evidence/doctor.txt"; fail 'prérequis Linux manquants (voir doctor.txt).'
fi
tail -n 1 "$evidence/doctor.txt"
dirty=$(git -C "$root" status --porcelain -- crates app/ui app/web Cargo.toml Cargo.lock vendor app/linux/src/local-api.js tools/linux-captures.cjs .cursor/skills/verify-ezerbot/scripts)
[[ -z $dirty ]] || fail "sources compilées modifiées depuis $sha ; committer avant la preuve :
$dirty"

cargo=${EZERBOT_VERIFY_CARGO:-$HOME/.cargo/bin/cargo}
target="$root/target/verify-linux"
export PATH="$(dirname -- "$cargo"):$PATH"
iso=(env -i PATH="$PATH" HOME="$run/home" XDG_CONFIG_HOME="$run/config" XDG_CACHE_HOME="$run/cache"
  XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run/runtime" TMPDIR="$run/tmp" LANG=C.UTF-8 TZ=UTC)
mkdir -p "$target"
# Un seul run à la fois par checkout : npm, Cargo et le pilote lisent les mêmes assets générés.
# Le verrou reste tenu jusqu'à la fin du run (fermé par la sortie du script).
# Refus immédiat si un autre run le tient : une attente bloquerait aussi SIGTERM et le nettoyage.
exec 8>"$target/.verify-build.lock"
flock -n 8 || fail "un autre run tient $target/.verify-build.lock ; relancer quand il est fini."
# Un échec de build écrit quand même report.md et une ligne FIX (erreur d'outil), puis sort avec 1.
build_failed() {
  local name=$1 code=$2
  python3 "$DIR/linux_verdict.py" --evidence "$evidence" --sha "$sha" --build-failed "$name" --build-status "$code" "${patch_args[@]}" >"$evidence/verdict.txt" || true
  cat "$evidence/verdict.txt"
  exit 1
}
step "$root/app/web" "${iso[@]}" npm_config_cache="$run/cache/npm" nice -n 10 npm ci --no-audit --no-fund --loglevel=error || build_failed "npm ci" $?
step "$root/app/web" "${iso[@]}" nice -n 10 npm run build --silent || build_failed "npm run build" $?
rm -f -- "$target/debug/grokbot-wrapper"
code=0
step "$root" env CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" PATH="$PATH" \
  TMPDIR="$run/tmp" CARGO_TARGET_DIR="$target" CARGO_BUILD_JOBS=3 EZERBOT_GIT_COMMIT="$sha" EZERBOT_GIT_COMMIT_UNIX="$unix" \
  nice -n 10 "$cargo" build -p grokbot-wrapper --locked || code=$?
(( code == 0 )) || build_failed "cargo build" "$code"
binary="$target/debug/grokbot-wrapper"
[[ -x $binary ]] || build_failed "cargo build (binaire absent)" 1

# Même résolution que le doctor : EZERBOT_VERIFY_CHROME, puis CHROME_BIN, puis le PATH.
chrome=$(python3 "$DIR/doctor.py" --chrome) || fail 'Chrome introuvable (voir doctor.txt).'
drive=0
step "$root" "${iso[@]}" CHROME_BIN="$chrome" \
  xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' \
  node --experimental-websocket "$DIR/linux_drive.cjs" "$sha" "$binary" "$evidence/captures" || drive=$?
python3 "$DIR/linux_verdict.py" --evidence "$evidence" --sha "$sha" --drive-status "$drive" "${patch_args[@]}"
