#!/usr/bin/env python3
"""Create readable analysis copies. Original extracted bytes remain unchanged."""
import json
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
client = root / 'artifacts/client'
destination = root / 'artifacts/formatted'
selected = list((client / 'dist/electron-main').glob('*.cjs'))
selected += list((client / 'dist/electron-preload').glob('*.cjs'))
selected += list((client / 'dist/local-exec-daemon').glob('*.cjs'))
selected += list((client / 'dist/node-agent-coordinator').glob('*.cjs'))
selected += list((client / 'dist/renderer/assets').glob('index*.js'))
selected += list((client / 'dist/renderer/assets').glob('desktop-mount-panel*.js'))
selected += list((client / 'dist/renderer/assets').glob('index*.css'))
outputs = []
for source in sorted(selected):
    target = destination / source.relative_to(client)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    outputs.append(target)
subprocess.run(['npx', '--yes', 'prettier@3.6.2', '--write', *map(str, outputs)], check=True)
mapping = [{'source':str(p.relative_to(destination)), 'lines':p.read_text().count('\n')} for p in outputs]
(root / 'evidence/formatting.json').write_text(json.dumps({'tool':'prettier@3.6.2','files':mapping},indent=2)+'\n')
print(json.dumps({'formatted_files':len(outputs),'output':str(destination)},indent=2))
