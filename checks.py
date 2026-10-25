import json, subprocess, time
from pathlib import Path
wt = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i295')
e = Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295')
g = '/Users/cstpierrecharles/.treehouse/pm/_orch/bin/'
cargo = '/Users/cstpierrecharles/.cargo/bin/cargo'
commands = [('rust-pages-head', [g+'cargo-gate', cargo, 'test', '--locked', '-p', 'grokbot-wrapper', '--test', 'pages', '--', '--nocapture']), ('fmt-head', [g+'cargo-gate', cargo, 'fmt', '--all', '--', '--check']), ('ui-head', [g+'test-gate', 'node', '--test', 'app/ui/panels/pages.test.cjs', 'app/ui/panels/pages-browser.test.cjs', 'app/ui/draft-transfer.test.cjs'])]
head = subprocess.check_output(['git','rev-parse','HEAD'], cwd=wt, text=True).strip()
rows=[]
for name, cmd in commands:
    start=time.monotonic()
    with (e/(name+'.log')).open('w') as log:
        result=subprocess.run(cmd,cwd=wt,stdout=log,stderr=subprocess.STDOUT)
    row=dict(name=name,command=cmd,code=result.returncode,seconds=round(time.monotonic()-start,3),head=head,log=name+'.log')
    rows.append(row)
    (e/'checks.json').write_text(json.dumps(rows,indent=2)+'\n')
    print(name, 'exit', result.returncode, 'head', head, flush=True)
    if result.returncode: raise SystemExit(result.returncode)
