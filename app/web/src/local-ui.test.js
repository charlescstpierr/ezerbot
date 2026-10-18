import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { localCallView } from "./local-call.js";
import { revealPendingApproval } from "./approval-follow.js";

class Element {
  constructor(tag) { this.tagName=tag;this.children=[];this.listeners={}; }
  append(...children) {this.children.push(...children);}
  replaceChildren(...children) {this.children=children;}
  setAttribute(name,value) {this[name]=value;}
  addEventListener(name,fn) {this.listeners[name]=fn;}
}
const all = e => [e,...e.children.flatMap(all)];
function setup({scroll,ResizeObserver,dialog,clock=Date}={}) {
  let registration;
  const requests=[], notifications=[];
  const state={bots:[{id:"bot",name:"Mon bot"}],local:{bots:[],folders:["/tmp/<dossier>"]}};
  const panels={state,register(p){registration=p;},async api(path,body){requests.push({path,body});},async refresh(){},notify(message){notifications.push(message);}};
  const window={GrokbotPanels:panels,GrokbotChat:{revealPendingApproval},ResizeObserver};
  const document=Object.assign(new Element("document"),{body:new Element("body"),getElementById:id=>id==="chat-scroll"?scroll:id==="settings-dialog"?dialog:undefined,createElement:tag=>new Element(tag),createTextNode:text=>Object.assign(new Element("#text"),{textContent:text})});
  document.activeElement=document.body;
  runInNewContext(readFileSync(new URL("../../ui/panels/local.js",import.meta.url),"utf8"),{window,document,Date:clock});
  const root=new Element("div");registration.settingsSection(root);
  return {root,requests,registration,panels,notifications,window,document};
}
test("malformed model arguments stay printable and never become React object children",()=>{
  for(const arguments_ of [null,[],"invalid",{command:{invalide:true},cwd:{invalide:true}},{path:{invalide:true}}]) {
    const view=localCallView({name:"run_shell",arguments:arguments_});
    assert.equal(typeof view.preview,"string");assert.equal(view.shellArguments,false);
  }
  const view=localCallView({name:"run_shell",arguments:{command:"printf '<script>'",cwd:"/tmp"}});
  assert.equal(view.preview,"printf '<script>'");assert.equal(view.shellArguments,true);
});

test("local tools default off and rapid independent toggles send atomic patches",async()=>{
  const {root,requests}=setup();
  const inputs=all(root).filter(e=>e.type==="checkbox");
  assert.equal(inputs.length,2);assert.equal(inputs[0].checked,false);assert.equal(inputs[1].checked,false);
  assert.ok(inputs.every(e=>e.role==="switch"));
  inputs[0].checked=true;inputs[1].checked=true;
  await Promise.all(inputs.map(e=>e.listeners.change()));
  assert.deepEqual(JSON.parse(JSON.stringify(requests)),[
    {path:"/api/local/bot",body:{bot_id:"bot",shell:true}},
    {path:"/api/local/bot",body:{bot_id:"bot",files:true}},
  ]);
});
test("settings errors remain visible inside the modal and preserve the folder draft",async()=>{
  const {root,panels,notifications}=setup();
  panels.api=async()=>{throw new Error("Un chemin absolu sans « .. » est requis.");};
  const form=all(root).find(e=>e.className==="local-folder-form");
  form.children[0].value="relatif/dossier";
  await form.listeners.submit({preventDefault(){}});
  const error=all(root).find(e=>e.className==="local-folder-error");
  assert.equal(error.hidden,false);assert.equal(error.role,"alert");assert.match(error.textContent,/chemin absolu/);
  const input=all(root).find(e=>e.name==="local-folder");assert.equal(input.value,"relatif/dossier");
  input.listeners.input();assert.equal(error.hidden,true);assert.deepEqual(notifications,[]);
});

