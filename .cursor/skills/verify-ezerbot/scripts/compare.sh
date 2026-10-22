#!/usr/bin/env bash
# Compare les captures aux références Grok Bot, sans les modifier.
set -euo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
export EZERBOT_VERIFY_LOGIN_HOME="${EZERBOT_VERIFY_LOGIN_HOME:-$HOME}"
exec python3 "$DIR/compare.py" "$@"
