#!/usr/bin/env bash
# Pilote Chrome. Défaut : chrome-devtools-axi. Substitut : EZERBOT_VERIFY_BROWSER=chrome-headless.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/browser.py" "$@"
