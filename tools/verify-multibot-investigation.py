#!/usr/bin/env python3
"""Verify saved multibot sources and REA records without evaluating target JS."""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import datetime,hashlib,json,subprocess,sys
ROOT=Path(__file__).resolve().parents[1]
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def read(path):return json.loads((ROOT/path).read_text())
def run(args):
 result=subprocess.run([sys.executable,*args],cwd=ROOT,capture_output=True,text=True)
 assert result.returncode==0,(args,result.stdout,result.stderr)
 return {'argv':[sys.executable,*args],'result':json.loads(result.stdout)}
def main():
 commands=[['tools/evidence_ledger.py','evidence/multibot-membership-paths.json'],
           ['evidence/multibot-turn-paths-verify.py'],['tools/multibot-public-evidence.py']]
 with ThreadPoolExecutor(max_workers=3) as pool:checks=list(pool.map(run,commands))
 member=read('evidence/multibot-membership-paths.json')
 assert digest(ROOT/member['report'])==member['report_sha256_utf8']
 for source in member['sources']:
  assert digest(Path(source['inventory_path']))==source['inventory_sha256']
  inventory=json.loads(Path(source['inventory_path']).read_text())
  path=source['id'].split(':',1)[1]
  assert next(e['sha256'] for e in inventory['entries'] if e['path']==path)==source['original_sha256']
 provenance=read('evidence/multibot-rea-focus-input.json')
 source=ROOT/provenance['source_path'];subset=Path(provenance['subset_root'])/provenance['relative_path']
 assert digest(source)==digest(subset)==provenance['sha256']
 assert source.stat().st_size==provenance['bytes']==33283
 rea=read('evidence/multibot-rea-run.json');bundle_path=ROOT/rea['bundle']
 assert digest(bundle_path)==rea['bundle_sha256']
 bundle=json.loads(bundle_path.read_bytes());records={r['evidence_id']:r for r in bundle['records']}
 assert len(records)==15
 for key in ['original_parent_evidence_id','module_identities_evidence_id','original_module_evidence_id',
             'focused_parent_evidence_id','focused_summary_evidence_id','semantic_trace_evidence_id','module_trace_evidence_id']:
  assert rea[key] in records,key
 old=records[rea['original_parent_evidence_id']]
 assert old['subject']['digest']['sha256']==rea['original_asar_sha256']
 focused=records[rea['focused_summary_evidence_id']]['normalized_result']['summary']
 assert focused==rea['focused_summary']
 assert focused['statistics']['parsed_javascript_files']==1
 assert focused['coverage']['semantic']['status']=='unknown'
 trace=records[rea['semantic_trace_evidence_id']]['normalized_result']
 assert trace==rea['semantic_trace_result'] and trace['status']=='partial' and trace['summary']['total_seed_matches']==0
 module=records[rea['module_trace_evidence_id']]['normalized_result']
 assert module['summary']==rea['module_trace_summary'] and module['coverage']==rea['module_trace_coverage']
 cleanup=read(rea['cleanup_proof'])
 assert rea['producer_failure']['error']['code']=='cleanup_incomplete'
 assert cleanup['worker_absent'] and cleanup['empty_temp_removed'] and cleanup['target_code_executed'] is False
 outputs=['reports/multibot-membership-paths.md','reports/multibot-turn-paths.md','reports/multibot-public-usage.md',
          'reports/multibot-rea-followup.md','evidence/multibot-membership-paths.json','evidence/multibot-turn-paths.json',
          'evidence/multibot-public-sources.json','evidence/multibot-rea-run.json','evidence/multibot-rea-focus-input.json',
          'evidence/multibot-rea-worker-cleanup.json']
 result={'schema':1,'status':'passed','checked_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
         'scope':'Saved static UI/host .70/.71, anonymous product docs, retained REA subset and lifecycle evidence',
         'checks':checks,'membership_sources_manifest_bound':len(member['sources']),
         'rea_records_present':len(records),'rea_semantic_coverage':'unknown',
         'rea_producer_cleanup_error_retained':True,'recorded_worker_cleanup_verified':True,
         'bundle_sha256':rea['bundle_sha256'],'target_executed':False,'ios_runtime_verified':False,
         'server_implementation_recovered':False,'product_feature_implemented':False,
         'artifacts':[{'path':p,'sha256':digest(ROOT/p)} for p in outputs]}
 (ROOT/'evidence/multibot-investigation-verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps({'status':'passed','checks':len(checks),'source_bundles':len(member['sources']),
                   'rea_records':len(records),'target_executed':False,'product_feature_implemented':False}))
if __name__=='__main__':main()
