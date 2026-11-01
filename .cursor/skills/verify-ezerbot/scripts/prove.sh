#!/usr/bin/env bash
# Preuve : lancement, doctor, une sphère, verdict, nettoyage, preuves encore là.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
# Linux (plateforme principale) : sans choix explicite de navigateur, preuve sous Xvfb privé.
# Les sphères qu'elle ne couvre pas gardent le substitut headless de prove.py ; --patch et
# EZERBOT_VERIFY_PATCH_ID restent sur le chemin Linux.
linux_xvfb=1
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case ${args[i]} in
    --evidence | --patch) i=$((i + 1)) ;;
    --sphere) [[ ${args[i + 1]:-} =~ ^(all|accueil|reponse-fil|parametres-connecteurs)$ ]] || linux_xvfb=0; i=$((i + 1)) ;;
    *) linux_xvfb=0 ;;
  esac
done
if [[ "$(uname -s)" == "Linux" && -z "${EZERBOT_VERIFY_BROWSER:-}" && $linux_xvfb == 1 ]]; then
  exec bash "$DIR/prove-linux.sh" "$@"
fi
if [[ "$(uname -s)" != "Darwin" && -z "${EZERBOT_VERIFY_BROWSER:-}" ]]; then
  export EZERBOT_VERIFY_BROWSER=chrome-headless
fi
export CHROME_DEVTOOLS_AXI_SESSION="${CHROME_DEVTOOLS_AXI_SESSION:-verify-ezerbot}"
exec python3 "$DIR/prove.py" "$@"
