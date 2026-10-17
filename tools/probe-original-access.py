#!/usr/bin/env python3
"""Bounded inventory and optional fresh-profile startup of the selected original.

Does not read account profiles, credentials or device identifiers. The optional
launch uses the packaged executable unchanged and Chromium CDP instead of the
previously failed Playwright/Node-inspector startup path.
"""
from pathlib import Path
import argparse,datetime,hashlib,json,os,plistlib,shutil,socket,subprocess,tempfile,time,urllib.request

ROOT=Path(__file__).resolve().parents[1]
TARGET=ROOT/'inputs/Grok Bot.app'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def run(args):
    p=subprocess.run(args,capture_output=True,text=True,timeout=20)
    return p.returncode,p.stdout,p.stderr
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--launch',action='store_true')
    ap.add_argument('--out',type=Path,help='New JSON output path; existing evidence is never replaced.')
    a=ap.parse_args()
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    output=a.out or ROOT/'evidence'/('access-inventory-'+stamp+'.json')
    assert not output.exists(),'Choose a new output path to preserve historical evidence.'
    value={'schema':1,'observed_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
      'scope':'Selected Grok Bot app, supplied inputs/evidence and availability of local iPhone tools',
      'credentials_or_user_profiles_read':False,'target_executed':False}
    apps=[]
    for p in [TARGET,Path('/Applications/Grok Bot.app')]:
        if not p.exists():continue
        d=plistlib.loads((p/'Contents/Info.plist').read_bytes())
        apps.append({'path':str(p),'bundle_id':d.get('CFBundleIdentifier'),'version':d.get('CFBundleShortVersionString'),
          'asar_sha256':sha(p/'Contents/Resources/app.asar'),'executable_sha256':sha(p/'Contents/MacOS'/d['CFBundleExecutable'])})
    value['apps']=apps
    rc,out,err=run(['ps','-axo','pid=,comm='])
    value['process_inventory']={'exit_code':rc,'matching_processes':[{'pid':int(s.strip().split(None,1)[0]),'executable':s.strip().split(None,1)[1]}
       for s in out.split('\n') if 'grok' in s.lower() or 'cugrok' in s.lower()],
       'original_active':any('/Grok Bot.app/' in s for s in out.split('\n'))}
    value['supplied_mobile_or_network_captures']=[str(p.relative_to(ROOT)) for name in ['inputs','evidence']
       for p in (ROOT/name).rglob('*') if p.is_file() and p.suffix.lower() in {'.ipa','.har','.pcap','.pcapng','.mitm'}]
    value['device_tools']={n:shutil.which(n) for n in ['idevice_id','ideviceinstaller','devicectl']}
    rc,out,err=run(['xcrun','--find','devicectl'])
    value['xcrun_devicectl']={'exit_code':rc,'available':rc==0,'path':out.strip() if rc==0 else None}
    rc,out,err=run(['system_profiler','SPUSBDataType','-json'])
    names=[]
    def walk(x):
        if isinstance(x,dict):
            if isinstance(x.get('_name'),str):names.append(x['_name'])
            for y in x.values():walk(y)
        elif isinstance(x,list):
            for y in x:walk(y)
    try:walk(json.loads(out))
    except json.JSONDecodeError:pass
    value['usb_inventory']={'exit_code':rc,'iphone_names':[n for n in names if 'iphone' in n.lower()],
       'enumeration_reported_errors':bool(err.strip()),'serial_numbers_retained':False,
       'limitation':'No iPhone name in this bounded USB inventory does not exclude wireless pairing or unenumerated devices.'}
    if a.launch:
        assert not value['process_inventory']['original_active'],'Original already active; use passive discovery instead.'
        profile=Path(tempfile.mkdtemp(prefix='rea-original-profile-',dir=ROOT/'artifacts'))
        with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
        executable=TARGET/'Contents/MacOS/Grok Bot'
        args=[str(executable),'--user-data-dir='+str(profile),'--remote-debugging-address=127.0.0.1','--remote-debugging-port='+str(port)]
        env={k:v for k,v in os.environ.items() if k in {'HOME','USER','LOGNAME','PATH','TMPDIR','LANG','LC_CTYPE','SHELL','__CF_USER_TEXT_ENCODING'}}
        env.update(SAND_USER_DATA_DIR=str(profile),SAND_DISABLE_UPDATES='1')
        with tempfile.TemporaryFile() as log:
            p=subprocess.Popen(args,env=env,stdout=log,stderr=log,start_new_session=True,cwd=ROOT)
            deadline=time.monotonic()+12;ready=False;product=None
            while time.monotonic()<deadline and p.poll() is None:
                try:
                    with urllib.request.urlopen('http://127.0.0.1:'+str(port)+'/json/version',timeout=.4) as res:
                        d=json.load(res)
                    product=d.get('Browser');ready=True;break
                except Exception:time.sleep(.15)
            log.seek(0);data=log.read()
        value['target_executed']=True
        value['startup_probe']={'pid':p.pid,'args':args,'environment_keys':sorted(env),'updates_disabled':True,
           'fresh_profile':str(profile),'exit_code':p.poll(),'cdp_ready':ready,'cdp_endpoint':'http://127.0.0.1:'+str(port) if ready else None,
           'browser_product':product,'log_bytes':len(data),'log_sha256':hashlib.sha256(data).hexdigest(),
           'log_values_retained':False,'known_error_markers':[s for s in ['ASAR','Integrity','Error','FATAL','invariant','dyld','DevTools listening'] if s.encode() in data],
           'process_ownership':'analyst-owned','network_containment':False,'authenticated':False,
           'method':'Native packaged executable with fresh user-data directory and Chromium CDP; no Node inspector, Playwright launch or modified fuse.'}
        if p.poll() is None and not ready:
            p.terminate()
            try:p.wait(timeout=5)
            except subprocess.TimeoutExpired:p.kill();p.wait(timeout=5)
            value['startup_probe']['cleanup']='terminated-owned-no-cdp-process'
            value['startup_probe']['exit_code']=p.returncode
    with output.open('x') as f:f.write(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({'evidence':str(output),'original_active_before':value['process_inventory']['original_active'],
      'supplied_captures':len(value['supplied_mobile_or_network_captures']),'devicectl':value['xcrun_devicectl']['available'],
      'iphone_usb_names':value['usb_inventory']['iphone_names'],'startup':value.get('startup_probe')},ensure_ascii=False))
if __name__=='__main__':main()
