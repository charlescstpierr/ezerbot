import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { createVoiceReader, voiceNeedsAction } from "./voice-conversation.js";
class Element {
  constructor(tag) { this.tagName=tag; this.children=[]; this.dataset={}; this.attributes={}; this.value=""; this.scrollHeight=300; this.scrollTop=0; }
  append(...children) { for (const child of children) {if(child.parent)child.parent.children=child.parent.children.filter(c=>c!==child);child.parent=this;this.children.push(child);} }
  replaceChildren(...children) {this.children=[];this.append(...children);}
  prepend(child) {if(child.parent)child.parent.children=child.parent.children.filter(c=>c!==child);child.parent=this;this.children.unshift(child);}
  before(child) {this.parent.append(child);}
  setAttribute(k,v) {this.attributes[k]=String(v);}
}
const all = n => [n,...n.children.flatMap(all)];
const cls = (n,c) => all(n).find(x => x.className === c);
function fixture() {
  let startExtras={}, notices=[], panel, timer, keydown, source={status:"idle",transcript:"",utterance_id:0,barged:0}, scenario="", nextSend, refusal="";
  const calls=[], sends=[], follows=[], held=[], holding=new Set();
  let script, refreshFails = false;
  const limits=[];
  let dictationRefreshes=0, clock=0, bargeOnSpeak=false, refuseSpeak=false;
  const root=new Element("div"), send=new Element("button"), settings=new Element("div"), composer=new Element("div"), controls=new Element("div");controls.className="voice-controls";root.append(controls,send,settings,composer);
  const voices=[{id:"v-a",name:"Voix A",language:"fr-CA"},{id:"v-b",name:"Voix B",language:"fr-FR"},{id:"v-en",name:"Voice C",language:"en-US"}];
  const bots={ezerbot:{voice:"v-a",speed:1,language:"",chosen:false},chef:{voice:"v-b",speed:1.8,language:"fr",chosen:true}};
  let context={connected:true,bot:{id:"ezerbot",name:"Ezerbot"},conversation:{id:"chat",messages:[]},state:{voice:{bots,voices,single:false}}};
  // Like the native bridge: each queued sentence has a number; `speech_heard` is the last one read to the end,
  // `speech_cut` the first one a cut dropped (with those queued behind it).
  let queue=[], speeches=0;
  const cut=()=>{if(queue.length)source.speech_cut=queue[0];queue=[];source.speaking=false;};
  const bridge=async body => {
    calls.push(body);
    if(body.command === "options")return {voices};
    const action=script?.(body) ?? (holding.has(body.command) ? "hold" : "");
    if(action === "reject")throw new Error("refus");
    let speechId;
    if(action !== "hold-reject") {
      if(body.command === "start"){source={status:scenario || "listening",token:body.token,transcript:"",utterance_id:0,barged:0,muted:false,speech_heard:0,speech_cut:0,...startExtras};queue=[];speeches=0;}
      if(body.command === "mute" || body.command === "unmute") { source.muted=body.command === "mute"; if(source.muted)source.transcript=""; }
      if(body.command === "speak" && refuseSpeak){refuseSpeak=false;return {status:"tts_unavailable"};}
      if(body.command === "speak" && bargeOnSpeak){bargeOnSpeak=false;cut();source.barged++;}
      else if(body.command === "speak"){source.speaking=true;queue.push(speechId=++speeches);}
      if(body.command === "interrupt") { cut();source.transcript=""; }
      if(body.command === "cancel") { cut();source.status="idle";source.muted=false; }
      if(body.command === "pause")source.paused=true;
      if(body.command === "resume")source.paused=false;
    }
    const reply={...source,speaking:Boolean(source.speaking && !source.paused),...(speechId ? {speech_id:speechId} : {})};
    if(action.startsWith("hold"))await new Promise(resolve=>held.push({command:body.command,resolve}));
    if(action === "hold-reject")throw new Error("refus");
    return reply;
  };
  // The sentences at the head of the queue are read to the end.
  const heard=(count=queue.length)=>{source.speech_heard=queue.splice(0,count).at(-1) ?? source.speech_heard;source.speaking=queue.length>0;};
  // Charles speaks: the native bridge cuts what is queued at his first word.
  const talk=text=>{source.barged++;source.transcript=text;cut();};
  // A native failure cancels the session, which cuts what is queued.
  const failWith=status=>{cut();source.status=status;};
  const panels={
    register(p){ panel=p;p.composer.mount(composer,context);p.onState(context.state,context);},
    composerChanged(){}, notify(text){notices.push(text);}, async refresh(){if(refreshFails)throw new Error("Refresh interrompu");}, async api(path,body){calls.push({path,...body});if(path==="/api/voice/bot"&&refusal)throw new Error(refusal);if(path==="/api/conversations")return {conversation:{id:"created",messages:[]}};},
    openConversation(id){context={...context,conversation:{id,messages:[]}};panel.onState(context.state,context);return true;},
    async ensureDraftConversation(){
      const {conversation}=await this.api("/api/conversations",{bot_id:context.bot.id});
      this.transfer(conversation);
      return conversation;
    },
    transfer(conversation){
      const next={...context,conversation};
      panel.composer.transfer?.({from:context.conversation?.id || `new:${context.bot.id}`,to:conversation.id},next);
      context=next;panel.onState(context.state,context);
    },
    async sendVoice(text,id){sends.push({text,id});nextSend?.();},
    followVoiceReply(id){follows.push(id);},
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice-conversation.js",import.meta.url),"utf8"),{
    window:{GrokbotPanels:panels,EzerbotMicrophone:{current:()=>context.state.voice.microphone||"",fellBack(id){notices.push("repli:"+id);}},EzerbotDictation:{refresh(){dictationRefreshes++;},cancel(){} },GrokbotChat:{createVoiceReader,voiceNeedsAction},webkit:{messageHandlers:{ezerbotVoiceSession:{postMessage:bridge}}},addEventListener(){}},
    document:{createElement:tag=>new Element(tag),getElementById:()=>send,querySelector:()=>controls,addEventListener(name,fn){if(name==="keydown")keydown=fn;}},
    Date:{now:()=>clock},crypto:{randomUUID:()=>"token"},setTimeout(fn,ms){if(ms===100){timer=fn;return 1;}limits.push(fn);return limits.length+1;},clearTimeout(id){if(id>1)limits[id-2]=null;else timer=null;},
  });
  const section=bot=>{const container=new Element("div");const cleanup=panel.botSection(bot,container,context);return {container,cleanup};};
  return {section,heard,talk,failWith,set clock(ms){clock=ms;},set bargeOnSpeak(value){bargeOnSpeak=value;},set refuseSpeak(value){refuseSpeak=value;},hold(...commands){holding.clear();for(const command of commands)holding.add(command);},set script(fn){script=fn;},release(...commands){if(!commands.length)holding.clear();for(const late of held.splice(0))if(!commands.length||commands.includes(late.command))late.resolve();else held.push(late);},set refusal(text){refusal=text;},root,calls,sends,follows,notices,panel,set startExtras(value){startExtras=value;},setMeetingActive(value){panels.meetingActive=()=>value;panel.onState(context.state,context);},get dictationRefreshes(){return dictationRefreshes;},get source(){return source;},set scenario(s){scenario=s;},set nextSend(fn){nextSend=fn;},get context(){return context;},
    transfer:conversation=>panels.transfer(conversation),set refreshFails(value){refreshFails=value;},setContext(c){context=c;panel.onState(c.state,c);},tick:async()=>timer?.(),keydown:e=>keydown(e),expire(){limits.forEach((fn,i)=>{limits[i]=null;fn?.();});}};
}
test("R32 voice session follows an automatic transfer while listening on the welcome draft",async()=>{
  const f=fixture();f.setContext({...f.context,conversation:null});
  await cls(f.root,"voice-session-toggle").onclick();
  f.transfer({id:"text-send",messages:[]});
  f.source.final_text="Une phrase ajoutée après la création.";f.source.utterance_id=1;
  await f.tick();
  assert.deepEqual(f.sends,[{text:"Une phrase ajoutée après la création.",id:"text-send"}]);
  assert.equal(f.calls.filter(c=>c.command==="start").length,1);
});
test("R32 welcome voice sends using the created conversation even when refresh is unavailable",async()=>{
  const f=fixture();f.setContext({...f.context,conversation:null});f.refreshFails=true;
  await cls(f.root,"voice-session-toggle").onclick();
  f.source.final_text="Phrase conservée.";f.source.utterance_id=1;
  await f.tick();
  assert.deepEqual(f.sends,[{text:"Phrase conservée.",id:"created"}]);
});
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
  assert.equal(all(f.root).find(e=>e.textContent==="Interrompre").hidden,true,"no interrupt action while a card waits");
  f.source.barged=2;f.source.utterance_id=2;f.source.transcript="Oui";f.source.final_text="Oui";await f.tick();
  await f.tick();await f.tick();
  assert.equal(cls(f.root,"voice-live-transcript").textContent,"Oui");
  assert.equal(cls(f.root,"voice-held-notice").hidden,false,"held-input warning survives repeated polling");
  f.source.transcript="Oui vas-y";await f.tick();
  assert.equal(cls(f.root,"voice-held-notice").hidden,false,"new partial speech cannot remove the warning");
  assert.equal(f.sends.length,1,"speech must not approve an action or leak a card answer into the model");
  assert.equal(f.calls.filter(c=>c.path==="/api/stop").length,0,"speaking does not cancel the pending approval");
  f.context.conversation.messages[1].payload.status.status="denied";
  f.context.conversation.messages.push({id:"after",role:"assistant",kind:"message",state:"streaming",body:"Refus respecté."});
  await f.tick();assert.equal(f.source.paused,false);
  assert.equal(f.calls.filter(c=>c.command==="speak").at(-1).text,"Refus respecté.");
  assert.equal(cls(f.root,"voice-held-notice").hidden,true);
  assert.deepEqual(f.follows,["chat"],"only the resumed spoken reply requests scroll-follow");
  await f.tick();assert.deepEqual(f.follows,["chat"],"polling cannot repeatedly pull the reader to the bottom");
});
test("voice controls share one toolbar group; listening has no false interrupt and live text follows its tail",async()=>{
  const f=fixture();const button=cls(f.root,"voice-session-toggle");
  assert.equal(button.parent.className,"voice-controls");
  assert.equal(button.attributes["aria-label"],"Mode voix");
  assert.equal(cls(f.root,"voice-session-icon").attributes["aria-hidden"],"true");
  const before=f.dictationRefreshes;await button.onclick();
  assert.ok(f.dictationRefreshes>before,"dictation refreshes immediately without waiting for /api/state");
  assert.equal(all(f.root).find(e=>e.textContent==="Interrompre").hidden,true);
  f.source.transcript="Une très longue phrase. ".repeat(80)+"Voici les derniers mots.";await f.tick();
  const transcript=cls(f.root,"voice-live-transcript");
  assert.ok(transcript.textContent.endsWith("Voici les derniers mots."));
  assert.equal(transcript.scrollTop,transcript.scrollHeight);
  await button.onclick();assert.equal(f.panel.wantsPolling(),false);
});