test("new uncertainty is brought into view after shell anchoring, without revisiting old failures",()=>{
  const {registration,panels,window,document}=setup();const frames=[];let moves=0;
  window.requestAnimationFrame=fn=>frames.push(fn);
  document.querySelector=()=>null;
  document.querySelectorAll=()=>[{dataset:{messageId:"critical",localState:"failed"},scrollIntoView(){moves+=1;}}];
  const failed={id:"critical",kind:"tool",payload:{call:{name:"run_shell"},outcome:{status:"failed",reason:"Résultat inconnu"}}};
  const current={conversation:{id:"chat",messages:[{role:"user"},failed]}};
  registration.onState(panels.state,current);assert.equal(frames.length,1);frames.shift()();assert.equal(moves,0);frames.shift()();assert.equal(moves,1);
  registration.onState(panels.state,current);assert.equal(frames.length,0);
  registration.onState(panels.state,{conversation:{id:"chat",messages:[{role:"user"},{...failed,id:"older"},{role:"user"}]}});assert.equal(frames.length,0);
  registration.onState(panels.state,{conversation:{id:"chat",messages:[{role:"user"},{...failed,id:"older-2"},{id:"success",kind:"tool",payload:{call:{name:"run_shell"},outcome:{status:"done",output:{status:"terminé"}}}}]}});assert.equal(frames.length,0);
});

test("attention waits for the result commit, abandons stale turns, and bounds missing-DOM retries",()=>{
  const {registration,panels,window,document}=setup();const frames=[];let moves=0;
  const target={dataset:{messageId:"late",localState:""},scrollIntoView(){moves+=1;}};
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;document.querySelectorAll=()=>[target];
  const failed={id:"late",kind:"tool",payload:{call:{name:"read_file"},outcome:{status:"failed",reason:"Absent"}}};
  const context={conversation:{id:"chat",messages:[{role:"user"},failed]}};
  const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
  registration.onState(panels.state,context);frames.shift()();frames.shift()();assert.equal(moves,0);assert.equal(frames.length,1);
  target.dataset.localState="failed";flush();assert.equal(moves,1);
  failed.id="stale";target.dataset.messageId="stale";registration.onState(panels.state,context);
  registration.onState(panels.state,{conversation:{id:"chat",messages:[...context.conversation.messages,{role:"user"}]}});flush();assert.equal(moves,1);
  document.querySelectorAll=()=>[];registration.onState(panels.state,context);flush();assert.equal(moves,1);
});

test("pending decisions defer to open dialogs and never scroll a newly selected conversation",()=>{
  const {registration,panels,window,document}=setup();const frames=[];let modal=true,moves=0;
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>modal?{}:null;
  document.querySelectorAll=()=>[{dataset:{messageId:"decision"},scrollIntoView(){moves+=1;}}];
  const pending={id:"decision",kind:"approval",payload:{status:{status:"pending"},kind:{tool:"write_file"}}};
  const context={conversation:{id:"chat",messages:[{role:"user"},pending]}};
  const flush=()=>{while(frames.length)frames.shift()();};
  registration.onState(panels.state,context);flush();assert.equal(moves,0);
  modal=false;registration.onState(panels.state,context);flush();assert.equal(moves,1);
  pending.id="decision-2";document.querySelectorAll=()=>[{dataset:{messageId:pending.id},scrollIntoView(){moves+=1;}}];
  registration.onState(panels.state,context);registration.onState(panels.state,{conversation:{id:"another",messages:[]}});flush();assert.equal(moves,1);
});

