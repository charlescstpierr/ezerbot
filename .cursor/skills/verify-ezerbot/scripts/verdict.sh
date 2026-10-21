#!/usr/bin/env bash
# Écrit report.md et la ligne PASS <sha> ou FIX <sha>. Refuse si le commit n'existe pas.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/verdict.py" "$@"