const muteButton=f=>cls(f.root,"icon-button voice-mute-toggle"), stateOf=f=>cls(f.root,"voice-session-state").textContent;
const shortcut=(f,extra={})=>{const event={key:"M",metaKey:true,shiftKey:true,prevented:false,preventDefault(){this.prevented=true;},stopImmediatePropagation(){},...extra};f.keydown(event);return new Promise(r=>setImmediate(()=>r(event)));};
test("the mute button sends mute, says the bot cannot hear, and empties the live transcript; a second click listens again",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();const mute=muteButton(f);
  assert.equal(mute.attributes["aria-label"],"Couper le micro");assert.equal(mute.attributes["aria-pressed"],"false");
  f.source.transcript="Bonjour";await f.tick();assert.equal(cls(f.root,"voice-live-transcript").textContent,"Bonjour");
  await mute.onclick();
  assert.equal(f.calls.at(-1).command,"mute");assert.equal(f.calls.at(-1).token,"token");
  assert.equal(stateOf(f),"Micro coupé · le bot ne vous entend pas");
  assert.equal([mute.attributes["aria-pressed"],mute.attributes["aria-label"]].join(),"true,Réactiver le micro");
  assert.equal(cls(f.root,"voice-live-transcript").hidden,true);assert.doesNotMatch(cls(f.root,"voice-live-transcript").textContent,/Bonjour/);
  await f.tick();assert.equal(stateOf(f),"Micro coupé · le bot ne vous entend pas","the screen follows the bridge status, tick after tick");
  await mute.onclick();
  assert.equal(f.calls.at(-1).command,"unmute");assert.match(stateOf(f),/^À l’écoute/);
  assert.equal(mute.attributes["aria-pressed"],"false");assert.equal(cls(f.root,"voice-live-transcript").hidden,false);
});
test("the screen follows the bridge, not the click: a bridge that stays unmuted keeps the button released",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();
  Object.defineProperty(f.source,"muted",{get:()=>false,set(){}});
  await muteButton(f).onclick();
  assert.equal(muteButton(f).attributes["aria-pressed"],"false");assert.match(stateOf(f),/^À l’écoute/);
});
test("⌘⇧M toggles the microphone during a session and does nothing without one",async()=>{
  const f=fixture();let event=await shortcut(f);
  assert.equal(event.prevented,false);assert.equal(f.calls.filter(c=>c.command==="mute").length,0,"no session, no command");
  await cls(f.root,"voice-session-toggle").onclick();
  event=await shortcut(f);assert.equal(event.prevented,true);assert.equal(f.source.muted,true);assert.equal(muteButton(f).attributes["aria-pressed"],"true");
  await shortcut(f);assert.equal(f.source.muted,false);assert.equal(muteButton(f).attributes["aria-pressed"],"false");
  for(const extra of [{shiftKey:false},{metaKey:false},{altKey:true},{ctrlKey:true},{key:"N"}])await shortcut(f,extra);
  assert.equal(f.calls.filter(c=>c.command==="mute"||c.command==="unmute").length,2,"only ⌘⇧M");
});
test("leaving the voice mode and starting it again listens: the mute never sticks",async()=>{
  const f=fixture();const toggle=cls(f.root,"voice-session-toggle");await toggle.onclick();await muteButton(f).onclick();assert.equal(f.source.muted,true);
  await toggle.onclick();assert.equal(f.source.status,"idle");
  await toggle.onclick();
  assert.equal(f.source.muted,false);assert.equal(muteButton(f).attributes["aria-pressed"],"false");assert.match(stateOf(f),/^À l’écoute/);
  await f.tick();assert.match(stateOf(f),/^À l’écoute/);
});

