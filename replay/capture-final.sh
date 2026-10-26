#!/bin/bash
set -euo pipefail
E=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i561/r2
S=/Users/cstpierrecharles/.treehouse/pm/ez-i561/.cursor/skills/verify-ezerbot/scripts
source "$E/env.sh"
export EZERBOT_VERIFY_REF_SHOTS=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i561/references
export EZERBOT_VERIFY_REF_ANNEX="$EZERBOT_VERIFY_REF_SHOTS/fondations-README.md"
export EZERBOT_VERIFY_REA_REPORTS="$EZERBOT_VERIFY_REF_SHOTS"
export EZERBOT_VERIFY_REA_EVIDENCE="$EZERBOT_VERIFY_REF_SHOTS"
export EZERBOT_VERIFY_REA_FORMATTED="$EZERBOT_VERIFY_REF_SHOTS"
CURRENT="$E/final-v2"
cleanup() {
  if [ -f "$CURRENT/run.json" ]; then bash "$S/cleanup.sh" --evidence "$CURRENT" > "$CURRENT/cleanup.log" 2>&1; fi
}
trap cleanup EXIT
bash "$S/launch.sh" --sphere accueil --evidence "$CURRENT" > "$E/launch-final.log" 2>&1
bash "$S/doctor.sh" --evidence "$CURRENT" > "$CURRENT/doctor-before.log" 2>&1
bash "$S/drive.sh" --sphere accueil --evidence "$CURRENT" > "$CURRENT/drive-accueil.log" 2>&1
python3 "$E/seed-capture.py" "$CURRENT" --seed-only > "$CURRENT/seed.log" 2>&1
bash "$S/browser.sh" --evidence "$CURRENT" emulate --viewport 820x560x1 > "$CURRENT/emulate.log" 2>&1
python3 "$E/proof-rail.py" "$CURRENT" > "$CURRENT/rail.log" 2>&1
python3 "$E/proof-updates.py" "$CURRENT" > "$CURRENT/updates.log" 2>&1
python3 "$E/proof-create.py" "$CURRENT" > "$CURRENT/create.log" 2>&1
bash "$S/doctor.sh" --evidence "$CURRENT" > "$CURRENT/doctor-after.log" 2>&1
bash "$S/compare.sh" --evidence "$CURRENT" > "$CURRENT/compare.log" 2>&1
cleanup
CURRENT="$E/final-pages"
bash "$S/launch.sh" --sphere pages --evidence "$CURRENT" > "$E/launch-pages.log" 2>&1
bash "$S/doctor.sh" --evidence "$CURRENT" > "$CURRENT/doctor-before.log" 2>&1
bash "$S/drive.sh" --sphere pages --evidence "$CURRENT" > "$CURRENT/drive-pages.log" 2>&1
python3 "$E/proof-pages-captures.py" "$CURRENT" > "$CURRENT/captures.log" 2>&1
bash "$S/doctor.sh" --evidence "$CURRENT" > "$CURRENT/doctor-after.log" 2>&1
bash "$S/compare.sh" --evidence "$CURRENT" > "$CURRENT/compare.log" 2>&1
cleanup
trap - EXIT
echo 'Parcours R2 terminés et deux Cleanup effectués.'
