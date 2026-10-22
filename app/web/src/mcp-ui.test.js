import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

// Execute the settings panel against a small DOM boundary. Assertions concern
// rendered controls and actual submitted API bodies, never implementation text.
class Node {
  constructor(tag, ownerDocument) { this.tag = tag; this.ownerDocument = ownerDocument; this.children = []; this.events = {}; this.attrs = {}; this.value = ""; this.className = ""; this.text = ""; }
  set textContent(value) { this.text = String(value); this.children = []; }
  get textContent() { return this.text + this.children.map(n => n.textContent).join(""); }
  append(...nodes) { for (const n of nodes) { n.remove(); n.parentNode = this; this.children.push(n); } }
  replaceChildren(...nodes) { for (const n of [...this.children]) n.remove(); this.append(...nodes); }
  remove() { if (!this.parentNode) return; if (this.contains(this.ownerDocument?.activeElement)) this.ownerDocument.activeElement = this.ownerDocument.body; this.parentNode.children = this.parentNode.children.filter(n => n !== this); this.parentNode = null; }
  replaceWith(node) { const p = this.parentNode, i = p.children.indexOf(this); this.remove(); node.remove(); p.children.splice(i,0,node); node.parentNode=p; }
  contains(node) { return this.all().includes(node); }
  setAttribute(key, value) { this.attrs[key] = value; }
  getAttribute(key) { return this.attrs[key] ?? null; }
  addEventListener(name, action) { this.events[name] = action; }
  all() { return [this, ...this.children.flatMap(n => n.all())]; }
  querySelectorAll(selector) { return this.all().slice(1).filter(n => selector === 'input[type="password"]' ? n.tag === "input" && n.type === "password" : selector.split(",").map(s=>s.trim()).includes(n.tag)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0]; }
  reset() { for (const n of this.all()) n.value = ""; }
  focus() { this.focused = true; this.ownerDocument.activeElement = this; }
  setSelectionRange(start,end,direction) { this.selectionStart=start; this.selectionEnd=end; this.selectionDirection=direction; }
  requestSubmit() { return this.events.submit({preventDefault(){}}); }
}
async function mount(servers = [], actionResult = {}, catalogue = []) {
  const root = new Node("section"), calls = [], notifications = [], opened = [], copies = [], timers = new Map();
  let nextTimer=0;
  const dialog = { open: true, close() { this.open = false; } };
  let panel;
  const panels = {
    state: { bots: [{ id: "grokbot", name: "Ezerbot" }], mcp: { servers, catalogue } },
    register(value) { panel = value; },
    async api(path, body) { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); if (panels.failRequest) throw new Error("Action refusée."); return actionResult; },
    async copyText(text) { copies.push(text); },
    async refresh() {}, notify(text) { notifications.push(text); },
    openConversation(id) { opened.push(id); return true; },
  };
  const document = { getElementById: id => id === "settings-dialog" ? dialog : undefined, createElement: tag => new Node(tag,document), createTextNode: text => { const n = new Node("#text",document); n.textContent = text; return n; } };
  root.ownerDocument = document; document.body = new Node("body",document); document.body.append(root); document.activeElement = document.body;
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setTimeout(fn) { const id=++nextTimer;timers.set(id,fn);return id; },clearTimeout(id){timers.delete(id);} });
  vm.runInContext(await readFile(new URL("../../ui/panels/mcp.js", import.meta.url), "utf8"), context);
  panel.settingsSection(root);
  return { root, calls, notifications, opened, dialog, copies, document, panels, setState(state) {panels.state=state;panel.onState(state);},runTimers(){const callbacks=[...timers.values()];timers.clear();callbacks.forEach(fn=>fn());} };
}

test("MCP management opens its approval without a toast over the decision buttons", async () => {
  const { root, calls, notifications, opened, dialog } = await mount([{id:"local",name:"Atelier",status:"disconnected",command:"python3",arguments:[],environment_names:[],enabled_bots:[],tools:[]}], {conversation_id:"approbations"});
  await root.all().find(n => n.tag === "button" && n.textContent === "Démarrer…").events.click();
  assert.equal(calls[0].path, "/api/mcp/restart");
  assert.equal(calls[0].body.id, "local");
  assert.equal(dialog.open, false);
  assert.deepEqual(opened, ["approbations"]);
  assert.equal(notifications.length, 0);
});