test("a microphone gone while muted is announced when the microphone is turned back on",async()=>{
  const f=fixture();f.context.state.voice.microphone="usb";await cls(f.root,"voice-session-toggle").onclick();f.source.microphone_fallback=true;
  await muteButton(f).onclick();assert.deepEqual(f.notices,[],"muted, nothing is listened to");
  await muteButton(f).onclick();assert.deepEqual(f.notices,["repli:usb"]);
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
test("composer voice control stays an icon with a hidden label at every width",()=>{
  const css=readFileSync(new URL("../../ui/panels/voice-conversation.css",import.meta.url),"utf8");
  assert.match(css,/\.voice-session-toggle \{[^}]*width: 32px; height: 32px;/);
  assert.match(css,/\.voice-session-icon \{[^}]*display:\s*flex/);
  assert.match(css,/\.voice-session-label \{[^}]*overflow:\s*hidden/);
  assert.doesNotMatch(css,/@media \(max-width: 900px\)[\s\S]*\.voice-session-label \{ display: none;/);
  assert.match(css,/background: var\(--accent-ink\)/);
  assert.match(css,/@media \(prefers-reduced-motion: reduce\).*animation: none !important/);
});

test("meeting ownership disables conversation voice without cancelling the meeting",async()=>{
  const f=fixture();const toggle=cls(f.root,"voice-session-toggle");f.setMeetingActive(true);
  assert.equal(toggle.disabled,true);assert.match(toggle.title,/réunion/);await toggle.onclick();
  assert.equal(f.calls.filter(c=>c.command==="start").length,0);
  f.setMeetingActive(false);assert.equal(toggle.disabled,false);await toggle.onclick();assert.equal(f.panel.wantsPolling(),true);
});

test("permission denial and transcription errors stop safely and give French recovery",async()=>{
  const f=fixture();f.scenario="mic_denied";await cls(f.root,"voice-session-toggle").onclick();
  assert.match(cls(f.root,"voice-session-state").textContent,/microphone refusé/);
  assert.equal(cls(f.root,"voice-live-transcript").hidden,true,"no listening placeholder under an error");
  assert.equal(f.source.status,"idle");assert.equal(f.panel.wantsPolling(),false);
  const settings=all(f.root).find(e=>e.textContent==="Ouvrir Réglages Système");assert.equal(settings.hidden,false);await settings.onclick();
  assert.equal(f.calls.at(-1).token,"token");assert.equal(f.calls.at(-1).reason,"mic_denied");
  f.scenario="";await cls(f.root,"voice-session-toggle").onclick();f.source.status="failed";await f.tick();
  assert.match(cls(f.root,"voice-session-state").textContent,/transcription a échoué/);assert.equal(f.sends.length,0);
});

test("the voice mode listens to the chosen microphone and never sends it as a voice setting",async()=>{
  const f=fixture(); f.context.state.voice.microphone="usb";
  await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.calls.find(c=>c.command==="start").microphone,"usb");
  f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bonjour. Je vous écoute"});
  await f.tick();
  const speak=f.calls.find(c=>c.command==="speak");
  assert.deepEqual(Object.keys(speak).filter(k=>!["command","token","text","barge","voice","speed","language"].includes(k)),[],"the microphone is not a voice setting");
});
test("a microphone gone when the voice mode starts is announced and the mode still runs",async()=>{
  const f=fixture(); f.context.state.voice.microphone="usb"; f.startExtras={microphone_fallback:true};
  await cls(f.root,"voice-session-toggle").onclick();
  assert.deepEqual(f.notices,["repli:usb"],"only the missing choice is forgotten");
  assert.equal(cls(f.root,"voice-session-toggle").attributes["aria-pressed"],"true");
});

const speaks=f=>f.calls.filter(c=>c.command==="speak").map(({voice,speed,language})=>({voice,speed,language}));
const reply=(f,id,body)=>f.context.conversation.messages.push({id,role:"assistant",kind:"message",state:"streaming",body});
test("each bot's voice mode speaks with the voice, speed and language of its own bot",async()=>{
  const first=fixture(); await cls(first.root,"voice-session-toggle").onclick();
  reply(first,"r1","Bonjour. Je vous écoute"); await first.tick();
  assert.deepEqual(speaks(first),[{voice:"v-a",speed:1,language:""}]);
  const second=fixture(); second.setContext({...second.context,bot:{id:"chef",name:"Chef"}});
  await cls(second.root,"voice-session-toggle").onclick();
  reply(second,"r2","Bonjour. Je vous écoute"); await second.tick();
  assert.deepEqual(speaks(second),[{voice:"v-b",speed:1.8,language:"fr"}]);
});
test("the bot setting shows the voice of its bot, offers speeds up to 2 and a language or Auto",async()=>{
  const f=fixture();
  const {container}=f.section({id:"chef",name:"Chef"});
  const nodes=all(container);
  assert.match(nodes.find(n=>n.className==="voice-bot-current").textContent,/Voix de Chef : Voix B · 1,8× · français/);
  const speed=nodes.find(n=>n.name==="bot-voice-speed");
  assert.deepEqual([speed.min,speed.max,speed.value],["0.5","2",1.8]);
  const language=nodes.find(n=>n.name==="bot-voice-language");
  assert.deepEqual(language.children.map(o=>o.value),["","fr","en"]);
  assert.equal(language.children[0].textContent,"Auto");
  const other=all(f.section({id:"ezerbot",name:"Ezerbot"}).container);
  assert.match(other.find(n=>n.className==="voice-bot-current").textContent,/Voix de Ezerbot : Voix A · 1× · langue Auto/);
  assert.match(other.find(n=>n.className==="voice-bot-hint").textContent,/attribuée automatiquement/);
});
test("a single voice is announced as shared by every bot",()=>{
  const f=fixture(); f.context.state.voice.voices=[f.context.state.voice.voices[0]]; f.context.state.voice.single=true;
  const hint=all(f.section({id:"ezerbot",name:"Ezerbot"}).container).find(n=>n.className==="voice-bot-hint");
  assert.equal(hint.textContent,"Une seule voix est installée sur ce Mac : tous les bots la partagent.");
});
test("saving sends the bot's choice, and a refused speed is said without replacing the saved one",async()=>{
  const f=fixture(); const nodes=all(f.section({id:"chef",name:"Chef"}).container);
  const speed=nodes.find(n=>n.name==="bot-voice-speed"), save=nodes.find(n=>n.textContent==="Enregistrer la voix"), notice=nodes.find(n=>n.className==="voice-settings-notice");
  speed.value="2"; speed.oninput(); await save.onclick();
  assert.deepEqual(f.calls.find(c=>c.path==="/api/voice/bot"),{path:"/api/voice/bot",bot_id:"chef",voice:"v-b",speed:2,language:"fr"});
  assert.match(notice.textContent,/Voix enregistrée/);
  const refusal="Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse du bot reste inchangée.";
  f.refusal=refusal;
  speed.value="2.5"; speed.oninput(); await save.onclick();
  assert.equal(notice.textContent,refusal);
  assert.equal(nodes.find(n=>n.className==="voice-bot-current").textContent.includes("1,8×"),true,"the saved speed is still the one displayed");
});
test("the voices the Mac offers are handed to the engine once, so that bots without a choice get distinct ones",async()=>{
  const f=fixture();
  await new Promise(r=>setImmediate(r));
  assert.deepEqual(f.calls.find(c=>c.path==="/api/voice/voices").voices.map(v=>v.id),["v-a","v-b","v-en"]);
  f.setContext(f.context); await new Promise(r=>setImmediate(r));
  assert.equal(f.calls.filter(c=>c.path==="/api/voice/voices").length,1);
});
test("in a group, each reply is spoken with the voice of the bot that wrote it",async()=>{
  const f=fixture(); await cls(f.root,"voice-session-toggle").onclick();
  f.context.conversation.kind="group";
  f.context.conversation.messages.push(
    {id:"a",role:"assistant",kind:"reply",state:"streaming",body:"Je commence.",payload:{bot_id:"ezerbot"}},
    {id:"b",role:"assistant",kind:"reply",state:"streaming",body:"Je continue.",payload:{bot_id:"chef"}},
  );
  await f.tick();
  assert.deepEqual(f.calls.filter(c=>c.command==="speak").map(({text,voice,speed,language})=>({text,voice,speed,language})),[
    {text:"Je commence.",voice:"v-a",speed:1,language:""},
    {text:"Je continue.",voice:"v-b",speed:1.8,language:"fr"},
  ]);
});

test("an older bridge answer cannot undo a newer unmute: the screen never says muted while the bridge listens",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();await muteButton(f).onclick();
  assert.equal(muteButton(f).attributes["aria-pressed"],"true");
  f.hold("status");const late=f.tick();f.hold();
  await muteButton(f).onclick();assert.equal(f.source.muted,false);assert.equal(muteButton(f).attributes["aria-pressed"],"false");
  f.release();await late;
  assert.equal(muteButton(f).attributes["aria-pressed"],"false","the status read while muted is older than the unmute");
  assert.match(stateOf(f),/^À l’écoute/);
});
test("a gesture while a mute command is in flight is ignored: one command, and the screen follows its answer",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();
  f.hold("mute","unmute");const click=muteButton(f).onclick();const key=shortcut(f);
  f.release();await click;await key;
  assert.deepEqual(f.calls.map(c=>c.command).filter(c=>c==="mute"||c==="unmute"),["mute"]);
  assert.equal(f.source.muted,true);assert.equal(muteButton(f).attributes["aria-pressed"],"true");assert.equal(stateOf(f),"Micro coupé · le bot ne vous entend pas");
});
test("a card waiting for a click and a muted microphone are both said",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();
  f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  await f.tick();assert.equal(stateOf(f),"En attente de vous · répondez dans la carte");
  await muteButton(f).onclick();
  assert.match(stateOf(f),/carte/);assert.match(stateOf(f),/micro coupé/i);assert.equal(muteButton(f).attributes["aria-pressed"],"true");
  await f.tick();assert.match(stateOf(f),/carte/);assert.match(stateOf(f),/micro coupé/i);
  await muteButton(f).onclick();assert.equal(stateOf(f),"En attente de vous · répondez dans la carte");
});