test("large-to-820 resize preserves attention until the reader deliberately scrolls away",()=>{
  const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:521,scrollTop:0});
  let resized,observed,moves=0;const frames=[];
  class ResizeObserver {constructor(callback){resized=callback;}observe(element){observed=element;}}
  const {registration,panels,window,document}=setup({scroll,ResizeObserver});
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
  document.querySelectorAll=()=>[{dataset:{messageId:"critical",localState:"failed"},scrollIntoView(){moves+=1;scroll.scrollTop=scroll.clientHeight>400?0:168;}}];
  const context={conversation:{id:"chat",messages:[{role:"user"},{id:"critical",kind:"tool",payload:{call:{name:"run_shell"},outcome:{status:"failed",reason:"Désactivé"}}}]}};
  const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
  assert.equal(observed,scroll);resized();assert.equal(frames.length,0);
  registration.onState(panels.state,context);flush();assert.equal(moves,1);assert.equal(scroll.scrollTop,0);
  // A wheel gesture at the boundary did not move the reader anywhere.
  scroll.listeners.wheel({type:"wheel"});scroll.listeners.scroll();flush();
  scroll.clientWidth=560;scroll.clientHeight=301;resized();flush();assert.equal(moves,2);assert.equal(scroll.scrollTop,168);
  // Programmatic/layout scrolling is not a deliberate departure.
  scroll.scrollTop=0;scroll.listeners.scroll();scroll.clientHeight=302;resized();flush();assert.equal(moves,3);
  scroll.listeners.wheel({type:"wheel"});scroll.scrollTop=220;scroll.listeners.scroll();flush();
  scroll.clientWidth=860;scroll.clientHeight=521;resized();flush();
  scroll.clientWidth=560;scroll.clientHeight=301;resized();flush();assert.equal(moves,3);assert.equal(scroll.scrollTop,220);
  registration.onState(panels.state,context);flush();assert.equal(moves,3);
});

test("resize attention waits for settings to close and respects keyboard scrolling",()=>{
  const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:521,scrollTop:0});
  const dialog=new Element("dialog");let resized,modal=false,moves=0;const frames=[];
  class ResizeObserver {constructor(callback){resized=callback;}observe(){}}
  const {registration,panels,window,document}=setup({scroll,ResizeObserver,dialog});
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>modal?dialog:null;
  document.querySelectorAll=()=>[{dataset:{messageId:"decision"},scrollIntoView(){moves+=1;}}];
  const context={conversation:{id:"chat",messages:[{role:"user"},{id:"decision",kind:"approval",payload:{kind:{tool:"write_file"},status:{status:"pending"}}}]}};
  const flush=()=>{while(frames.length)frames.shift()();};
  registration.onState(panels.state,context);flush();assert.equal(moves,1);
  modal=true;scroll.clientHeight=301;resized();flush();assert.equal(moves,1);
  modal=false;dialog.listeners.close();flush();assert.equal(moves,2);
  document.listeners.keydown({type:"keydown",key:"PageDown",target:document.body});scroll.scrollTop=100;scroll.listeners.scroll();flush();
  scroll.clientHeight=521;resized();flush();assert.equal(moves,2);
});

test("tall pending approvals expose their action fieldset instead of clipping buttons",()=>{
  const scroll=Object.assign(new Element("section"),{clientHeight:301});
  const {registration,panels,window,document}=setup({scroll});const frames=[];let actionBlock,cardMoves=0;
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
  const target={dataset:{messageId:"decision"},getBoundingClientRect:()=>({height:450}),querySelector:()=>({scrollIntoView(options){actionBlock=options.block;}}),scrollIntoView(){cardMoves+=1;}};
  document.querySelectorAll=()=>[target];
  registration.onState(panels.state,{conversation:{id:"chat",messages:[{role:"user"},{id:"decision",kind:"approval",payload:{kind:{tool:"run_shell"},status:{status:"pending"}}}]}});
  while(frames.length)frames.shift()();assert.equal(actionBlock,"end");assert.equal(cardMoves,0);
});

test("generated-file actions reveal the decision controls at 820×560 without a new user message",()=>{
  const scroll=Object.assign(new Element("section"),{clientHeight:301});
  const {registration,panels,window,document}=setup({scroll});const frames=[];let actionMoves=0;
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
  document.querySelectorAll=()=>[{dataset:{messageId:"document-approval"},getBoundingClientRect:()=>({height:450}),querySelector:()=>({scrollIntoView(options){assert.equal(options.block,"end");actionMoves+=1;}})}];
  registration.onState(panels.state,{conversation:{id:"chat",messages:[{role:"user"},{id:"document-approval",kind:"approval",payload:{kind:{tool:"document_action"},status:{status:"pending"}}}]}});
  while(frames.length)frames.shift()();assert.equal(actionMoves,1);
});

