#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" == "--self-check" || "${1:-}" == "--guard-calls" ]]; then
  mode="$1"
  shift
  [[ $# -eq 0 ]] || { printf '%s\n' "Usage: tools/preuve-chatgpt-mac.sh [--self-check|--guard-calls]" >&2; exit 2; }
  exec python3 "$root/tools/verify-proof-guard.py" "$mode"
fi
if [[ "${1:-}" == "--keep" ]]; then
  printf '%s\n' "L'ancien mode --keep est fermé. Utilisez verify-ezerbot avec --evidence pour conserver les preuves isolées." >&2
  exit 2
fi
printf '%s\n' "L'identité de l'app ouverte reste dans la coque. La preuve utilise désormais verify-ezerbot, un HOME isolé et un faux fournisseur."
exec bash "$root/.cursor/skills/verify-ezerbot/scripts/prove.sh" "$@"