const settle=()=>new Promise(r=>setImmediate(r));
function* orders(items){if(items.length<2){yield items;return;}for(const [i,x] of items.entries())for(const rest of orders(items.filter((_,j)=>j!==i)))yield [x,...rest];}
test("whatever the outcome, lateness and order of the bridge answers, the screen ends on the bridge's muted and the next gesture sends the right command",async()=>{
  let runs=0;
  for(const startMuted of [false,true])for(const mode of ["","hold","reject","hold-reject"])for(const tick of ["","before","during"])for(const gesture of ["","click","key"])for(const second of ["","reject"])for(const order of orders([...tick?["status"]:[],...mode.startsWith("hold")?["command"]:[]])){
    const f=fixture(),name=JSON.stringify({startMuted,mode,tick,gesture,second,order});
    await cls(f.root,"voice-session-toggle").onclick();if(startMuted)await muteButton(f).onclick();
    let sent=0,reads=0;
    f.script=body=>body.command==="status"?(++reads===1&&tick?"hold":""):body.command==="mute"||body.command==="unmute"?(++sent===1?mode:second):"";
    const pending=[];
    if(tick==="before")pending.push(f.tick());
    pending.push(muteButton(f).onclick());
    if(gesture==="click")pending.push(muteButton(f).onclick());
    if(gesture==="key")pending.push(shortcut(f));
    if(tick==="during")pending.push(f.tick());
    await settle();
    for(const late of order){f.release(...late==="status"?["status"]:["mute","unmute"]);await settle();}
    f.release();await Promise.all(pending);await settle();
    const muted=f.source.muted;
    assert.equal(muteButton(f).attributes["aria-pressed"],String(muted),name);
    assert.equal(stateOf(f),muted?"Micro coupé · le bot ne vous entend pas":"À l’écoute · parlez pour envoyer",name);
    f.script=undefined;const before=f.calls.length;await muteButton(f).onclick();
    assert.equal(f.calls[before].command,muted?"unmute":"mute",name);
    assert.equal(f.source.muted,!muted,name);assert.equal(muteButton(f).attributes["aria-pressed"],String(!muted),name);
    runs++;
  }
  assert.ok(runs>150,String(runs));
});
test("a mute answered after the session ended changes nothing in the next session",async()=>{
  for(const mode of ["hold","hold-reject"]){
    const f=fixture(),toggle=cls(f.root,"voice-session-toggle");await toggle.onclick();
    f.script=body=>body.command==="mute"?mode:"";const click=muteButton(f).onclick();
    await toggle.onclick();await toggle.onclick();
    f.release();await click;await settle();
    assert.equal(muteButton(f).attributes["aria-pressed"],"false",mode);assert.match(stateOf(f),/^À l’écoute/);
    f.script=undefined;await muteButton(f).onclick();assert.equal(f.calls.at(-1).command,"mute",mode);assert.equal(f.source.muted,true);
  }
});
test("a status that carries the microphone fallback announces it, once, for the microphone asked at the start",async()=>{
  const f=fixture();f.context.state.voice.microphone="usb-A";await cls(f.root,"voice-session-toggle").onclick();
  await muteButton(f).onclick();f.source.microphone_fallback=true;
  f.context.state.voice.microphone="usb-B";
  f.hold("unmute");const unmute=muteButton(f).onclick();await f.tick();f.release();await unmute;await f.tick();
  assert.deepEqual(f.notices,["repli:usb-A"],"the choice saved meanwhile is not the one that was missing");
});
test("a command whose answer never comes ends at its limit: the screen follows the bridge again and the gestures come back",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();await muteButton(f).onclick();
  f.hold("unmute");const lost=muteButton(f).onclick();await settle();
  assert.equal(f.source.muted,false,"the bridge did unmute; its answer is withheld");
  for(let i=0;i<3;i++)await f.tick();
  assert.equal(muteButton(f).attributes["aria-pressed"],"true","inside the limit the command is still waited for");
  f.expire();
  for(let i=0;i<10;i++)await f.tick();
  assert.equal(muteButton(f).attributes["aria-pressed"],"false");assert.match(stateOf(f),/^À l’écoute/);
  await muteButton(f).onclick();assert.equal(f.calls.at(-1).command,"mute");assert.equal(f.source.muted,true);
  f.release();await lost;await settle();
  assert.equal(muteButton(f).attributes["aria-pressed"],"true","the abandoned answer says nothing any more");
});
test("the microphone fallback is announced even when the answer that carries it is an error",async()=>{
  for(const carrier of ["command","status"]){
    const f=fixture();f.context.state.voice.microphone="usb-A";await cls(f.root,"voice-session-toggle").onclick();await muteButton(f).onclick();
    f.source.microphone_fallback=true;f.source.status="mic_denied";
    if(carrier==="command")await muteButton(f).onclick();else{f.source.muted=false;await f.tick();}
    assert.deepEqual(f.notices,["repli:usb-A"],carrier);
  }
});