for (const tool of ["mcp_start_fixture", "routines_background_configure"]) test(`${tool} approvals without a user turn keep their decisions reachable on resize`,()=>{
  const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:521,scrollTop:0});
  let resized,cardMoves=0,actions=0;const frames=[];
  class ResizeObserver {constructor(callback){resized=callback;}observe(){}}
  const {registration,panels,window,document}=setup({scroll,ResizeObserver});
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
  document.querySelectorAll=()=>[{dataset:{messageId:"mcp-settings"},getBoundingClientRect:()=>({height:450}),scrollIntoView(){cardMoves+=1;},querySelector:()=>({scrollIntoView(options){assert.equal(options.block,"end");actions+=1;}})}];
  const context={conversation:{id:"management",messages:[{id:"mcp-settings",kind:"approval",payload:{status:{status:"pending"},kind:{tool},...(tool.startsWith("mcp_")?{connector:{origin:"settings"}}:{})}}]}};
  const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
  registration.onState(panels.state,context);flush();assert.equal(cardMoves,1);
  scroll.clientHeight=301;scroll.clientWidth=560;resized();flush();assert.equal(actions,1);
  scroll.listeners.wheel({type:"wheel"});scroll.scrollTop=80;scroll.listeners.scroll();flush();
  scroll.clientHeight=521;resized();flush();assert.equal(actions,1);assert.equal(cardMoves,1);
});

test("only each affected granted-folder row carries its own app-data warning",()=>{
  const {root,registration,panels}=setup();panels.state.local.folders.push("/tmp/app");
  panels.state.local.folders_with_app_data=["/tmp/app"];
  registration.onState(panels.state,{});
  const items=all(root).filter(e=>e.className==="local-folder-item");
  assert.equal(items.length,2);
  assert.ok(!all(items[0]).some(e=>e.className==="local-data-warning"));
  assert.equal(all(items[1]).find(e=>e.tagName==="code").textContent,"/tmp/app");
  assert.match(all(items[1]).find(e=>e.className==="local-data-warning").textContent,/restent protégées par une carte/);
});

test("a running local call after this turn's human approval waits for its commit and respects reader navigation",()=>{
  const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:521,scrollTop:0});
  let resized,moves=0;const frames=[];
  class ResizeObserver {constructor(callback){resized=callback;}observe(){}}
  const {registration,panels,window,document}=setup({scroll,ResizeObserver});
  window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
  const target={dataset:{messageId:"running-call",localState:""},getBoundingClientRect:()=>({height:250}),scrollIntoView(){moves+=1;}};
  document.querySelectorAll=()=>[target];
  const approval={kind:"approval",payload:{run_id:"run",kind:{tool:"run_shell"},status:{status:"resolved"}}};
  const tool={id:"running-call",kind:"tool",payload:{run_id:"run",run_status:"running",call:{name:"run_shell"}}};
  const context={conversation:{id:"chat",messages:[{role:"user"},approval,tool]}};
  const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
  registration.onState(panels.state,context);assert.equal(frames.length,1);
  frames.shift()();frames.shift()();assert.equal(moves,0);
  target.dataset.localState="running";flush();assert.equal(moves,1);
  registration.onState(panels.state,context);flush();assert.equal(moves,1);
  scroll.clientHeight=301;resized();flush();assert.equal(moves,2);
  document.listeners.keydown({type:"keydown",key:"PageUp",target:document.body});scroll.scrollTop=80;scroll.listeners.scroll();flush();
  scroll.clientHeight=521;resized();flush();assert.equal(moves,2);
});

