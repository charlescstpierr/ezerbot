#!/usr/bin/env python3
"""Create extra LF reading copies for the nine-topic investigation."""
from pathlib import Path
import subprocess,shutil,json,hashlib
ROOT=Path(__file__).resolve().parents[1]
NAMES=['chunk-auto-review-selector-BhU4DknL.js','chunk-team-setup-new-secret-dialog-CKCsKEFm.js','chunk-grok-bot-secrets-vYdoKF7p.js','chunk-view-CvFjZLO2.js','chunk-view-C686wUqG.js','chunk-provider-team-policy-DQumFZ2C.js','chunk-view-CO8DDi40.js','chunk-team-bot-details-dialog-n9GXyxFk.js']
def main():
 rows=[]
 for name in NAMES:
  rel=Path('dist/renderer/assets')/name
  src=ROOT/'artifacts/client'/rel;dst=ROOT/'artifacts/formatted'/rel
  shutil.copyfile(src,dst)
  rows.append({'path':str(rel),'original_sha256':hashlib.sha256(src.read_bytes()).hexdigest()})
 subprocess.run(['npx','--yes','prettier@3.6.2','--ignore-path','/dev/null','--end-of-line','lf','--write',*[str(ROOT/'artifacts/formatted'/r['path']) for r in rows]],check=True)
 for r in rows:
  p=ROOT/'artifacts/formatted'/r['path']
  r.update(formatted_sha256=hashlib.sha256(p.read_bytes()).hexdigest(),lines=p.read_text().count('\n'))
 (ROOT/'evidence/goal-formatting.json').write_text(json.dumps({'tool':'prettier@3.6.2','ignore_path':'/dev/null','additional_files':rows},indent=2)+'\n')
 print(json.dumps({'formatted':len(rows)}))
if __name__=='__main__':main()