// A sub-task launched from the thread, as the engine publishes it in state.delegation.
const task=(id,status,over={})=>({id,objective:"Résumer le rapport",parent_conversation_id:"chat",status:{status},...over});
const tasksOf=(f,...edges)=>{f.context.conversation.messages.push(...edges.map(payload=>({id:"card-"+payload.id,kind:"delegation",role:"assistant",payload})));return edges;};
const spoken=f=>f.calls.filter(c=>c.command==="speak").map(c=>c.text);
const phrase="La tâche « Résumer le rapport » est terminée.";

test("a task of this thread that finishes during the voice mode is announced once, with its title, and the bar shows it for 5 s",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[],"a running task is not announced");
  edge.status={status:"done",result:"Fait."};f.clock=1000;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
  f.source.speaking=false;f.clock=2000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[phrase],"two successive polls do not repeat the announcement");
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
  f.clock=6001;await f.tick();
  assert.equal(cls(f.root,"voice-session-state").textContent,"À l’écoute · parlez pour envoyer");
});

test("a failed or refused task is announced as failed, a cancelled one is not",async()=>{
  const f=fixture();const edges=tasksOf(f,task("a","running",{objective:"Trier"}),task("b","running",{objective:"Archiver"}),task("c","running",{objective:"Ranger"}));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edges[0].status={status:"failed",reason:"x"};edges[1].status={status:"refused",reason:"y"};edges[2].status={status:"cancelled"};
  for(let i=0;i<3;i++){await f.tick();f.heard();}
  assert.deepEqual(spoken(f),["La tâche « Archiver » a échoué.","La tâche « Trier » a échoué."]);
});