test("late shell scrolling keeps the complete running header and stop visible at both layouts without overriding readers",()=>{
  for(const height of [521,301]){
    const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:height,scrollTop:0,
      getBoundingClientRect(){return {top:72,bottom:72+this.clientHeight};}});
    let resized,moves=0;const frames=[];
    class ResizeObserver {constructor(callback){resized=callback;}observe(){}}
    const {registration,panels,window,document}=setup({scroll,ResizeObserver});
    window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;
    const header={getBoundingClientRect:()=>({top:192-scroll.scrollTop,bottom:214-scroll.scrollTop})};
    const stop={getBoundingClientRect:()=>({top:372-scroll.scrollTop,bottom:403-scroll.scrollTop})};
    const target={dataset:{messageId:"running-call",localState:"running"},getBoundingClientRect:()=>({height:240}),
      querySelector:selector=>selector===".gb-local-header"?header:selector===".gb-local-stop"?stop:null,
      scrollIntoView(){moves+=1;scroll.scrollTop=120;scroll.listeners.scroll();}};
    document.querySelectorAll=()=>[target];
    const context={conversation:{id:"chat",messages:[{role:"user"},
      {kind:"approval",payload:{run_id:"run",kind:{tool:"run_shell"},status:{status:"resolved"}}},
      {id:"running-call",kind:"tool",payload:{run_id:"run",run_status:"running",call:{name:"run_shell"}}}]}};
    const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
    registration.onState(panels.state,context);flush();assert.equal(moves,1);
    // Actual layout geometry after a late shell/tail follow, not a new state.
    scroll.scrollTop=380;scroll.listeners.scroll();assert.equal(moves,2);flush();
    const view=scroll.getBoundingClientRect();
    for(const element of [header,stop]){const box=element.getBoundingClientRect();assert.ok(box.top>=view.top&&box.bottom<=view.bottom);}
    // A real reading gesture disarms the same local anchor before any correction.
    document.listeners.keydown({type:"keydown",key:"PageDown",target:document.body});scroll.scrollTop=480;scroll.listeners.scroll();flush();
    assert.equal(scroll.scrollTop,480);assert.equal(moves,2);
    scroll.clientHeight=height===521?301:521;resized();flush();assert.equal(moves,2);
    scroll.scrollTop=500;scroll.listeners.scroll();flush();assert.equal(moves,2);
  }
});

test("running attention never borrows an approval from another run, tool, turn or refused decision",()=>{
  for(const change of ["run","tool","turn","denied","expired","cancelled","missing-run"]){

    const {registration,panels,window,document}=setup();const frames=[];
    window.requestAnimationFrame=fn=>frames.push(fn);document.querySelector=()=>null;document.querySelectorAll=()=>[];
    const approval={kind:"approval",payload:{run_id:change==="run"?"other":"run",kind:{tool:change==="tool"?"write_file":"run_shell"},status:{status:["denied","expired","cancelled"].includes(change)?change:"resolved"}}};
    const tool={id:"call",kind:"tool",payload:{run_id:"run",run_status:"running",call:{name:"run_shell"}}};
    if(change==="missing-run"){delete approval.payload.run_id;delete tool.payload.run_id;}
    const messages=[{role:"user"},approval,...(change==="turn"?[{role:"user"}]:[]),tool];
    registration.onState(panels.state,{conversation:{id:"chat",messages}});assert.equal(frames.length,0,change);
  }
});

