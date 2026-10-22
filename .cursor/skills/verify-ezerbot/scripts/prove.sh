#!/usr/bin/env bash
# Preuve : lancement, doctor, une sphère, verdict, nettoyage, preuves encore là.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
if [[ "$(uname -s)" != "Darwin" && -z "${EZERBOT_VERIFY_BROWSER:-}" ]]; then
  export EZERBOT_VERIFY_BROWSER=chrome-headless
fi
export CHROME_DEVTOOLS_AXI_SESSION="${CHROME_DEVTOOLS_AXI_SESSION:-verify-ezerbot}"
exec python3 "$DIR/prove.py" "$@"
