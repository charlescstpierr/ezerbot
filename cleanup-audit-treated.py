import json, os, sys, time
from pathlib import Path
e=Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295/r2/treated')
os.environ['TMPDIR']=str(e.parent/'tmp')
sys.path.insert(0,'/Users/cstpierrecharles/.treehouse/pm/ez-i295/.cursor/skills/verify-ezerbot/scripts')
from verify_lib import load_run, load_browser, identity_matches, assert_created_run_root, assert_created_home
run=load_run(e)
root=assert_created_run_root(Path(run['run_root']),run)
home=assert_created_home(Path(run['home']),root)
assert run['session']=='pm-i295'
records={name:(run[name+'_pid'],run[name+'_identity']) for name in ('wrapper','fake','auth')}
browser=load_browser(run)
assert browser and browser['started_by']=='verify-ezerbot'
records['chrome']=(browser['chrome_pid'],browser['identity'])
for _ in range(600):
    receipt=e/'cleanup.json'
    if receipt.exists() and json.loads(receipt.read_text()).get('cleaned'): break
    time.sleep(1)
else: raise SystemExit('Cleanup non terminé après dix minutes')
ours={name:dict(pid=pid,still_matches=identity_matches(pid,identity)) for name,(pid,identity) in records.items()}
result=dict(head=run['build_sha'],session=run['session'],signed_root_valid=True,signed_home_valid=True,home_still_present=home.exists(),root_still_present=root.exists(),processes=ours)
(e/'cleanup-audit.json').write_text(json.dumps(result,indent=2)+'\n')
assert not home.exists() and not root.exists() and not any(row['still_matches'] for row in ours.values())
print('cleanup audit ok',run['build_sha'],run['session'])
