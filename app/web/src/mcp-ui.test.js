import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

// Execute the settings panel against a small DOM boundary. Assertions concern
// rendered controls and actual submitted API bodies, never implementation text.
class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.events = {}; this.attrs = {}; this.value = ""; this.className = ""; this.text = ""; }
  set textContent(value) { this.text = String(value); this.children = []; }
  get textContent() { return this.text + this.children.map(n => n.textContent).join(""); }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(key, value) { this.attrs[key] = value; }
  addEventListener(name, action) { this.events[name] = action; }
  all() { return [this, ...this.children.flatMap(n => n.all())]; }
  querySelectorAll(selector) { return this.all().filter(n => selector === "input" ? n.tag === "input" : selector === 'input[type="password"]' && n.tag === "input" && n.type === "password"); }
  reset() { for (const n of this.all()) n.value = ""; }
  focus() { this.focused = true; }
  requestSubmit() { return this.events.submit({preventDefault(){}}); }
}
async function mount(servers = [], actionResult = {}, catalogue = []) {
  const root = new Node("section"), calls = [], notifications = [], opened = [], copies = [];
  const dialog = { open: true, close() { this.open = false; } };
  let panel;
  const panels = {
    state: { bots: [{ id: "grokbot", name: "Ezerbot" }], mcp: { servers, catalogue } },
    register(value) { panel = value; },
    async api(path, body) { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); return actionResult; },
    async copyText(text) { copies.push(text); },
    async refresh() {}, notify(text) { notifications.push(text); },
    openConversation(id) { opened.push(id); return true; },
  };
  const document = { getElementById: id => id === "settings-dialog" ? dialog : undefined, createElement: tag => new Node(tag), createTextNode: text => { const n = new Node("#text"); n.textContent = text; return n; } };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document });
  vm.runInContext(await readFile(new URL("../../ui/panels/mcp.js", import.meta.url), "utf8"), context);
  panel.settingsSection(root);
  return { root, calls, notifications, opened, dialog, copies };
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


const service = { id: "gmail", name: "Gmail", status: "disconnected", description: "Courriels", permissions: "Lecture et envoi", scopes: "gmail.modify", setup: "Client OAuth de bureau", license: "MIT", source: "https://example.test", needs_client: true };

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
  const { root, calls } = await mount([server], {}, [{...service,status:"connected",server}]);
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
    const {root}=await mount([server],{},[{...service,server,status,account:"factice@example.test"}]);
    assert.equal(root.all().find(n=>n.type==="checkbox").checked,true);
    assert.ok(root.textContent.includes("Outils (1)"));
    assert.ok(root.textContent.includes("Compte connecté : factice@example.test"));
    const action=root.all().find(n=>n.attrs["data-connector-action"]==="connect");
    if (status==="connected" || status==="saved") assert.equal(action.className,"secondary-button");
  }
});

test("revocation warning has an action tone and public setup values can be copied", async () => {
  const redirect="http://127.0.0.1:52187/oauth/slack/callback";
  const {root,copies}=await mount([],{},[{...service,id:"slack",name:"Slack",warning:true,message:"Révocation distante non confirmée.",redirect_uri:redirect}]);
  assert.equal(root.all().find(n=>n.textContent==="Révocation distante non confirmée.").className,"mcp-warning");
  const copy=root.all().find(n=>n.attrs["aria-label"]==="Copier url de retour oauth — Slack");
  await copy.events.click();
  assert.deepEqual(copies,[redirect]);assert.equal(copy.textContent,"Copié");
});