test("MCP form ignores trailing blank argument lines, keeps interior spaces, and confirms inside settings", async () => {
  const { root, calls, notifications } = await mount();
  root.all().find(n => n.name === "name").value = "Atelier";
  root.all().find(n => n.name === "command").value = "python3";
  root.all().find(n => n.name === "arguments").value = "script1.py\r\n  argument espacé  \r\n\r\n";
  await root.all().find(n => n.tag === "form").events.submit({ preventDefault() {} });
  assert.equal(calls[0].path, "/api/mcp/add");
  assert.deepEqual(Array.from(calls[0].body.arguments), ["script1.py", "  argument espacé  "]);
  assert.equal(notifications.length, 0);
  const feedback = root.all().find(n => n.attrs.role === "status");
  assert.equal(feedback.className, "mcp-note");
  assert.match(feedback.textContent, /Connecteur enregistré/);
});

test("MCP validation uses the error boundary and does not send invalid environment entries", async () => {
  const { root, calls } = await mount();
  root.all().find(n => n.tag === "button" && n.textContent === "Ajouter une variable privée").events.click();
  await root.all().find(n => n.tag === "form").events.submit({ preventDefault() {} });
  assert.equal(calls.length, 0);
  const feedback = root.all().find(n => n.attrs.role === "status");
  assert.equal(feedback.className, "mcp-error");
  assert.match(feedback.textContent, /nom unique et non vide/);
});

test("MCP statuses have accessible text, decorative dots, and one shared per-bot explanation", async () => {
  const servers = ["connected", "crashed", "error", "busy", "starting"].map((status, i) => ({ id: String(i), name: `Serveur ${i}`, status, command: "python3", arguments: [], environment_names: [], enabled_bots: [], tools: [] }));
  const { root } = await mount(servers);
  const statuses = root.all().filter(n => n.className.startsWith("mcp-status "));
  assert.deepEqual(statuses.map(n => n.textContent), ["Connecté", "Serveur arrêté", "En erreur", "Occupé", "Démarrage en cours"]);
  assert.ok(statuses.every(n => n.children[0].className === "status-dot" && n.children[0].attrs["aria-hidden"] === "true"));
  assert.equal(root.all().filter(n => n.tag === "p" && n.textContent.startsWith("La désactivation")).length, 1);
});


const service = { id: "gmail", name: "Gmail", status: "disconnected", description: "Courriels", permissions: "Lecture et envoi", scopes: "gmail.modify", setup: "Client OAuth de bureau", license: "MIT", source: "https://example.test", needs_client: true, has_tokens: false };

test("catalogue explains permissions and submits private setup only to its host approval path", async () => {
  const { root, calls, opened } = await mount([], {conversation_id:"approbations"}, [service]);
  assert.match(root.textContent, /Catalogue/);
  assert.match(root.textContent, /Autorisations : Lecture et envoi/);
  root.all().find(n => n.name === "gmail-client").value = "client-factice";
  const secret = root.all().find(n => n.name === "gmail-secret");
  assert.equal(secret.type, "password");
  secret.value = "secret-factice";
  const form = root.all().find(n => n.tag === "form");
  await form.events.submit({preventDefault(){}});
  assert.equal(calls[0].path, "/api/connectors/connect");
  assert.equal(calls[0].body.id, "gmail");
  assert.equal(secret.value, "");
  assert.equal(calls[0].body.client_secret, "secret-factice", "the submitted request contains the private setup, never the rendered card");
  assert.equal(root.all().filter(n => n.tag === "article").flatMap(n => n.all()).filter(n => n.tag === "button" && n.textContent === "Connecter").length, 1);
  assert.deepEqual(opened, ["approbations"]);
});

test("connected catalogue services use the existing bot selection and explicit disconnect approvals", async () => {
  const server = {id:"catalogue-gmail",enabled_bots:[],tools:[{name:"echo",description:"Outil factice"}]};
  const { root, calls } = await mount([server], {}, [{...service,status:"connected",has_tokens:true,server}]);
  assert.equal(root.all().filter(n => n.tag === "article").length, 1, "not duplicated as a custom server");
  const check = root.all().find(n => n.type === "checkbox"); check.checked = true;
  await check.events.change();
  assert.equal(calls[0].path, "/api/mcp/select"); assert.equal(calls[0].body.id, server.id);
  await root.all().find(n => n.tag === "button" && n.textContent === "Déconnecter").events.click();
  assert.equal(calls[1].path, "/api/connectors/disconnect");
});

test("pending OAuth has a browser link, while expired credentials have a reconnect action", async () => {
  const { root } = await mount([], {}, [{...service,status:"authorizing",authorization_url:"http://127.0.0.1:1234/authorize"},{...service,id:"github",name:"GitHub",status:"reconnect"}]);
  const link = root.all().find(n => n.tag === "a");
  assert.equal(link.target, "_blank"); assert.equal(link.rel, "noopener noreferrer");
  assert.equal(root.all().filter(n => n.tag === "button" && n.textContent === "Reconnecter").length, 1);
  assert.match(root.textContent, /À reconnecter/);
});

