#!/usr/bin/env python3
"""Bounded literal searches over all saved JavaScript, without target execution."""
from pathlib import Path
import hashlib,json,re
ROOT=Path(__file__).resolve().parents[1]
PATTERNS={
 'direct_send_call':r'\.\s*sendGrokBotAgentMessage\s*\(',
 'room_turn_call':r'\.\s*(?:requestGrokBotRoomMemberTurn|cancelGrokBotRoomMemberTurn|deliverGrokBotRoomMemberTurnResult)\s*\(',
 'mobile_pairing_terms':r'(?i)qr.?code|pairing|pair.{0,12}(?:phone|device)|bluetooth|multipeer|nearby.?connection',
 'mobile_push_terms':r'(?i)\bAPNs?\b|\bFCM\b|push.?token|device.?token|register.?push|web.?push',
 'apns_specific':r'api(?:\.sandbox)?\.push\.apple\.com|didRegisterForRemoteNotifications|registerForRemoteNotifications|apns-token|apnsToken',
 'mobile_permission':r'mobile-tool-permission|mobileLocalTools|mobile_local_tools',
 'mobile_bridge_call':r'(?i)(?:resolve|request|register|connect|pair)Mobile[A-Za-z_]*',
 'shared_watch':r'watchGrokBotTranscripts',
}
def main():
 files=sorted(p for p in (ROOT/'artifacts/client').rglob('*') if p.suffix in {'.js','.mjs','.cjs'})
 manifest={r['path']:r['sha256'] for r in json.loads((ROOT/'artifacts/manifest.json').read_text())['entries']}
 records=[];searches={n:{'pattern':p,'hits':[]} for n,p in PATTERNS.items()}
 for path in files:
  raw=path.read_bytes();rel=str(path.relative_to(ROOT/'artifacts/client'));digest=hashlib.sha256(raw).hexdigest()
  assert manifest[rel]==digest
  records.append({'path':rel,'sha256':digest})
  text=raw.decode()
  for name,pattern in PATTERNS.items():
   for match in re.finditer(pattern,text):
    searches[name]['hits'].append({'path':rel,'offset_utf16':len(text[:match.start()].encode('utf-16-le'))//2,'matched':match[0],'preview':text[max(0,match.start()-70):match.end()+100]})
 for s in searches.values():s['count']=len(s['hits'])
 result={'target':'Grok Bot 0.70.2 saved macOS JavaScript','target_executed':False,'files':records,'file_count':len(files),
  'offset_unit':'UTF-16 code units in original decoded JavaScript','searches':searches,
  'limits':['Literal calls and terms only; aliases/computed dispatch and unshipped native/mobile/server bodies remain unknown.','A term in a translation or SDK descriptor is not an active feature.','No network, account, push registration or pairing scenario was performed.']}
 (ROOT/'evidence/interbot-mobile-search.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps({'files':len(files),'matches':{n:s['count'] for n,s in searches.items()}}))
if __name__=='__main__':main()
