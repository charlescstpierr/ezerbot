#!/usr/bin/env python3
"""Save the investigation and shipped input without live browser/account profiles."""
from pathlib import Path
import json
import shutil
import subprocess

source = Path(__file__).resolve().parents[1]
destination = Path.home() / 'Desktop/grokbot-rust'
view = source.parent / '.lavish/grokbot'
app = Path('/Applications/Grok Bot.app')
if destination.exists():
    raise SystemExit(f'Refusing to overwrite existing handoff: {destination}')
shutil.copytree(source, destination, ignore=shutil.ignore_patterns('runtime-profile', '__pycache__'))
(destination/'inputs').mkdir()
subprocess.run(['/usr/bin/ditto', str(app), str(destination/'inputs/Grok Bot.app')], check=True)
(destination/'vue').mkdir()
for name in ('architecture.html', 'artifacts', 'evidence', 'reports'):
    current = view/name
    target = destination/'vue'/name
    if current.is_dir():
        shutil.copytree(current, target)
    else:
        shutil.copy2(current, target)
(destination/'vue/qa').mkdir()
for name in ('light.png', 'dark.png', 'narrow.png', 'code.png'):
    shutil.copy2(view/'qa'/name, destination/'vue/qa'/name)
for folder in (destination/'reports', destination/'vue/reports'):
    for file in folder.glob('*.md'):
        file.write_text(file.read_text().replace(str(source), str(destination)))
readme = destination/'README.md'
readme.write_text(readme.read_text().replace(str(view/'architecture.html'), 'vue/architecture.html'))
# Retained evidence records keep their original paths for provenance.
# Verification prefers the saved input and does not require the installed app.
verifier = destination/'tools/verify-dossier.py'
code = verifier.read_text()
code = code.replace("assert hashlib.sha256(Path(manifest['source']['path']).read_bytes()).hexdigest()==manifest['source']['sha256']",
    "saved_source=root/'inputs/Grok Bot.app/Contents/Resources/app.asar'\n"
    "source=saved_source if saved_source.exists() else Path(manifest['source']['path'])\n"
    "assert hashlib.sha256(source.read_bytes()).hexdigest()==manifest['source']['sha256']")
verifier.write_text(code)
builder = destination/'tools/build-review.py'
code = builder.read_text().replace("out = root.parent / '.lavish/grokbot'", "out = root / 'vue'")
builder.write_text(code)
print(json.dumps({'destination':str(destination),'app_copy':str(destination/'inputs/Grok Bot.app'),
    'excluded':['artifacts/runtime-profile', '.lavish/grokbot/visible-browser', 'desktop screenshots'],
    'raw_evidence_records_rewritten':False},ensure_ascii=False,indent=2))