test("catalogue has one CTA, no Notion form, and no saved-client claim for a new client", async () => {
  const { root, calls } = await mount([], {}, [service,{...service,id:"notion",name:"Notion",needs_client:false,scopes:"default"}]);
  const [gmail,notion] = root.all().filter(n => n.tag === "article");
  assert.equal(gmail.all().filter(n => n.tag === "button" && n.textContent === "Connecter").length, 1);
  assert.ok(!gmail.textContent.includes("réutiliser la configuration"));
  const action=gmail.all().find(n => n.attrs["data-connector-action"] === "connect");
  await action.events.click();
  assert.equal(calls.length,0);
  assert.equal(gmail.all().find(n => n.name === "gmail-client").focused,true);
  assert.equal(gmail.all().filter(n => n.tag === "form")[0].children.some(n => n.tag === "button"),false);
  assert.equal(notion.all().filter(n => n.tag === "form").length,0);
  assert.ok(!notion.textContent.includes("default"));
  await notion.all().find(n => n.attrs["data-connector-action"] === "connect").events.click();
  assert.equal(calls[0].body.id,"notion");
});

test("a retained OAuth client connects directly and only an open setup submits changed credentials", async () => {
  const { root,calls } = await mount([],{},[{...service,has_client:true}]);
  const card=root.all().find(n => n.tag === "article");
  assert.match(card.textContent,/Laissez les champs vides/);
  const action=card.all().find(n => n.attrs["data-connector-action"] === "connect");
  await action.events.click();
  assert.deepEqual(calls[0].body,{id:"gmail"});
  const setup=card.all().find(n => n.tag === "details");setup.open=true;
  card.all().find(n => n.name === "gmail-client").value="changed-client";
  card.all().find(n => n.name === "gmail-secret").value="changed-secret";
  await action.events.click();
  assert.equal(calls[1].body.client_id,"changed-client");
  assert.equal(calls[1].body.client_secret,"changed-secret");
});

test("bot switches and last tools stay available in every persisted connection state", async () => {
  for (const status of ["connected","authorizing","reconnect","saved","error"]) {
    const server={id:"gmail-server",enabled_bots:["grokbot"],tools:[{name:"echo",description:"Outil"}]};
    const {root}=await mount([server],{},[{...service,server,status,has_tokens:true,account:"factice@example.test"}]);
    assert.equal(root.all().find(n=>n.type==="checkbox").checked,true);
    assert.ok(root.textContent.includes("Outils (1)"));
    assert.ok(root.textContent.includes("Compte connecté : factice@example.test"));
    const action=root.all().find(n=>n.attrs["data-connector-action"]==="connect");
    if (status==="connected" || status==="saved") assert.equal(action.className,"secondary-button");
  }
});

test("revocation warning has an action tone and public setup values can be copied", async () => {
  const redirect="http://127.0.0.1:52187/oauth/slack/callback";
  const {root,copies,runTimers}=await mount([],{},[{...service,id:"slack",name:"Slack",warning:true,message:"Révocation distante non confirmée.",redirect_uri:redirect}]);
  assert.equal(root.all().find(n=>n.textContent==="Révocation distante non confirmée.").className,"mcp-warning");
  const copy=root.all().find(n=>n.attrs["aria-label"]==="Copier url de retour oauth — Slack");
  await copy.events.click();
  assert.deepEqual(copies,[redirect]);assert.equal(copy.textContent,"Copié");
  runTimers();assert.equal(copy.textContent,"Copier");
});

const catalogueCard = (root,id) => root.all().find(n => n.attrs["data-connector"] === id);

test("account tokens make application setup read-only until the approved disconnect", async () => {
  for (const status of ["connected","saved","reconnect","error","authorizing"]) {
    const {root}=await mount([],{},[{...service,status,has_tokens:true,has_client:true}]);
    const card=catalogueCard(root,"gmail");
    assert.equal(card.all().filter(n=>n.tag==="form").length,0);
    assert.ok(card.textContent.includes("déconnectez d’abord le service"));
  }
});

test("a first consent does not offer a misleading bot switch or last empty catalogue", async () => {
  const server={id:"first",enabled_bots:[],tools:[]};
  for (const status of ["authorizing","reconnect","error","saved"]) {
    const {root}=await mount([],{},[{...service,server,status}]);
    const card=catalogueCard(root,"gmail");
    assert.equal(card.all().filter(n=>n.type==="checkbox").length,0);
    assert.ok(!card.textContent.includes("Outils (0)"));
    assert.ok(card.textContent.includes("disponibles après la connexion"));
  }
  const {root}=await mount([],{},[{...service,status:"authorizing",server:{...server,tools:[{name:"echo"}]}}]);
  assert.match(catalogueCard(root,"gmail").textContent,/Une nouvelle identification réinitialise l’activation par bot/);
});