test("the announcement waits while Charles speaks and is said once he stops",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.transcript="Je voudrais savoir";f.source.barged=1;
  await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
  f.source.transcript="";await f.tick();
  assert.deepEqual(spoken(f),[],"his words just stopped: a short silence first");
  f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
});

test("the announcement waits for the end of the sentence being spoken, even while the reply is still being written",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.speaking=true;
  f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bien sûr. Je regarde"});
  await f.tick();assert.deepEqual(spoken(f),["Bien sûr."],"the sentence of the reply is read first");
  f.source.speaking=true;await f.tick();assert.deepEqual(spoken(f),["Bien sûr."]);
  f.source.speaking=false;await f.tick();
  assert.deepEqual(spoken(f),["Bien sûr.",phrase],"a task ended during a long reply is not held until the reply ends");
});

test("a task of another thread is never announced",async()=>{
  const f=fixture();
  f.context.state.delegation={delegations:[task("t1","running",{parent_conversation_id:"autre"})]};
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  f.context.state.delegation.delegations[0].status={status:"done",result:"Fait."};await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
});

test("a task finished before the voice mode opened is not announced",async()=>{
  const f=fixture();tasksOf(f,task("old","done"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
});

test("muted with a card waiting, nothing is announced until the card is answered",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  f.source.muted=true;f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  edge.status={status:"done",result:"Fait."};await f.tick();await f.tick();
  assert.equal(f.source.paused,true);assert.deepEqual(spoken(f),[]);
  f.context.conversation.messages.find(m=>m.id==="approval").payload.status.status="approved";await f.tick();
  assert.equal(f.source.paused,false);assert.deepEqual(spoken(f),[phrase]);
});

test("an announcement that Charles cuts off after the bridge took it is said again once he stops",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  f.talk("Attends");await f.tick();
  assert.deepEqual(spoken(f),[phrase],"nothing while Charles speaks");
  f.source.transcript="";f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
  f.heard();await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"heard to the end, it is not said a third time");
});

