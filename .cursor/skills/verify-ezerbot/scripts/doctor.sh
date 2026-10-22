#!/usr/bin/env bash
# Contrôle en lecture seule : PID, port, commit du binaire.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/doctor.py" "$@"