test("another service finishing OAuth or changing busy/bot state cannot detach a typed Slack draft", async () => {
  const google={...service,id:"calendar",name:"Calendar",status:"authorizing",server:{id:"calendar-server",enabled_bots:[],tools:[]}};
  const slack={...service,id:"slack",name:"Slack"};
  const manual={id:"manual",name:"Atelier",status:"disconnected",command:"python3",arguments:[],environment_names:[],enabled_bots:[],tools:[]};
  const f=await mount([manual,google.server],{},[google,slack]);
  const original=catalogueCard(f.root,"slack"),custom=f.root.all().find(n=>n.className==="mcp-server");
  original.querySelector("details").open=true;
  const [client,secret]=original.querySelectorAll("input");client.value="slack-app-draft";secret.value="slack-secret-draft";secret.focus();secret.setSelectionRange(4,7,"forward");
  for (const status of ["connected","busy","connected"]) {
    const updated={...google,has_tokens:true,status:status==="busy"?"connected":status,server:{...google.server,status,enabled_bots:["grokbot"],tools:[{name:"echo"}]}};
    f.setState({...f.panels.state,mcp:{servers:[manual,updated.server],catalogue:[updated,{...slack}]}});
    assert.ok(catalogueCard(f.root,"slack")===original);
    assert.equal(client.value,"slack-app-draft");assert.equal(secret.value,"slack-secret-draft");
    assert.equal(original.querySelector("details").open,true);assert.ok(f.document.activeElement===secret);
    assert.equal(secret.selectionStart,4);assert.equal(secret.selectionEnd,7);
    assert.ok(f.root.all().find(n=>n.className==="mcp-server")===custom);
    assert.ok(!JSON.stringify(f.panels.state).includes("slack-secret-draft"));
  }
});

test("even its own metadata update preserves an editable draft, open details, focus and selection", async () => {
  const slack={...service,id:"slack",name:"Slack",has_client:true};const f=await mount([],{},[slack]);
  const original=catalogueCard(f.root,"slack");original.querySelector("details").open=true;
  const [client,secret]=original.querySelectorAll("input");client.value="client-draft";secret.value="secret-draft";secret.focus();secret.setSelectionRange(2,6,"forward");
  f.setState({...f.panels.state,mcp:{servers:[],catalogue:[{...slack,message:"État actualisé"}]}});
  const updated=catalogueCard(f.root,"slack");assert.ok(updated!==original);
  const fields=updated.querySelectorAll("input");assert.equal(fields[0].value,"client-draft");assert.equal(fields[1].value,"secret-draft");
  assert.equal(updated.querySelector("details").open,true);assert.ok(f.document.activeElement===fields[1]);
  assert.equal(fields[1].selectionStart,2);assert.equal(fields[1].selectionEnd,6);
});

test("a focused setup field disappearing after sign-in cannot interrupt the remaining render", async () => {
  const f=await mount([],{},[{...service,has_client:true}]);const card=catalogueCard(f.root,"gmail");card.querySelector("details").open=true;card.querySelectorAll("input")[1].focus();
  const server={id:"gmail-server",enabled_bots:[],tools:[{name:"echo"}]};
  assert.doesNotThrow(()=>f.setState({...f.panels.state,mcp:{servers:[server],catalogue:[{...service,server,status:"connected",has_tokens:true,has_client:true}]}}));
  assert.equal(catalogueCard(f.root,"gmail").querySelectorAll("form").length,0);assert.ok(f.document.activeElement===f.document.body);
});

test("a focused bot checkbox survives the service's own confirmed metadata update", async () => {
  const server={id:"gmail-server",enabled_bots:[],tools:[{name:"echo"}]};const item={...service,server,status:"connected",has_tokens:true};const f=await mount([server],{},[item]);
  const old=catalogueCard(f.root,"gmail").querySelector("input");old.focus();
  const confirmed={...server,enabled_bots:["grokbot"]};f.setState({...f.panels.state,mcp:{servers:[confirmed],catalogue:[{...item,server:confirmed}]}});
  const next=catalogueCard(f.root,"gmail").querySelector("input");assert.ok(next!==old);assert.ok(f.document.activeElement===next);assert.equal(next.checked,true);
});

