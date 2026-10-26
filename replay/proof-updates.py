import json
import subprocess
import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js, theme_js
from capture import capture

e = Path(sys.argv[1])
fixture = repo / 'app/ui/updates-browser-fixture.cjs'


def function(name):
    return subprocess.check_output(['node', '-p', f'require({json.dumps(str(fixture))}).{name}.toString()'], text=True).strip()


eval_js(e, f'async () => {{ await ({function("installUpdatesService")})(); return true; }}')
geometry = function('updateGeometry')
checks = []
error = 'La compilation a échoué. L’application actuelle est restée en place. Réessayez depuis la barre latérale.'


def set_state(phase, extra=None):
    eval_js(e, f'async () => {{ await updatesFixture.set({json.dumps(phase)}, {json.dumps(extra or {})}); return true; }}')


for theme in ('dark', 'light'):
    eval_js(e, theme_js(theme, True))
    for width, height in ((1440, 900), (820, 560)):
        browser(e, 'emulate', '--viewport', f'{width}x{height}x1')
        wait_js(e, f'() => innerWidth==={width} && innerHeight==={height} && devicePixelRatio===1', 'viewport')
        set_state('a_jour')
        eval_js(e, '() => { document.querySelector(".sidebar-content").scrollTop=0; return true; }')
        baseline = eval_js(e, f'() => ({geometry})()')
        capture(e, e / 'sidebar' / f'{theme}-{width}x{height}.png')

        def check(phase):
            result = eval_js(e, f'() => ({geometry})()')
            assert result['buttons'] == baseline['buttons'], (phase, result)
            assert result['rail'] == baseline['rail'], (phase, result)
            assert all(result[k] for k in ('contained', 'hitTargets', 'legible', 'separate', 'railChildrenContained')), (phase, result)
            checks.append({'theme': theme, 'size': [width, height], 'phase': phase, 'geometry': result})

        set_state('disponible')
        check('disponible')
        capture(e, e / 'updates' / f'{theme}-disponible-{width}x{height}.png')
        eval_js(e, '() => { document.querySelector(".update-corner-action").focus(); return true; }')
        browser(e, 'press', 'Enter')
        wait_js(e, '() => document.querySelector(".update-corner-label").textContent==="Téléchargement…"', 'action update clavier')
        check('telechargement')
        capture(e, e / 'updates' / f'{theme}-telechargement-{width}x{height}.png')
        for phase in ('compilation', 'installation', 'redemarrage', 'attente'):
            set_state(phase)
            check(phase)
            assert eval_js(e, '() => document.querySelector(".update-corner-action").disabled')
        set_state('erreur', {'message': error, 'last_error': error, 'last_attempt': 'echec'})
        check('erreur')
        capture(e, e / 'updates' / f'{theme}-erreur-{width}x{height}.png')
        eval_js(e, '() => { document.getElementById("settings-button").focus(); return true; }')
        browser(e, 'press', 'Enter')
        wait_js(e, '() => document.getElementById("settings-dialog").open', 'Réglages accessibles pendant erreur')
        assert eval_js(e, '() => document.querySelector(".updates-error").textContent') == error
        browser(e, 'press', 'Escape')
        eval_js(e, '() => { document.querySelector(".update-corner-dismiss").focus(); return true; }')
        browser(e, 'press', 'Enter')
        check('erreur fermée')
        eval_js(e, '() => { updatesFixture.state={...updatesFixture.state,message:"Service de mise à jour indisponible.",last_error:"Service de mise à jour indisponible."}; updatesFixture.failApply=true; document.querySelector(".update-corner-action").click(); return true; }')
        wait_js(e, '() => document.querySelector(".update-corner-error-text").textContent==="Service de mise à jour indisponible."', 'échec HTTP')
        check('échec HTTP')
        eval_js(e, '() => { updatesFixture.failApply=false; document.querySelector(".update-corner-dismiss").click(); document.querySelector(".update-corner-action").click(); return true; }')
        wait_js(e, '() => document.querySelector(".update-corner-label").textContent==="Téléchargement…"', 'nouvelle tentative')
        check('nouvelle tentative')
        set_state('disponible')
        scroll = eval_js(e, '''() => {
          const content=document.querySelector('.sidebar-content'); content.scrollTop=content.scrollHeight;
          const last=document.querySelector('#conversation-list .conversation-item:last-child');
          const r=last.getBoundingClientRect(), s=content.getBoundingClientRect();
          return {visible:r.top>=s.top && r.bottom<=s.bottom, last:r.toJSON(), content:s.toJSON()};
        }''')
        assert scroll['visible'], scroll
        check('liste défilée avec action disponible')
        capture(e, e / 'updates' / f'{theme}-defilement-{width}x{height}.png')
        set_state('a_jour')
        check('repos')
        assert eval_js(e, '() => document.querySelector(".update-corner").hidden')
        eval_js(e, '() => { document.querySelector(".sidebar-content").scrollTop=0; return true; }')

calls = eval_js(e, '() => updatesFixture.calls')
assert len([c for c in calls if c['path'] == '/api/updates/apply']) == 12
(e / 'updates-checks.json').write_text(json.dumps({'checks': checks, 'calls': calls, 'boundary': 'fetch /api/state et /api/updates/* simulés avant toute action. Aucun apply transmis au wrapper.'}, indent=2, ensure_ascii=False) + '\n')
print(f'{len(checks)} contrôles de rectangles et transitions réussis. 12 apply simulés.')