test("an announcement that Charles cut by speaking is said again, and shown only once said",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.bargeOnSpeak=true;await f.tick();
  assert.notEqual(cls(f.root,"voice-session-state").textContent,phrase);
  await f.tick();f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
});

test("an announcement waits for the silence after Charles starts to speak, even before his words are transcribed",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.barged=1;
  f.clock=1000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[],"barged moved, speaking is false and nothing is transcribed yet");
  f.clock=2400;await f.tick();
  assert.deepEqual(spoken(f),[]);
  f.clock=2600;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
});

test("an announcement the bridge refused is tried again when the voice mode is relaunched",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.refuseSpeak=true;await f.tick();
  assert.equal(f.panel.wantsPolling(),false,"a refused voice stops the session");
  assert.deepEqual(spoken(f),[phrase]);
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"never said, so said again");
  f.heard();await f.tick();
  await cls(f.root,"voice-session-toggle").onclick();await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"said, so not said a third time after a relaunch");
});

test("two announcements in a row are each shown for 5 s, one after the other",async()=>{
  const f=fixture();const edges=tasksOf(f,task("a","running",{objective:"Trier"}),task("b","running",{objective:"Archiver"}));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  const state=()=>cls(f.root,"voice-session-state").textContent;
  edges[0].status={status:"done"};f.clock=1000;await f.tick();
  edges[1].status={status:"done"};f.heard();f.clock=3000;await f.tick();
  assert.deepEqual(spoken(f),["La tâche « Trier » est terminée.","La tâche « Archiver » est terminée."]);
  f.heard();
  f.clock=5500;await f.tick();assert.equal(state(),"La tâche « Trier » est terminée.","the first is not erased by the second");
  f.clock=7000;await f.tick();assert.equal(state(),"La tâche « Archiver » est terminée.");
  f.clock=11001;await f.tick();assert.equal(state(),"À l’écoute · parlez pour envoyer");
});