test("a denied bot selection restores the confirmed check without erasing another draft", async () => {
  const server={id:"gmail-server",enabled_bots:[],tools:[{name:"echo"}]};
  const f=await mount([server],{},[{...service,server,status:"connected",has_tokens:true},{...service,id:"slack",name:"Slack"}]);
  const draft=catalogueCard(f.root,"slack");draft.querySelector("details").open=true;const secret=draft.querySelectorAll("input")[1];secret.value="unsent-private-draft";secret.focus();
  const check=catalogueCard(f.root,"gmail").querySelector("input");check.checked=true;f.panels.failRequest=true;await check.events.change();
  assert.equal(catalogueCard(f.root,"gmail").querySelector("input").checked,false);
  assert.ok(catalogueCard(f.root,"slack")===draft);assert.equal(secret.value,"unsent-private-draft");assert.ok(f.document.activeElement===secret);
});

const github = {...service,id:"github",name:"GitHub",needs_client:false,has_client:true,configured:true,device_flow:true,device:null,scopes:"",scopes_label:"Permissions de la GitHub App d’Ezerbot, choisies chez GitHub",redirect_uri:null};

test("GitHub offers no client fields and connects in one click", async () => {
  const { root, calls } = await mount([], {}, [github]);
  const card = catalogueCard(root,"github");
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  assert.ok(!card.textContent.includes("Identifiant OAuth"));
  assert.ok(!card.textContent.includes("URL de retour"));
  assert.ok(!card.textContent.includes("Portées OAuth"));
  assert.match(card.textContent, /aucun secret à saisir/);
  await card.all().find(n => n.attrs["data-connector-action"] === "connect").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/connect",body:{id:"github"}});
});

const gmail = {...service,id:"gmail",name:"Gmail",needs_client:false,has_client:true,configured:true,unconfigured_message:null,scopes:"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose https://www.googleapis.com/auth/gmail.modify",scopes_label:"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose https://www.googleapis.com/auth/gmail.modify",redirect_uri:null};

