import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { createVoiceReader, voiceNeedsAction } from "./voice-conversation.js";
class Element {
  constructor(tag) { this.tagName=tag; this.children=[]; this.dataset={}; this.attributes={}; this.value=""; }
  append(...children) { for (const child of children) {child.parent=this;this.children.push(child);} }
  before(child) {this.parent.append(child);}
  setAttribute(k,v) {this.attributes[k]=String(v);}
}
const all = n => [n,...n.children.flatMap(all)];
const cls = (n,c) => all(n).find(x => x.className === c);
function fixture() {
  let panel, timer, keydown, source={status:"idle",transcript:"",utterance_id:0,barged:0}, scenario="", nextSend;
  const calls=[], sends=[];
  const root=new Element("div"), send=new Element("button"), settings=new Element("div"), composer=new Element("div");root.append(send,settings,composer);
  let context={connected:true,conversation:{id:"chat",messages:[]},state:{voice:{settings:{voice:"",speed:1}}}};
  const bridge=async body => {
    calls.push(body);
    if(body.command === "options")return {voices:[]};
    if(body.command === "start")source={status:scenario || "listening",token:body.token,transcript:"",utterance_id:0,barged:0};
    if(body.command === "speak")source.speaking=true;
    if(body.command === "interrupt") { source.speaking=false;source.transcript=""; }
    if(body.command === "cancel")source.status="idle";
    if(body.command === "pause")source.paused=true;
    if(body.command === "resume")source.paused=false;
    return {...source};
  };
  const panels={
    register(p){ panel=p;p.settingsSection(settings,context);p.composer.mount(composer,context);p.onState(context.state,context);},
    composerChanged(){}, notify(){}, async refresh(){}, async api(path,body){calls.push({path,...body});if(path==="/api/conversations")return {conversation:{id:"created"}};},
    openConversation(id){context={...context,conversation:{id,messages:[]}};panel.onState(context.state,context);return true;},
    async sendVoice(text,id){sends.push({text,id});nextSend?.();},
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice-conversation.js",import.meta.url),"utf8"),{
    window:{GrokbotPanels:panels,GrokbotChat:{createVoiceReader,voiceNeedsAction},webkit:{messageHandlers:{ezerbotVoiceSession:{postMessage:bridge}}},addEventListener(){}},
    document:{createElement:tag=>new Element(tag),getElementById:()=>send,addEventListener(name,fn){if(name==="keydown")keydown=fn;}},
    crypto:{randomUUID:()=>"token"},setTimeout(fn){timer=fn;return 1;},clearTimeout(){timer=null;},
  });
  return {root,calls,sends,panel,get source(){return source;},set scenario(s){scenario=s;},set nextSend(fn){nextSend=fn;},get context(){return context;},
    setContext(c){context=c;panel.onState(c.state,c);},tick:async()=>timer?.(),keydown:e=>keydown(e)};
}
test("end-to-end voice sends only the recognized utterance once, reads streamed sentences, pauses on approval",async()=>{
  const f=fixture(); await cls(f.root,"voice-session-toggle").onclick();
  f.source.transcript="Bonjour";f.source.barged=1;await f.tick();
  assert.equal(cls(f.root,"voice-live-transcript").textContent,"Bonjour");
  f.source.final_text="Bonjour Ezerbot.";f.source.utterance_id=1;
  f.nextSend=()=>f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bonjour. Je vous écoute"});
  await f.tick();await f.tick();
  assert.deepEqual(f.sends,[{text:"Bonjour Ezerbot.",id:"chat"}]);
  assert.equal(f.calls.filter(c=>c.command==="speak").length,1);
  assert.equal(f.calls.find(c=>c.command==="speak").text,"Bonjour.");
  f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  await f.tick();assert.equal(f.source.paused,true);assert.match(cls(f.root,"voice-session-state").textContent,/carte/);
  f.source.barged=2;f.source.utterance_id=2;f.source.final_text="Oui";await f.tick();
  assert.equal(f.sends.length,1,"speech must not approve an action or leak a card answer into the model");
  assert.equal(f.calls.filter(c=>c.path==="/api/stop").length,0,"speaking does not cancel the pending approval");
  f.context.conversation.messages[1].payload.status.status="denied";
  f.context.conversation.messages.push({id:"after",role:"assistant",kind:"message",state:"streaming",body:"Refus respecté."});
  await f.tick();assert.equal(f.source.paused,false);
  assert.equal(f.calls.filter(c=>c.command==="speak").at(-1).text,"Refus respecté.");
});
test("welcome voice creates a thread only after a successful utterance, never for permission refusal",async()=>{
  const f=fixture();f.setContext({...f.context,conversation:null});
  f.scenario="mic_denied";await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.calls.filter(c=>c.path==="/api/conversations").length,0);
  f.scenario="";await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.context.conversation,null);
  f.source.barged=1;f.source.utterance_id=1;f.source.final_text="Bonjour depuis l’accueil.";await f.tick();
  assert.equal(f.context.conversation.id,"created");
  assert.deepEqual(f.sends,[{text:"Bonjour depuis l’accueil.",id:"created"}]);
  assert.equal(f.panel.wantsPolling(),true);
});

test("Escape interrupts audio, switching conversations cancels capture, and late replies cannot restart it",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();
  f.source.speaking=true;f.keydown({key:"Escape",preventDefault(){},stopImmediatePropagation(){}});await new Promise(r=>setImmediate(r));
  assert.equal(f.source.speaking,false);
  f.setContext({...f.context,conversation:{id:"other",messages:[]}});
  assert.equal(f.source.status,"idle");assert.equal(f.panel.wantsPolling(),false);
  assert.equal(cls(f.root,"voice-session-bar").hidden,true);
});
test("permission denial and transcription errors stop safely and give French recovery",async()=>{
  const f=fixture();f.scenario="mic_denied";await cls(f.root,"voice-session-toggle").onclick();
  assert.match(cls(f.root,"voice-session-state").textContent,/microphone refusé/);
  assert.equal(f.source.status,"idle");assert.equal(f.panel.wantsPolling(),false);
  const settings=all(f.root).find(e=>e.textContent==="Ouvrir Réglages Système");assert.equal(settings.hidden,false);await settings.onclick();
  assert.equal(f.calls.at(-1).token,"token");assert.equal(f.calls.at(-1).reason,"mic_denied");
  f.scenario="";await cls(f.root,"voice-session-toggle").onclick();f.source.status="failed";await f.tick();
  assert.match(cls(f.root,"voice-session-state").textContent,/transcription a échoué/);assert.equal(f.sends.length,0);
});
