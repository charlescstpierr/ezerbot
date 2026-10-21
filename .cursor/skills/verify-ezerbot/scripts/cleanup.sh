#!/usr/bin/env bash
# Arrête seulement les processus de ce run, supprime le HOME jetable, garde les preuves.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/cleanup.py" "$@"
