#!/usr/bin/env bash
# Preuve : lancement, doctor, une sphère, verdict, nettoyage, preuves encore là.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
# Linux (plateforme principale) : sans choix explicite de navigateur, preuve sous Xvfb privé.
# Ce chemin s'appelle `--sphere linux` (défaut sous Linux) ; toute autre sphère, `all` compris,
# garde le parcours de prove.py, inchangé. --patch et EZERBOT_VERIFY_PATCH_ID suivent le chemin choisi.
# Analyse stricte : `--opt valeur` ou `--opt=valeur` pour --evidence, --sphere, --patch ; tout
# autre argument est refusé (code 2) au lieu de retomber silencieusement sur un autre parcours.
usage() { printf 'prove.sh: %s\nUsage : prove.sh [--sphere <sphère>|linux] [--evidence <dossier>] [--patch <patch-id>]\n' "$1" >&2; exit 2; }
sphere=''
args=()
while (($#)); do
  case $1 in
    --evidence=* | --sphere=* | --patch=*) name=${1%%=*}; value=${1#*=}; shift ;;
    --evidence | --sphere | --patch) [[ $# -ge 2 ]] || usage "$1 exige une valeur."; name=$1; value=$2; shift 2 ;;
    *) usage "argument inconnu : $1" ;;
  esac
  [[ -n $value ]] || usage "$name exige une valeur non vide."
  [[ $name != --sphere ]] || sphere=$value
  args+=("$name" "$value")
done
set -- ${args[@]+"${args[@]}"}
linux_xvfb=0
[[ -z $sphere || $sphere == linux ]] && linux_xvfb=1
if [[ "$(uname -s)" == "Linux" && -z "${EZERBOT_VERIFY_BROWSER:-}" && $linux_xvfb == 1 ]]; then
  exec bash "$DIR/prove-linux.sh" "$@"
fi
if [[ "$(uname -s)" != "Darwin" && -z "${EZERBOT_VERIFY_BROWSER:-}" ]]; then
  export EZERBOT_VERIFY_BROWSER=chrome-headless
fi
export CHROME_DEVTOOLS_AXI_SESSION="${CHROME_DEVTOOLS_AXI_SESSION:-verify-ezerbot}"
exec python3 "$DIR/prove.py" "$@"
