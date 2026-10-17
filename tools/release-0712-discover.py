#!/usr/bin/env python3
"""Bounded, execution-free Grok Bot 0.71.2 acquisition/inventory comparison.

Does not install, launch or evaluate target code. No account/network endpoints
other than the exact public CDN DMG URL are supported by this script.
"""
from pathlib import Path
from datetime import datetime, timezone
import argparse, hashlib, json, os, plistlib, re, struct, subprocess, tempfile, shutil, urllib.request

ROOT=Path(__file__).resolve().parents[1]
URL='https://downloads.cursor.com/grokbot/stable/darwin-x64/0.71.2/Grok_Bot_0.71.2_x64.dmg'
REL=ROOT/'inputs/releases/0.71.2'
OUT=ROOT/'artifacts/releases/0.71.2'
EXPECTED_SIZE=156486357
def digest(b):return hashlib.sha256(b).hexdigest()
def hashfile(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
def save(p,obj):
 p.parent.mkdir(parents=True,exist_ok=True)
 p.write_text(json.dumps(obj,ensure_ascii=False,indent=2)+'\n')
def acquire():
 dest=REL/'Grok_Bot_0.71.2_x64.dmg';meta=REL/'acquisition.json'
 if dest.exists():
  assert meta.exists(),'Existing DMG has no acquisition provenance; refusing overwrite'
  record=json.loads(meta.read_text());assert dest.stat().st_size==record['bytes']==EXPECTED_SIZE
  assert hashfile(dest)==record['sha256'];print(json.dumps({'reused':True,**record}));return
 REL.mkdir(parents=True,exist_ok=True)
 part=dest.with_suffix('.dmg.part');assert not part.exists(),'Refusing to overwrite partial acquisition'
 class NoRedirect(urllib.request.HTTPRedirectHandler):
  def redirect_request(self,*args,**kwargs):return None
 request=urllib.request.Request(URL,headers={'User-Agent':'Mozilla/5.0'},method='GET')
 started=datetime.now(timezone.utc).isoformat();h=hashlib.sha256();size=0
 with urllib.request.build_opener(NoRedirect).open(request,timeout=60) as response:
  assert response.status==200 and response.url==URL
  headers=dict(response.headers.items());assert int(response.headers['Content-Length'])==EXPECTED_SIZE
  with part.open('xb') as f:
   while True:
    b=response.read(1024*1024)
    if not b:break
    f.write(b);h.update(b);size+=len(b)
    assert size<=EXPECTED_SIZE
 assert size==EXPECTED_SIZE
 record={'url':URL,'final_url':URL,'method':'GET','http_status':200,'started_at_utc':started,'completed_at_utc':datetime.now(timezone.utc).isoformat(),'headers':{k:v for k,v in headers.items() if k.lower() in ['content-type','content-length','last-modified','etag','date']},'path':str(dest.relative_to(ROOT)),'bytes':size,'sha256':h.hexdigest(),'authentication':False,'cookies':False,'redirects_followed':False,'installed':False,'executed':False,'provenance':'Exact official CDN URL discovered through official app download route HEAD302. SHA256 is observed content identity, not a publisher signature.'}
 os.rename(part,dest);save(meta,record);print(json.dumps(record))

def inventory_tree(root):
 entries=[]
 for p in sorted(root.rglob('*')):
  rel=str(p.relative_to(root))
  if p.is_symlink():entries.append({'path':rel,'kind':'symlink','target':os.readlink(p)})
  elif p.is_file():entries.append({'path':rel,'kind':'file','bytes':p.stat().st_size,'sha256':hashfile(p)})
 return entries

def copy_package():
 dest=OUT/'package';manifest=OUT/'mount-copy.json'
 if dest.exists():
  saved=json.loads(manifest.read_text())
  for row in saved['copied_files']:
   if row['kind']=='file':assert hashfile(dest/row['path'])==row['sha256']
  print(json.dumps({'reused':True,'copied_files':len(saved['copied_files'])}));return
 dmg=REL/'Grok_Bot_0.71.2_x64.dmg';meta=json.loads((REL/'acquisition.json').read_text());assert hashfile(dmg)==meta['sha256']
 mount=Path(tempfile.mkdtemp(prefix='grokbot0712-readonly-'));device=None
 command=['hdiutil','attach','-readonly','-nobrowse','-noautoopen','-plist','-mountroot',str(mount),str(dmg)]
 record={'attach_command':command,'target_executed':False,'copy_scope':['Contents/Info.plist','Contents/Resources/app.asar','Contents/Resources/app.asar.unpacked'],'copied_full_app':False}
 try:
  result=subprocess.run(command,capture_output=True,check=True);record['attach_stdout_sha256']=digest(result.stdout);data=plistlib.loads(result.stdout)
  entities=data['system-entities'];record['system_entities']=entities;device=entities[0]['dev-entry']
  apps=[p for e in entities if 'mount-point' in e for p in Path(e['mount-point']).glob('*.app')];assert len(apps)==1
  app=apps[0];info=plistlib.loads((app/'Contents/Info.plist').read_bytes());assert info['CFBundleShortVersionString']=='0.71.2'
  record['info']={k:info.get(k) for k in ['CFBundleIdentifier','CFBundleShortVersionString','CFBundleVersion']}
  save(OUT/'app-inventory.json',{'mounted_app_path':str(app),'entries':inventory_tree(app),'materialized_full_app':False})
  for name in record['copy_scope']:
   source=app/name;target=dest/name;target.parent.mkdir(parents=True,exist_ok=True)
   if source.is_dir():shutil.copytree(source,target,symlinks=True)
   else:shutil.copyfile(source,target)
  record['copied_files']=inventory_tree(dest)
 finally:
  if device:
   result=subprocess.run(['hdiutil','detach',device],capture_output=True)
   record['detach']={'device':device,'exit_code':result.returncode,'stdout':result.stdout.decode(),'stderr':result.stderr.decode()};assert result.returncode==0
  mount.rmdir();record['owned_mountpoint_removed']=not mount.exists();save(manifest,record)
 print(json.dumps({'version':record.get('info'),'copied_regular_files':sum(x['kind']=='file' for x in record['copied_files']),'detach':record.get('detach')}))

def extract_asar(asar,dest):
 """Parse Chromium pickle ASAR header; copy only regular members, never eval JS."""
 assert not dest.exists(),'Destination must be fresh'
 with asar.open('rb') as f:
  outer=f.read(8);size_size,header_size=struct.unpack('<II',outer)
  assert size_size==4 and 8<header_size<16_000_000
  header=f.read(header_size);pickle_size,json_size=struct.unpack('<II',header[:8])
  assert json_size<=len(header)-8
  obj=json.loads(header[8:8+json_size]);payload_offset=8+header_size
  entries=[];dest.mkdir(parents=True)
  def walk(node,prefix):
   for name,n in sorted(node['files'].items()):
    assert name not in ['.','..'] and '/' not in name and '\\' not in name
    rel=prefix/name;path=dest/rel
    if 'files' in n:walk(n,rel);continue
    if 'link' in n:
     entries.append({'path':str(rel),'kind':'symlink','target':n['link'],'materialized':False});continue
    size=n['size'];assert size>=0
    if n.get('unpacked'):
     source=Path(str(asar)+'.unpacked')/rel;assert source.is_file()
     data=source.read_bytes();origin={'unpacked':True,'source_path':str(source.relative_to(ROOT))}
    else:
     off=payload_offset+int(n['offset']);f.seek(off);data=f.read(size)
     origin={'unpacked':False,'asar_file_offset':off}
    if not n.get('unpacked'):assert len(data)==size
    path.parent.mkdir(parents=True,exist_ok=True)
    with path.open('xb') as out:out.write(data)
    h=digest(data);integrity=n.get('integrity')
    trust='no-declared-integrity'
    if integrity and integrity.get('algorithm')=='SHA256':
     trust='matches' if h==integrity['hash'] else 'observed-untrusted'
     if trust=='observed-untrusted':assert n.get('unpacked'),'Unexpected packed member integrity contradiction: '+str(rel)
    entries.append({'path':str(rel),'kind':'file','bytes':len(data),'declared_bytes':size,'sha256':h,**origin,'integrity':integrity,'integrity_status':trust})
  walk(obj,Path())
 manifest={'asar_path':str(asar.relative_to(ROOT)),'asar_sha256':hashfile(asar),'output_root':str(dest.relative_to(ROOT)),'payload_offset':payload_offset,'header_sha256':digest(header),'entries':entries,'extractor':'tools/release-0712-discover.py pure Python struct/json copy, no target code execution','symlinks_materialized':False,'integrity_checked':True,'integrity_contradictions':[e for e in entries if e.get('integrity_status')=='observed-untrusted']}
 save(OUT/'asar-inventory.json',manifest);return manifest

def compare():
 old=ROOT/'artifacts/client';new=OUT/'client'
 manifest=json.loads((OUT/'asar-inventory.json').read_text())
 newrows={e['path']:e for e in manifest['entries'] if e['kind']=='file'}
 oldrows={e['path']:e for e in inventory_tree(old) if e['kind']=='file'}
 common=sorted(oldrows.keys()&newrows.keys())
 unchanged=[p for p in common if oldrows[p]['sha256']==newrows[p]['sha256']]
 changed=[p for p in common if oldrows[p]['sha256']!=newrows[p]['sha256']]
 added=sorted(newrows.keys()-oldrows.keys());removed=sorted(oldrows.keys()-newrows.keys())
 def identity(p):return re.sub(r'-[A-Za-z0-9_-]{8}(?=\.(?:js|css)$)','-<build-hash>',p)
 oldnorm={};newnorm={}
 for rows,norm in [(oldrows,oldnorm),(newrows,newnorm)]:
  for p in rows:norm.setdefault(identity(p),[]).append(p)
 normalized_added=sorted(newnorm.keys()-oldnorm.keys());normalized_removed=sorted(oldnorm.keys()-newnorm.keys())
 categories=[]
 for needle in ['server','harness','schedul','context','compaction','memory','room','interbot']:
  categories.append({'needle':needle,'old_paths':[p for p in sorted(oldrows) if needle in p.lower()],'new_paths':[p for p in sorted(newrows) if needle in p.lower()]})
 searches=[]
 literals=['sendGrokBotAgentMessage','SendGrokBotAgentMessage','sourceChannel','targetChannel','source_channel','target_channel','replyChannel','promoteGrokBotMemoriesToTeam','startGrokBotImagePrecaption','grokBotAgentContextCompacted','grokBotAgentContext','recordingToSkill','recording-to-skill','client_speculative_summarization_config','tokenUsageThresholdPercentage','getGrokBotTeamContextSummary','sourceMappingURL=']
 for literal in literals:
  results={}
  for label,base,rows in [('0.70.2',old,oldrows),('0.71.2',new,newrows)]:
   found=[]
   for p in sorted(rows):
    if Path(p).suffix not in ['.js','.cjs','.mjs']:continue
    b=(base/p).read_bytes();term=literal.encode();count=b.count(term)
    if count:
     pos=b.index(term);found.append({'path':p,'sha256':rows[p]['sha256'],'count':count,'first_byte_offset':pos,'context':b[max(0,pos-80):pos+len(term)+120].decode(errors='replace')})
   results[label]=found
  searches.append({'literal':literal,'matches':results,'meaning':'Lexical locator only; findings require focused caller/body inspection. No semantic analysis over full corpus.'})
 delta={'baseline_root':'artifacts/client','new_root':str(new.relative_to(ROOT)),'baseline_file_count':len(oldrows),'new_file_count':len(newrows),'javascript_counts':{'0.70.2':sum(Path(p).suffix in ['.js','.cjs','.mjs'] for p in oldrows),'0.71.2':sum(Path(p).suffix in ['.js','.cjs','.mjs'] for p in newrows)},'same_path_unchanged':unchanged,'same_path_changed':changed,'added_paths':added,'removed_paths':removed,'hash_normalized_added_paths':normalized_added,'hash_normalized_removed_paths':normalized_removed,'hash_normalization':'Replace final 8-char build hash immediately before .js/.css; filename correspondence heuristic only, not source equivalence. Collisions retained as groups.','ambiguous_normalized_groups':{k:{'0.70.2':oldnorm.get(k,[]),'0.71.2':newnorm.get(k,[])} for k in sorted(oldnorm.keys()|newnorm.keys()) if len(oldnorm.get(k,[]))>1 or len(newnorm.get(k,[]))>1},'filename_categories':categories,'source_map_paths':{'0.70.2':[p for p in oldrows if p.endswith('.map')],'0.71.2':[p for p in newrows if p.endswith('.map')]},'mobile_package_paths':{'0.70.2':[p for p in oldrows if Path(p).suffix in ['.ipa','.apk','.aab']],'0.71.2':[p for p in newrows if Path(p).suffix in ['.ipa','.apk','.aab']]},'bounded_literal_searches':searches}
 save(OUT/'inventory-comparison.json',delta)
 print(json.dumps({'old_files':len(oldrows),'new_files':len(newrows),'unchanged':len(unchanged),'changed_same_path':len(changed),'added':len(added),'removed':len(removed),'normalized_added':normalized_added,'normalized_removed':normalized_removed,'same_path_changed':changed,'categories':categories,'source_maps':delta['source_map_paths']}))

def format_selected():
 candidates=list((Path.home()/'.npm/_npx').glob('*/node_modules/prettier/bin/prettier.cjs'))
 tool=next(p for p in candidates if json.loads((p.parent.parent/'package.json').read_text())['version']=='3.6.2')
 selected=['dist/local-exec-daemon/messages-worker.cjs','dist/local-exec-daemon/main.cjs','dist/electron-main/main-app.cjs','dist/electron-main/main-core.cjs','dist/electron-main/proto.cjs','dist/node-agent-coordinator/main.cjs']
 selected += [str(p.relative_to(OUT/'client')) for p in (OUT/'client/dist/renderer/assets').glob('index.eager-app-*.js')]
 selected += [str(p.relative_to(OUT/'client')) for prefix in ['chunk-auto-review-approval-','chunk-provider-computer-use-'] for p in (OUT/'client/dist/renderer/assets').glob(prefix+'*.js')]
 records=[]
 for name in selected:
  raw=OUT/'client'/name;dest=OUT/'formatted'/name
  if not dest.exists():
   dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(raw.read_bytes())
   subprocess.run(['node',str(tool),'--ignore-path','/dev/null','--end-of-line','lf','--write',str(dest)],check=True,capture_output=True)
  records.append({'original_path':str(raw.relative_to(ROOT)),'original_sha256':hashfile(raw),'formatted_path':str(dest.relative_to(ROOT)),'formatted_sha256':hashfile(dest),'lines':len(dest.read_text().split('\n'))})
 save(OUT/'formatting-selected.json',{'tool':str(tool),'tool_sha256':hashfile(tool),'version':'3.6.2','target_code_executed':False,'derived_copies':records});print(json.dumps(records))

def verify():
 """Offline replay: hashes, REA inventory, integrity, anchors and comparisons."""
 result={'schema_version':1,'verified_at_utc':datetime.now(timezone.utc).isoformat(),'offline':True,'network_used':False,'mount_used':False,'reformatted':False,'target_executed':False,'javascript_evaluated':False,'checks':{}}
 verification_path=ROOT/'evidence/releases/0.71.2/verification.json'
 try:
  ledger_path=ROOT/'evidence/release-0712-discovery.json';ledger=json.loads(ledger_path.read_text());result['ledger_sha256']=hashfile(ledger_path)
  acquisition=json.loads((REL/'acquisition.json').read_text());dmg=ROOT/acquisition['path'];assert dmg.stat().st_size==EXPECTED_SIZE==acquisition['bytes'];assert hashfile(dmg)==acquisition['sha256']==ledger['target']['dmg_sha256'];result['checks']['dmg_identity']=True
  manifest=json.loads((OUT/'asar-inventory.json').read_text());asar=ROOT/manifest['asar_path'];assert hashfile(asar)==manifest['asar_sha256']==ledger['target']['asar_sha256'];result['checks']['asar_identity']=True
  bundle=json.loads((ROOT/ledger['artifacts']['rea_bundle']['path']).read_text());inventory=next(r for r in bundle['records'] if r.get('operation')=='inventory_artifact');graph=inventory['normalized_result'];nodes={n['artifact_id']:n for n in graph['nodes']};occurrences={o['logical_path'].split('/Contents/Resources/app.asar/',1)[1]:o for o in graph['occurrences'] if '/Contents/Resources/app.asar/' in o['logical_path']}
  entries=[e for e in manifest['entries'] if e['kind']=='file'];assert len(entries)==668
  mismatches=[]
  for entry in entries:
   path=OUT/'client'/entry['path'];observed=hashfile(path);assert observed==entry['sha256']==nodes[occurrences[entry['path']]['artifact_id']]['sha256'];assert path.stat().st_size==entry['bytes']
   declared=entry.get('integrity',{});declared_hash=declared.get('hash') if declared else None
   if declared_hash is not None and observed!=declared_hash:
    assert entry['unpacked'] and entry['integrity_status']=='observed-untrusted';mismatches.append(entry['path'])
   elif declared_hash is not None:assert entry['integrity_status']=='matches'
  assert len(mismatches)==len(manifest['integrity_contradictions'])==len(graph['integrity_contradictions'])==6
  assert set(mismatches)=={e['path'] for e in manifest['integrity_contradictions']}
  result['checks']['members_match_rea_inventory']=668;result['checks']['native_integrity_contradictions_explicit']=6
  cache={}
  for alias,source in ledger['sources'].items():
   assert hashfile(ROOT/source['original_path'])==source['original_sha256'];assert hashfile(ROOT/source['formatted_path'])==source['formatted_sha256'];cache[alias]=(ROOT/source['formatted_path']).read_text().split('\n')
  anchors={r['id']:r for r in ledger['anchors']}
  for anchor in anchors.values():
   lines=cache[anchor['source_alias']];start=anchor['lines']['start'];end=anchor['lines']['end'];assert 1<=start<=end<=len(lines)
   fragment='\n'.join(lines[start-1:end]);assert fragment==anchor['excerpt'] and anchor['anchor'] in fragment and digest(fragment.encode())==anchor['excerpt_sha256']
  result['checks']['source_hashes']=len(ledger['sources']);result['checks']['anchors']=len(anchors)
  for row in ledger['focused_fragment_comparisons']:
   if 'baseline_anchor' in row:
    left=anchors[row['baseline_anchor']]['excerpt'];right=anchors[row['new_anchor']]['excerpt']
    for old,new in row['identifier_substitutions'].items():left=re.sub(r'(?<![A-Za-z0-9_$])'+re.escape(old)+r'(?![A-Za-z0-9_$])',new,left)
    assert left==right and digest(left.encode())==row['normalized_excerpt_sha256']
   else:
    left=next(x.strip() for x in cache[row['baseline_alias']] if row['literal_anchor'] in x);right=next(x.strip() for x in cache[row['new_alias']] if row['literal_anchor'] in x);assert left==right==row['serialized_descriptor'] and digest(left.encode())==row['descriptor_sha256']
  result['checks']['focused_fragment_comparisons']=len(ledger['focused_fragment_comparisons'])
  for artifact in ledger['artifacts'].values():assert hashfile(ROOT/artifact['path'])==artifact['sha256']
  report=ROOT/ledger['report'];assert hashfile(report)==ledger['verification']['report_sha256'];result['checks']['artifact_and_report_hashes']=True
  for alias,start,end in re.findall(r'\b([A-Z]+7[01]):(\d+)(?:[–-](\d+))?',report.read_text()):assert 1<=int(start)<=int(end or start)<=len(cache[alias])
  result['checks']['report_line_ranges_exist']=True
  mount=json.loads((OUT/'mount-copy.json').read_text());assert mount['detach']['exit_code']==0 and mount['owned_mountpoint_removed'];result['checks']['recorded_mount_cleanup']=True
  result['status']='passed'
 except Exception as error:
  result['status']='failed';result['error']=type(error).__name__+': '+str(error);save(verification_path,result);raise
 save(verification_path,result);print(json.dumps(result))

if __name__=='__main__':
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('action',choices=['acquire','copy-package','extract-asar','compare','inventory-app','format-selected','verify']);parser.add_argument('--asar');parser.add_argument('--app');args=parser.parse_args()
 if args.action=='acquire':acquire()
 elif args.action=='copy-package':copy_package()
 elif args.action=='extract-asar':
  assert args.asar;manifest=extract_asar(Path(args.asar).resolve(),OUT/'client');print(json.dumps({'entries':len(manifest['entries']),'asar_sha256':manifest['asar_sha256'],'output_root':manifest['output_root']}))
 elif args.action=='inventory-app':
  assert args.app;p=Path(args.app).resolve();records=inventory_tree(p);save(OUT/'app-inventory.json',{'root':str(p.relative_to(ROOT)),'entries':records});print(json.dumps({'entries':len(records),'files':sum(e['kind']=='file' for e in records),'symlinks':sum(e['kind']=='symlink' for e in records)}))
 elif args.action=='format-selected':format_selected()
 elif args.action=='verify':verify()
 else:compare()
