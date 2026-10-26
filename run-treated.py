import json, os, subprocess, sys
from pathlib import Path
wt=Path('/Users/cstpierrecharles/.treehouse/pm/ez-i295')
e=Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295/r2')
launcher=e/'launcher-home'
launcher.mkdir(exist_ok=True)
env=dict(os.environ, HOME=str(launcher), CFFIXED_USER_HOME=str(launcher), TMPDIR=str(e/'tmp'),
    CARGO_HOME='/Users/cstpierrecharles/.cargo', RUSTUP_HOME='/Users/cstpierrecharles/.rustup',
    CARGO_GATE_DIR='/Users/cstpierrecharles/.treehouse/pm/_orch/locks/cargo',
    TEST_GATE_DIR='/Users/cstpierrecharles/.treehouse/pm/_orch/locks/test',
    EZERBOT_VERIFY_CARGO='/Users/cstpierrecharles/.cargo/bin/cargo',
    EZERBOT_VERIFY_CARGO_GATE='/Users/cstpierrecharles/.treehouse/pm/_orch/bin/cargo-gate',
    CHROME_DEVTOOLS_AXI_SESSION='pm-i295')
scripts=wt/'.cursor/skills/verify-ezerbot/scripts'
steps=[('launch',['bash',str(scripts/'launch.sh'),'--sphere','pages','--evidence',str(e/'treated')]),
    ('drive',['python3',str(e/'drive-treated.py')])]
rows=[]
for name,cmd in steps:
    with (e/(name+'-treated.log')).open('w') as log:
        result=subprocess.run(cmd,cwd=wt,env=env,stdout=log,stderr=subprocess.STDOUT)
    rows.append(dict(name=name,command=cmd,code=result.returncode))
    (e/'run-treated.json').write_text(json.dumps(rows,indent=2)+'\n')
    print(name,'exit',result.returncode,flush=True)
    if result.returncode:
        with (e/'cleanup-after-treated-failure.log').open('w') as log:
            cleanup=subprocess.run(['bash',str(scripts/'cleanup.sh'),'--evidence',str(e/'treated')],cwd=wt,env=env,stdout=log,stderr=subprocess.STDOUT)
        print('cleanup after failure exit',cleanup.returncode,flush=True)
        sys.exit(result.returncode)
