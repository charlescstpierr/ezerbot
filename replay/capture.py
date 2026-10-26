import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path

from drive import browser, eval_js

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')


def capture(e, path):
    viewport = eval_js(e, '() => ({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,theme:document.documentElement.dataset.theme})')
    sha = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
    path.parent.mkdir(exist_ok=True, parents=True)
    browser(e, 'screenshot', str(path))
    data = path.read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n' and data[12:16] == b'IHDR'
    ihdr = list(struct.unpack('>II', data[16:24]))
    expected = [int(n) for n in re.search(r'(\d+)x(\d+)$', path.stem).groups()]
    assert ihdr == expected == [viewport['width'], viewport['height']] and viewport['dpr'] == 1, (path, ihdr, viewport)
    receipt = e / 'png-receipts.json'
    rows = json.loads(receipt.read_text()) if receipt.exists() else []
    rows.append({'file': str(path.relative_to(e)), 'sha': sha, 'theme': viewport['theme'], 'ihdr': ihdr, 'viewport': viewport, 'sha256': hashlib.sha256(data).hexdigest()})
    receipt.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + '\n')
    path.with_suffix('.txt').write_text(eval_js(e, '() => document.getElementById("sidebar").innerText'))

