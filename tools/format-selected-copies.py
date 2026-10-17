#!/usr/bin/env python3
"""Create only missing reading copies; never move anchors in existing copies."""
from pathlib import Path
import hashlib,json,subprocess,sys
ROOT=Path(__file__).resolve().parents[1]
def main():
 manifest={r['path']:r['sha256'] for r in json.loads((ROOT/'artifacts/manifest.json').read_text())['entries']}
 candidates=list((Path.home()/'.npm/_npx').glob('*/node_modules/prettier/bin/prettier.cjs'))
 tool=next(p for p in candidates if json.loads((p.parent.parent/'package.json').read_text())['version']=='3.6.2')
 rows=[]
 for name in sys.argv[1:]:
  rel=Path('dist/renderer/assets')/name
  src=ROOT/'artifacts/client'/rel;dst=ROOT/'artifacts/formatted'/rel
  assert src.is_file() and rel.as_posix() in manifest
  assert hashlib.sha256(src.read_bytes()).hexdigest()==manifest[rel.as_posix()]
  if dst.exists():continue
  dst.write_bytes(src.read_bytes())
  subprocess.run(['node',str(tool),'--ignore-path','/dev/null','--end-of-line','lf','--write',str(dst)],check=True,capture_output=True)
  rows.append({'path':str(rel),'original_sha256':manifest[str(rel)],'formatted_sha256':hashlib.sha256(dst.read_bytes()).hexdigest(),'lines_lf':len(dst.read_text().split('\n'))})
 print(json.dumps({'tool':str(tool),'version':'3.6.2','created':rows}))
if __name__=='__main__':main()