test("Gmail offers no client fields, shows its account and connects in one click", async () => {
  const { root, calls } = await mount([], {}, [{...gmail,status:"connected",account:"charles@example.test",has_tokens:true,server:{id:"g1",enabled_bots:[],tools:[{name:"list_recent_messages"}]}}]);
  const card = catalogueCard(root,"gmail");
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input" && n.type !== "checkbox").length, 0);
  assert.ok(!card.textContent.includes("Identifiant OAuth"));
  assert.ok(!card.textContent.includes("Secret OAuth"));
  assert.ok(!card.textContent.includes("Portées OAuth"));
  assert.match(card.textContent, /aucun secret à saisir/);
  assert.match(card.textContent, /Compte connecté : charles@example.test/);
  const idle = await mount([], {}, [gmail]);
  const fresh = catalogueCard(idle.root,"gmail");
  assert.equal(fresh.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  await fresh.all().find(n => n.attrs["data-connector-action"] === "connect").events.click();
  assert.deepEqual(idle.calls[0], {path:"/api/connectors/connect",body:{id:"gmail"}});
});

test("a Gmail card without the Google client of Ezerbot explains it and never shows the form", async () => {
  const sentence = "Le client Google d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion à Gmail reste indisponible tant que ce client n’est pas préparé.";
  const { root } = await mount([], {}, [{...gmail,configured:false,has_client:false,unconfigured_message:sentence}]);
  const card = catalogueCard(root,"gmail");
  assert.match(card.textContent, /client Google d’Ezerbot n’est pas configuré/);
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  assert.ok(!card.textContent.includes("Identifiant OAuth"));
});

test("an unconfigured GitHub application is explained on the card and by the refused click", async () => {
  const { root } = await mount([], {}, [{...github,configured:false,has_client:false}]);
  const card = catalogueCard(root,"github");
  assert.match(card.textContent, /n’est pas configurée/);
  assert.equal(card.all().filter(n => n.tag === "input").length, 0);
});

const slackClient = {...service,id:"slack",name:"Slack",needs_client:false,has_client:true,configured:true,unconfigured_message:null,scopes_label:"Lecture seule des canaux, choisie pendant le consentement Slack",redirect_uri:null};

test("Slack offers no client fields and connects in one click", async () => {
  const { root, calls } = await mount([], {}, [slackClient]);
  const card = catalogueCard(root,"slack");
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  assert.ok(!card.textContent.includes("Identifiant OAuth"));
  assert.ok(!card.textContent.includes("Secret OAuth"));
  assert.ok(!card.textContent.includes("URL de retour"));
  assert.ok(!card.textContent.includes("Portées OAuth"));
  assert.match(card.textContent, /aucun secret à saisir/);
  await card.all().find(n => n.attrs["data-connector-action"] === "connect").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/connect",body:{id:"slack"}});
});

test("a Slack client that is not configured is explained on the card and offers no form", async () => {
  const said = "Le client Slack d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion reste indisponible tant que le client n’est pas préparé.";
  const { root } = await mount([], {}, [{...slackClient,configured:false,has_client:false,unconfigured_message:said}]);
  const card = catalogueCard(root,"slack");
  assert.ok(card.textContent.includes(said));
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  const warning = card.all().find(n => n.className === "mcp-warning");
  assert.equal(warning.textContent, said);
});

test("the device code is shown, copyable and cancellable while GitHub waits", async () => {
  const expires_at = Date.UTC(2026,9,22,12,15);
  const item = {...github,status:"authorizing",authorization_url:"https://github.com/login/device",device:{user_code:"WDJB-MJHT",verification_uri:"https://github.com/login/device",expires_at},server:{id:"c1",enabled_bots:[],tools:[]}};
  const { root, calls, copies } = await mount([], {}, [item]);
  const card = catalogueCard(root,"github");
  const code = card.all().find(n => n.tag === "code" && n.textContent === "WDJB-MJHT");
  assert.ok(code, "the 8-character code is visible");
  assert.match(card.textContent, /En attente de votre autorisation chez GitHub/);
  const copy = card.all().find(n => n.attrs["aria-label"] === "Copier le code de connexion — GitHub");
  await copy.events.click();
  assert.deepEqual(copies, ["WDJB-MJHT"]);
  const links = card.all().filter(n => n.tag === "a");
  assert.equal(links.length, 1, "one page link, not two");
  assert.equal(links[0].href, "https://github.com/login/device");
  assert.equal(links[0].rel, "noopener noreferrer");
  assert.equal(card.all().filter(n => n.tag === "button" && n.textContent === "Déconnecter").length, 0);
  assert.equal(card.all().filter(n => n.tag === "button" && n.textContent === "Connecter").length, 0);
  await card.all().find(n => n.tag === "button" && n.textContent === "Annuler").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/cancel",body:{id:"github"}});
});

test("only the device flow offers a cancel while other services keep their browser link", async () => {
  const { root } = await mount([], {}, [{...service,status:"authorizing",authorization_url:"http://127.0.0.1:1234/authorize"}]);
  assert.equal(root.all().filter(n => n.tag === "button" && n.textContent === "Annuler").length, 0);
  assert.equal(root.all().filter(n => n.tag === "a").length, 1);
});

const catalogueServices = [
  {...service,description:"Rechercher et lire vos courriels, préparer et envoyer des messages."},
  {...service,id:"calendar",name:"Google Calendar",description:"Consulter vos agendas et gérer vos événements."},
  {...service,id:"drive",name:"Google Drive",description:"Rechercher, lire et organiser les fichiers de votre Drive."},
  {...service,id:"slack",name:"Slack",description:"Rechercher des discussions, lire les fils et envoyer des messages."},
  {...service,id:"notion",name:"Notion",description:"Rechercher et modifier les pages de votre espace Notion.",needs_client:false},
  {...github,description:"Lister les issues de vos dépôts et préparer des commentaires depuis le chat, avec votre compte GitHub."},
];
const searchField = root => root.all().find(n => n.tag === "input" && n.type === "search");
const type = (field, text) => { field.value = text; return field.events.input({}); };
const shown = root => root.all().filter(n => n.tag === "article" && !n.hidden).map(n => n.getAttribute("data-connector"));
const emptyState = root => root.all().find(n => n.attrs.role === "status" && n.className === "mcp-note" && /Aucun service/.test(n.textContent));

test("searching « git » keeps only GitHub, which connects in one click without any client field", async () => {
  const { root, calls } = await mount([], {}, catalogueServices);
  assert.deepEqual(shown(root), ["gmail","calendar","drive","slack","notion","github"]);
  type(searchField(root), "git");
  assert.deepEqual(shown(root), ["github"]);
  const card = catalogueCard(root,"github");
  assert.equal(card.all().filter(n => n.tag === "form" || n.tag === "input").length, 0);
  await card.all().find(n => n.attrs["data-connector-action"] === "connect").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/connect",body:{id:"github"}});
  assert.equal(emptyState(root), undefined);
});

test("clearing the search shows the six services again", async () => {
  const { root } = await mount([], {}, catalogueServices);
  const field = searchField(root);
  type(field, "git");
  type(field, "");
  assert.deepEqual(shown(root), ["gmail","calendar","drive","slack","notion","github"]);
  type(field, "   ");
  assert.equal(shown(root).length, 6, "blank spaces are not a query");
});

test("the search matches the description too, ignoring case and accents", async () => {
  const { root } = await mount([], {}, catalogueServices);
  const field = searchField(root);
  type(field, "ÉVÉNEMENTS");
  assert.deepEqual(shown(root), ["calendar"]);
  type(field, "rechercher");
  assert.deepEqual(shown(root), ["gmail","drive","slack","notion"]);
});

test("a search without result says so instead of leaving a blank panel", async () => {
  const { root } = await mount([], {}, catalogueServices);
  const field = searchField(root);
  assert.equal(emptyState(root), undefined);
  type(field, "zzzz-absent");
  assert.deepEqual(shown(root), []);
  assert.match(emptyState(root).textContent, /Aucun service ne correspond à « zzzz-absent »/);
  type(field, "slack");
  assert.equal(emptyState(root), undefined);
});

test("a service that needs a client keeps its client fields when the search shows it", async () => {
  const { root } = await mount([], {}, catalogueServices);
  type(searchField(root), "gmail");
  assert.deepEqual(shown(root), ["gmail"]);
  const card = catalogueCard(root,"gmail");
  assert.ok(card.all().find(n => n.name === "gmail-client"));
  assert.equal(card.all().find(n => n.name === "gmail-secret").type, "password");
});

test("the search and a typed draft survive a catalogue refresh", async () => {
  const { root, setState } = await mount([], {}, catalogueServices);
  const field = searchField(root);
  type(field, "slack");
  root.all().find(n => n.name === "slack-client").value = "brouillon";
  setState({bots:[{id:"grokbot",name:"Ezerbot"}],mcp:{servers:[],catalogue:catalogueServices.map(s => s.id === "slack" ? s : {...s,message:"Mise à jour"})}});
  assert.equal(searchField(root), field);
  assert.equal(field.value, "slack");
  assert.deepEqual(shown(root), ["slack"]);
  setState({bots:[{id:"grokbot",name:"Ezerbot"}],mcp:{servers:[],catalogue:catalogueServices.map(s => s.id === "gmail" ? {...s,status:"error",message:"Boum"} : s)}});
  assert.deepEqual(shown(root), ["slack"], "a card re-rendered while hidden stays hidden");
  assert.equal(root.all().find(n => n.name === "slack-client").value, "brouillon");
});

const twoAccounts = () => {
  const first = {id:"c1",enabled_bots:["grokbot"],tools:[{name:"list_issues",description:"Lister"}]};
  const second = {id:"c2",enabled_bots:[],tools:[{name:"list_issues",description:"Lister"}]};
  const accounts = [
    {id:"github",account:"charles-test",status:"connected",device:null,has_tokens:true,server:first},
    {id:"github:2",account:"autre-compte",status:"connected",device:null,has_tokens:true,server:second},
  ];
  return {first,second,item:{...github,slot:"github",status:"connected",has_tokens:true,account:"charles-test",server:first,accounts}};
};

test("GitHub shows both accounts, adds another one, and disconnects only the account it names", async () => {
  const {first,second,item} = twoAccounts();
  const { root, calls } = await mount([first,second], {}, [item]);
  const card = catalogueCard(root,"github");
  assert.match(card.textContent, /Compte connecté : charles-test/);
  assert.match(card.textContent, /Compte connecté : autre-compte/);
  assert.equal(root.all().filter(n => n.className === "mcp-server" && n.tag === "article").length, 0, "the accounts are not custom servers");
  await card.all().find(n => n.attrs["data-connector-action"] === "add-account").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/connect",body:{id:"github",another:true}});
  const block = card.all().find(n => n.attrs["data-account"] === "github:2");
  await block.all().find(n => n.attrs["data-connector-action"] === "disconnect-account").events.click();
  assert.deepEqual(calls[1], {path:"/api/connectors/disconnect",body:{id:"github",account:"github:2"}});
  const check = block.all().find(n => n.type === "checkbox"); check.checked = true;
  await check.events.change();
  assert.deepEqual(calls[2], {path:"/api/mcp/select",body:{id:"c2",bot_id:"grokbot",enabled:true}});
  const lead = card.all().find(n => n.attrs["data-connector-action"] === "disconnect");
  assert.equal(lead.attrs["aria-label"], "Déconnecter charles-test");
  await lead.events.click();
  assert.deepEqual(calls[3], {path:"/api/connectors/disconnect",body:{id:"github"}});
});

test("a second account waits for its code beside the first and is cancelled by its own button", async () => {
  const {first,item} = twoAccounts();
  const device = {user_code:"WDJB-MJH2",verification_uri:"https://github.com/login/device",expires_at:Date.UTC(2026,9,22,12,15)};
  item.accounts[1] = {id:"github:2",account:null,status:"authorizing",device,has_tokens:false,server:{id:"c2",enabled_bots:[],tools:[]}};
  const { root, calls, copies } = await mount([first], {}, [item]);
  const card = catalogueCard(root,"github");
  const block = card.all().find(n => n.attrs["data-account"] === "github:2");
  assert.ok(block.all().find(n => n.tag === "code" && n.textContent === "WDJB-MJH2"));
  assert.equal(card.all().filter(n => n.attrs["data-connector-action"] === "add-account").length, 0, "one wait at a time");
  await block.all().find(n => n.attrs["aria-label"] === "Copier le code de connexion — GitHub (nouveau compte)").events.click();
  assert.deepEqual(copies, ["WDJB-MJH2"]);
  await block.all().find(n => n.attrs["data-connector-action"] === "cancel-account").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/cancel",body:{id:"github",account:"github:2"}});
});

test("when the first slot is gone the card leads with the next account and names its slot", async () => {
  const second = {id:"c2",enabled_bots:[],tools:[]};
  const item = {...github,slot:"github:2",status:"authorizing",account:null,server:second,device:{user_code:"WDJB-MJH3",verification_uri:"https://github.com/login/device",expires_at:Date.UTC(2026,9,22,12,15)},accounts:[{id:"github:2",account:null,status:"authorizing",device:null,server:second}]};
  const { root, calls } = await mount([second], {}, [item]);
  const card = catalogueCard(root,"github");
  await card.all().find(n => n.tag === "button" && n.textContent === "Annuler").events.click();
  assert.deepEqual(calls[0], {path:"/api/connectors/cancel",body:{id:"github",account:"github:2"}});
});

test("a GitHub slot refused before its first connection is not a custom server", async () => {
  const {first,item} = twoAccounts();
  item.accounts = item.accounts.slice(0,1);
  const refused = {id:"c3",name:"GitHub",status:"disconnected",command:"https://api.github.com",arguments:[],environment_names:[],enabled_bots:[],tools:[]};
  item.slot_servers = ["c1","c3"];
  const { root } = await mount([first,refused], {}, [item]);
  assert.equal(root.all().filter(n => n.className === "mcp-server" && n.tag === "article").length, 0);
  assert.match(root.textContent, /Aucun connecteur ajouté/);
});

const pickAddress = root => {
  const radio = root.all().find(n => n.type === "radio" && n.getAttribute("aria-label") === "Adresse HTTPS");
  radio.checked = true;
  radio.events.change();
};

test("MCP by address submits the URL, the secret and the Mac confirmation, then empties the secret field", async () => {
  const { root, calls } = await mount();
  pickAddress(root);
  const field = name => root.all().find(n => n.name === name);
  const command = field("command"), url = field("url"), token = field("token");
  assert.equal(token.type, "password");
  assert.equal(command.required, false, "a command is not required for an address");
  assert.equal(url.required, true);
  field("name").value = "Serveur distant";
  url.value = "http://127.0.0.1:8080/mcp"; url.events.input();
  field("on-this-mac").checked = true;
  token.value = "jeton-factice-alpha";
  await root.all().find(n => n.tag === "form").events.submit({ preventDefault() {} });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/mcp/add");
  assert.deepEqual(calls[0].body, {name:"Serveur distant",url:"http://127.0.0.1:8080/mcp",token:"jeton-factice-alpha",on_this_mac:true});
  assert.equal(token.value, "", "the secret is not kept in the field after the request");
  const feedback = root.all().find(n => n.attrs.role === "status");
  assert.equal(feedback.className, "mcp-note");
  assert.match(feedback.textContent, /Serveur enregistré/);
  assert.ok(!root.textContent.includes("jeton-factice-alpha"));
});

test("the confirmation « sur ce Mac » only appears for an http address", async () => {
  const { root } = await mount();
  pickAddress(root);
  const url = root.all().find(n => n.name === "url");
  const confirmation = root.all().find(n => n.tag === "label" && n.textContent.includes("sur ce Mac"));
  assert.equal(confirmation.hidden, true);
  url.value = "https://exemple.fr/mcp"; url.events.input();
  assert.equal(confirmation.hidden, true);
  url.value = "http://127.0.0.1:8080/mcp"; url.events.input();
  assert.equal(confirmation.hidden, false);
});

test("a server added by address shows its address and a masked token, never a command", async () => {
  const server = {id:"distant",name:"Serveur distant",status:"disconnected",command:"",url:"https://exemple.fr/mcp",arguments:[],environment_names:["ACCESS_TOKEN"],enabled_bots:[],tools:[]};
  const { root } = await mount([server]);
  assert.match(root.textContent, /Adresse du serveur/);
  assert.ok(root.all().some(n => n.tag === "pre" && n.textContent === "https://exemple.fr/mcp"));
  assert.match(root.textContent, /Jeton privé conservé dans le coffre/);
  assert.ok(!root.textContent.includes("ACCESS_TOKEN"));
  assert.ok(root.all().some(n => n.tag === "button" && n.textContent === "Connecter…"));
});
