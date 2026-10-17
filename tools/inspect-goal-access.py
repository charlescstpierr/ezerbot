#!/usr/bin/env python3
"""Explicit read-only capture of evidence roots and iPhone access availability."""
from pathlib import Path
import argparse
import datetime
import json
import plistlib
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', action='store_true', required=True)
    parser.parse_args()
    result = {'schema': 1, 'recorded_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'Selected saved artifact roots and iPhone access-tool availability only',
              'account_state_inspected': False, 'profiles_read': False,
              'network_used': False, 'original_app_started': False}
    args = ['rg', '--files', '--hidden', '--no-ignore', 'inputs', 'artifacts/releases/0.71.2',
            '-g', '*.ipa', '-g', '*.apk', '-g', '*.har', '-g', '*.mitm', '-g', '*.flow']
    scan = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=15)
    assert scan.returncode in (0, 1)
    result['package_capture_scan'] = {'argv': args, 'exit_code': scan.returncode,
        'matches': scan.stdout.splitlines(),
        'limitation': 'Only these roots and suffixes; other formats/locations/devices remain unsearched.'}
    available = {}
    for name in ['devicectl', 'xctrace', 'simctl']:
        check = subprocess.run(['/usr/bin/xcrun', '--find', name], capture_output=True, timeout=15)
        available[name] = {'available': check.returncode == 0, 'exit_code': check.returncode}
    for name in ['idevice_id', 'ideviceinstaller', 'iproxy']:
        available[name] = {'available': shutil.which(name) is not None}
    result['ios_access_tools'] = available
    query = subprocess.run(['/usr/sbin/ioreg', '-r', '-c', 'IOUSBHostDevice', '-a'], capture_output=True, timeout=15)
    usb = {'query_exit_code': query.returncode, 'matching_product_names': [], 'serials_retained': False,
           'limitation': 'USB registry names only; no pairing/unlock/device trust or Wi-Fi inventory.'}

    def visit(value):
        if isinstance(value, dict):
            label = value.get('USB Product Name', '')
            if isinstance(label, str) and 'iphone' in label.lower():
                usb['matching_product_names'].append(label)
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    if query.returncode == 0:
        visit(plistlib.loads(query.stdout))
    result['iphone_usb_name_query'] = usb
    (ROOT / 'evidence/goal-access-audit.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
