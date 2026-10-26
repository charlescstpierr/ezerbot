import argparse
import json
import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js, theme_js

parser = argparse.ArgumentParser()
parser.add_argument('evidence', type=Path)
parser.add_argument('--seed-only', action='store_true')
args = parser.parse_args()
e = args.evidence
summary = eval_js(e, '''async () => {
  const post = async (path, body) => {
    const r = await fetch(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)});
    if (!r.ok) throw new Error(path + ' HTTP ' + r.status);
    return r.json();
  };
  for (const [id, name, color] of [['atelier','Atelier','cyan'],['lecture','Lecture','menthe'],['projets','Projets','violet'],['cuisine','Cuisine','orange']]) {
    if (!GrokbotPanels.state.bots.some(b => b.id === id)) await post('/api/bots', {id,name,color,shape:'galet'});
  }
  const titles = ['Plan en trois étapes', 'Relire l’annonce', 'Idées de nom', 'Explorer le code Rust'];
  for (let i = 0; i < 32; i++) await post('/api/conversations', {bot_id:'grokbot', title:titles[i] || 'Notes du projet ' + (i + 1)});
  await GrokbotPanels.refresh();
  return {bots:GrokbotPanels.state.bots.length, conversations:GrokbotPanels.state.conversations.length};
}''')
wait_js(e, '() => document.querySelectorAll(".sidebar-bot").length >= 5 && document.querySelectorAll("#conversation-list .conversation-item").length >= 32', 'données latérales absentes')
if args.seed_only:
    (e / 'seed-summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary))
    raise SystemExit(0)
shots = e / 'sidebar'
shots.mkdir(exist_ok=True)
for theme in ('dark', 'light'):
    eval_js(e, theme_js(theme, True))
    for width, height in ((1440,900), (820,560)):
        browser(e, 'resize', str(width), str(height))
        eval_js(e, '() => { const s = document.querySelector(".sidebar-content") || document.getElementById("sidebar"); s.scrollTop=0; document.getElementById("conversation-list").scrollTop=0; return true; }')
        browser(e, 'screenshot', str(shots / f'{theme}-{width}x{height}.png'))
        (shots / f'{theme}-{width}x{height}.txt').write_text(eval_js(e, '() => document.getElementById("sidebar").innerText'))
(e / 'seed-summary.json').write_text(json.dumps(summary, indent=2)+'\n')
print(json.dumps(summary))
