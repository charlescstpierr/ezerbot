import hashlib
import json
from pathlib import Path
import sys

from export import verify
from schema import receipt

raw = Path(sys.argv[1])
if not (raw / 'export-ready.json').exists():
    print('I557 no_safe_export')
    raise SystemExit(1)
ready = json.loads((raw / 'export-ready.json').read_text())
assert ready['final_scan_exit_code'] == 0
assert hashlib.sha256((raw / 'export/manifest.json').read_bytes()).hexdigest() == ready['manifest_sha256']
verify(raw / 'export')
state = json.loads((raw / 'export/receipt.json').read_text())
receipt(state)
print('I557 final measurement_valid=' + str(int(state['measurement_valid'])) + ' product_failed=' + str(int(state['product_failed'])) + ' preparation_failed=' + str(int(state['preparation_failed'])))
raise SystemExit(0 if state['measurement_valid'] and not state['product_failed'] and not state['preparation_failed'] else 1)
