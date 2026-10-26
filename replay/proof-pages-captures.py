import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js, theme_js
from capture import capture

e = Path(sys.argv[1])
for theme in ('dark', 'light'):
    eval_js(e, theme_js(theme, True))
    for width, height in ((1440, 900), (820, 560)):
        browser(e, 'emulate', '--viewport', f'{width}x{height}x1')
        wait_js(e, '() => !document.querySelector(".sidebar-pages").hidden', 'Page réelle absente')
        capture(e, e / 'pages' / f'{theme}-{width}x{height}.png')
print('Quatre captures Pages conformes.')