test("an announcement paused by a card and then cut by Charles is said again once the card is answered",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  await f.tick();await f.tick();
  assert.equal(f.source.paused,true);
  f.talk("Attends");await f.tick();
  f.source.transcript="";f.context.conversation.messages.find(m=>m.id==="approval").payload.status.status="approved";
  f.clock=2000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"suspended is not heard: its cut puts it back");
});

test("an announcement lost by a synthesis failure after the bridge took it is said at the relaunch",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  f.failWith("tts_unavailable");await f.tick();
  assert.equal(f.panel.wantsPolling(),false);
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
});

test("an announcement heard to the end is not said again when Charles cuts the bot's next sentence",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  reply(f,"r","Voici la suite. Et encore");await f.tick();
  assert.deepEqual(spoken(f),[phrase,"Voici la suite."]);
  f.heard(1);await f.tick();
  assert.equal(f.source.speaking,true,"the bot's sentence is still being read");
  f.talk("Stop");await f.tick();
  f.source.transcript="";f.clock=2000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f).filter(text=>text===phrase),[phrase]);
});

test("an announcement whose speak is still unanswered when Charles leaves is said at the relaunch",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.hold("speak");
  const pending=f.tick();await new Promise(r=>setImmediate(r));
  assert.deepEqual(spoken(f),[phrase]);
  all(f.root).find(e=>e.textContent==="Quitter").onclick();
  f.release();await pending;
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
});

test("a synthesis failure reported with the cut of the announcement's own sentence still owes it to the relaunch",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  Object.assign(f.source,{speech_cut:1,speech_heard:0,status:"tts_unavailable",speaking:false});await f.tick();
  assert.equal(f.panel.wantsPolling(),false);
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
});

test("an announcement cut by Charles and left before it is said again is said at the relaunch",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  f.talk("Attends");await f.tick();
  assert.deepEqual(spoken(f),[phrase],"the cut is seen, Charles is still speaking");
  all(f.root).find(e=>e.textContent==="Quitter").onclick();
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
});

test("muting during the 5 s of an announcement shows that the microphone is off",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.clock=1000;await f.tick();
  assert.equal(stateOf(f),phrase);
  await muteButton(f).onclick();
  assert.equal(stateOf(f),"Micro coupé · le bot ne vous entend pas");
});
