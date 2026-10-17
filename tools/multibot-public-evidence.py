#!/usr/bin/env python3
"""Capture public product documentation explicitly, or verify saved bodies offline."""
from pathlib import Path
from html.parser import HTMLParser
from concurrent.futures import ThreadPoolExecutor
import argparse,datetime,hashlib,json,re,urllib.request
ROOT=Path(__file__).resolve().parents[1]
BASE=Path('artifacts/public-multibot/2026-10-17')
LEDGER=Path('evidence/multibot-public-sources.json')
SOURCES=[
 ('xai-chat','https://docs.x.ai/grok-bot/chat-and-collaboration',
  ['two to six Bots','membership can be edited','asynchronous message','text-only','Messaging Between Bots']),
 ('cursor-groups','https://prod.cursor.com/help/grok-bot/group-chats',
  ['not alongside Bots','not in the group','no switch','one person']),
 ('xai-mobile','https://docs.x.ai/grok-bot/mobile',
  ['New Group Chat','manage group members']),
]
def sha(b):return hashlib.sha256(b).hexdigest()
class Text(HTMLParser):
 def __init__(self):super().__init__(convert_charrefs=True);self.parts=[];self.skip=0
 def handle_starttag(self,tag,attrs):
  if tag in {'script','style'}:self.skip+=1
 def handle_endtag(self,tag):
  if tag in {'script','style'}:self.skip=max(0,self.skip-1)
 def handle_data(self,data):
  if not self.skip:self.parts.append(data)
def visible(raw):
 p=Text();p.feed(raw.decode());return re.sub(r'\s+',' ',' '.join(p.parts)).strip()
def capture_one(source):
 ident,url,anchors=source
 request=urllib.request.Request(url,headers={'User-Agent':'Grokbot-local-evidence/1.0'})
 with urllib.request.urlopen(request,timeout=30) as response:
  raw=response.read();status=response.status;final=response.url
  headers={k:response.headers[k] for k in ['Content-Type','ETag','Last-Modified'] if k in response.headers}
 text=visible(raw);body=BASE/(ident+'.html');derived=BASE/(ident+'.txt')
 for anchor in anchors:assert anchor in text,(ident,anchor)
 (ROOT/body).write_bytes(raw);(ROOT/derived).write_text(text+'\n')
 return {'id':ident,'url':url,'final_url':final,'status':status,'headers':headers,
         'body_path':body.as_posix(),'body_sha256':sha(raw),
         'visible_text_path':derived.as_posix(),'visible_text_sha256':sha((text+'\n').encode()),
         'short_literal_anchors':[{'literal':a,'first_character_offset':text.index(a)} for a in anchors]}
def verify():
 d=json.loads((ROOT/LEDGER).read_text());n=0
 for source in d['sources']:
  raw=(ROOT/source['body_path']).read_bytes();assert sha(raw)==source['body_sha256']
  text=visible(raw);assert (ROOT/source['visible_text_path']).read_text()==text+'\n'
  assert sha((text+'\n').encode())==source['visible_text_sha256']
  for anchor in source['short_literal_anchors']:
   assert text.index(anchor['literal'])==anchor['first_character_offset'];n+=1
 return {'status':'passed','sources':len(d['sources']),'anchors':n,'verification_network_used':False}
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--capture',action='store_true');a=p.parse_args()
 if a.capture:
  assert not (ROOT/LEDGER).exists(),'Capture already exists; choose a new dated snapshot before refreshing'
  (ROOT/BASE).mkdir(parents=True,exist_ok=True)
  with ThreadPoolExecutor(max_workers=3) as pool:records=list(pool.map(capture_one,SOURCES))
  d={'schema':1,'captured_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
     'sources':records,'target_executed':False,'authenticated_requests':False,
     'scope':'Anonymous public documentation; product statements, not implementation/runtime proof',
     'limitations':['Documents can describe different releases and account scopes',
                    'Cursor no-switch statement and xAI admin setting are retained without equating their scopes',
                    'No one-to-one chat conversion is established by these documents']}
  (ROOT/LEDGER).write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(verify()))
if __name__=='__main__':main()
