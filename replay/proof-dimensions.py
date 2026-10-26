import hashlib
import json
import struct
import subprocess
import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js, theme_js

e = Path(sys.argv[1])
sha = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
assert sha == 'be2cedd617db0161bab4d3192170dc9369497056'
shots = e / 'sidebar'
shots.mkdir(exist_ok=True)
rows = []
for theme in ('dark', 'light'):
    eval_js(e, theme_js(theme, True))
    for width, height in ((1440, 900), (820, 560)):
        browser(e, 'emulate', '--viewport', f'{width}x{height}x1')
        metrics = wait_js(e, f'''() => {{
          const w={width},h={height};
          if (innerWidth!==w || innerHeight!==h || devicePixelRatio!==1) return false;
          return {{innerWidth,innerHeight,devicePixelRatio,clientWidth:document.documentElement.clientWidth,clientHeight:document.documentElement.clientHeight}};
        }}''', 'viewport non conforme')
        eval_js(e, '''() => {
          document.querySelector('.sidebar-content').scrollTop=0;
          return {bots:document.querySelectorAll('.sidebar-bot').length,threads:document.querySelectorAll('#conversation-list .conversation-item').length};
        }''')
        wait_js(e, "() => document.querySelectorAll('.sidebar-bot').length===5 && document.querySelectorAll('#conversation-list .conversation-item').length===32", 'données latérales non conformes')
        path = shots / f'{theme}-{width}x{height}.png'
        browser(e, 'screenshot', str(path))
        data = path.read_bytes()
        assert data[:8] == b'\x89PNG\r\n\x1a\n' and data[12:16] == b'IHDR'
        actual = struct.unpack('>II', data[16:24])
        assert actual == (width, height), f'IHDR {actual} attendu {(width,height)}'
        (path.with_suffix('.txt')).write_text(eval_js(e, '() => document.getElementById("sidebar").innerText'))
        rows.append({'file':str(path.relative_to(e)),'sha':sha,'run':'final-dimensions','theme':theme,'expected':[width,height],'ihdr':list(actual),'viewport':metrics,'sha256':hashlib.sha256(data).hexdigest(),'method':'nouvelle capture Chrome Mac, viewport émulée desktop DPR 1'})
(e / 'dimensions.json').write_text(json.dumps(rows, indent=2, ensure_ascii=False)+'\n')
print('IHDR et viewport conformes pour les quatre nouvelles captures.')
