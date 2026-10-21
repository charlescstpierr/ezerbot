#!/usr/bin/env bash
# Lance un Ezerbot isolé : build verrouillé, faux OpenAI, HOME jetable.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/launch.py" "$@"