function runningLayout() {
  let time=0,moves=0,reads=0,resized;const frames=[];
  class Clock extends Date { static now(){return time;} }
  class ResizeObserver {constructor(fn){resized=fn;}observe(){}}
  const scroll=Object.assign(new Element("section"),{clientWidth:860,clientHeight:301,scrollTop:0,getBoundingClientRect(){return {top:72,bottom:373};}});
  const f=setup({scroll,ResizeObserver,clock:Clock});
  f.window.requestAnimationFrame=fn=>frames.push(fn);f.document.querySelector=()=>null;
  const target={dataset:{messageId:"running",localState:"running"},getBoundingClientRect:()=>({height:240}),
    querySelector:selector=>({getBoundingClientRect:()=>({top:2072-scroll.scrollTop+(selector===".gb-local-stop"?180:0),bottom:2072-scroll.scrollTop+(selector===".gb-local-stop"?211:22)})}),
    scrollIntoView(){moves++;scroll.scrollTop=2000;}};
  f.document.querySelectorAll=()=>[target];
  const messages=[...Array.from({length:50},(_,i)=>({id:`old-${i}`,role:i%2?"assistant":"user"})),{role:"user"},
    {kind:"approval",payload:{run_id:"run",kind:{tool:"run_shell"},status:{status:"resolved"}}},
    {id:"running",kind:"tool",payload:{run_id:"run",run_status:"running",call:{name:"run_shell"}}}];
  const context={conversation:{id:"long-chat",get messages(){reads++;return messages;}}};
  const flush=()=>{let count=0;while(frames.length){assert.ok(++count<40);frames.shift()();}};
  f.registration.onState(f.panels.state,context);flush();
  return {...f,scroll,flush,context,resized,moves:()=>moves,reads:()=>reads,resetReads:()=>{reads=0;},time:value=>{time=value;}};
}

test("body-focused PageUp and Home release running anchoring without making the chat focusable",()=>{
  for(const key of ["PageUp","Home"]){
    const f=runningLayout();assert.equal(f.document.activeElement,f.document.body);assert.equal(f.scroll.tabIndex,undefined);
    assert.equal(f.moves(),1);assert.equal(typeof f.document.listeners.keydown,"function");
    f.document.listeners.keydown({type:"keydown",key,target:f.document.body});f.flush();
    f.scroll.scrollTop=key==="Home"?0:1200;f.scroll.listeners.scroll();f.flush();
    assert.equal(f.moves(),1);assert.equal(f.scroll.scrollTop,key==="Home"?0:1200);
    f.registration.onState(f.panels.state,f.context);f.flush();assert.equal(f.moves(),1);
  }
});

test("scrollbar pointer intent remains armed beyond two frames until pointer release",()=>{
  const f=runningLayout();f.scroll.listeners.pointerdown({type:"pointerdown",target:f.scroll});f.flush();
  f.scroll.scrollTop=1000;f.scroll.listeners.scroll();f.flush();assert.equal(f.moves(),1);assert.equal(f.scroll.scrollTop,1000);
  f.document.listeners.pointerup({type:"pointerup"});f.flush();f.resized();assert.equal(f.moves(),1);
});

test("late-shell correction expires after one second and cached scroll checks do not scan the transcript",()=>{
  const f=runningLayout();f.resetReads();
  for(let i=0;i<5;i++)f.scroll.listeners.scroll();assert.equal(f.reads(),0);
  f.time(500);f.scroll.scrollTop=2500;f.scroll.listeners.scroll();f.flush();assert.equal(f.moves(),2);
  f.resetReads();f.time(1100);f.scroll.scrollTop=2500;f.scroll.listeners.scroll();f.flush();assert.equal(f.moves(),2);assert.equal(f.reads(),0);
  f.scroll.clientHeight=521;f.resized();f.flush();assert.equal(f.moves(),2);
});

test("granted paths remain text and cancellation uses the existing run stop route",async()=>{
  const {root,requests,registration}=setup();
  assert.equal(all(root).find(e=>e.tagName==="code").textContent,"/tmp/<dossier>");
  assert.equal(await registration.onCardAction({kind:"tool",payload:{run_id:"run-local"}},"local.stop",{}, {conversation:{id:"chat"}}),true);
  assert.deepEqual(JSON.parse(JSON.stringify(requests)),[{path:"/api/stop",body:{conversation_id:"chat",message_id:"run-local"}}]);
});
