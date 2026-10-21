#!/usr/bin/env bash
# Parcourt une sphère et écrit les captures PNG dans le dossier de preuves.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/drive.py" "$@"
