import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { floatingConversations, floatingReply, floatingPendingRequests, floatingHasMore, resizeFloatingInput } from "./floating-view.js";
import { floatingFailureState } from "./connection-view.js";

class Element {
  constructor(tag) { this.tag = tag; this.children = []; this.events = {}; this.value = "cmd+option+space"; }
  append(...children) { this.children.push(...children); }
  setAttribute(key, value) { this[key] = value; }
  addEventListener(name, callback) { this.events[name] = callback; }
}
const flush = () => new Promise((resolve) => setImmediate(resolve));
function fixture(status = "recording") {
  const requests = [], calls = [], root = new Element("div");
  let registration;
  const state = { floating: { shortcut: "cmd+option+space" } };
  const host = {
    state, register(value) { registration = value; value.settingsSection(root); },
    async api(path, body) { requests.push({ path, body }); return {}; },
    async refresh() { calls.push("refresh"); },
    async focusConversation(id) { calls.push(id); return id === "visible"; },
  };
  const window = { GrokbotPanels: host, ...(status === null ? {} : { webkit: { messageHandlers: { ezerbotFloating: { async postMessage(body) { calls.push(body); return { status }; } } } } }) };
  runInNewContext(readFileSync(new URL("../../ui/panels/floating.js", import.meta.url), "utf8"), { window, document: { createElement: (tag) => new Element(tag) } });
  const nodes = (element) => [element, ...element.children.flatMap(nodes)];
  const all = nodes(root);
  return { window, registration, root, calls, requests, select: all.find((n) => n.tag === "select"), save: all.find((n) => n.textContent === "Enregistrer le raccourci"), status: all.find((n) => n.className === "floating-settings-status") };
}

test("shortcut conflict is visible and never persisted", async () => {
  const f = fixture("conflict"); f.select.value = "cmd+option+j";
  await f.save.events.click();
  assert.match(f.status.textContent, /déjà utilisé/);
  assert.equal(f.requests.length, 0);
  assert.equal(f.save.disabled, false);
});

test("debug recording-only registration persists the chosen shortcut", async () => {
  const f = fixture(); f.select.value = "ctrl+option+space";
  await f.save.events.click();
  assert.equal(f.requests[0].path, "/api/floating/settings");
  assert.equal(f.requests[0].body.shortcut, "ctrl+option+space");
  assert.match(f.status.textContent, /Raccourci enregistré : ⌃ ⌥ Espace/);
  assert.match(f.status.textContent, /aucun raccourci global/);
  assert.equal(f.select.disabled, false);
});

test("saved shortcut confirmation keeps the browser-only scope explicit", async () => {
  const f = fixture(null); f.select.value = "cmd+option+j";
  await f.save.events.click();
  assert.match(f.status.textContent, /Raccourci enregistré : ⌘ ⌥ J/);
  assert.match(f.status.textContent, /navigateur/);
});

test("selection publication is ordered, deduplicated and excludes task/routine composers", async () => {
  const f = fixture();
  const state = { floating: { shortcut: "disabled" } };
  const context = (id, kind = "chat") => ({ connected: true, conversation: { id, kind }, bot: { id: "grokbot" } });
  f.registration.onState(state, context("first"));
  f.registration.onState(state, context("first"));
  f.registration.onState(state, context("second"));
  f.registration.onState(state, context("task", "task"));
  f.registration.onState(state, context("routine", "routine"));
  await flush();
  assert.deepEqual(f.requests.map((r) => r.body.conversation_id), ["first", "second"]);
});

test("open in Ezerbot refreshes the shared transcript before focusing the exact target", async () => {
  const f = fixture();
  assert.equal(await f.window.EzerbotFloatingMain.openConversation("visible"), true);
  assert.deepEqual(f.calls, ["refresh", "visible"]);
  await assert.rejects(() => f.window.EzerbotFloatingMain.openConversation("deleted"), /plus disponible/);
});

test("compact content uses only writable threads and the latest text reply", () => {
  const state = { conversations: [{kind:"chat"},{kind:"group"},{kind:"task"},{kind:"routine"}] };
  assert.deepEqual(floatingConversations(state).map(c=>c.kind),["chat","group"]);
  assert.equal(floatingReply({messages:[{role:"assistant",body:"ancien"},{role:"assistant",body:"réponse"},{role:"assistant",kind:"approval",body:"carte"}]}).body,"réponse");
});

test("pending approval follows its originating run even when routed to the coordinator", () => {
  const request = {id:"approval",run_id:"panel-run",status:{status:"pending"},route:{target_conversation_id:"coordinator-thread"}};
  const state = {forms:{requests:[request,{...request,id:"done",status:{status:"resolved"}},{...request,id:"other",run_id:"foreign-run"}]}};
  const result = floatingPendingRequests(state,{id:"chosen-thread",messages:[{payload:{run_id:"panel-run"}}]});
  assert.deepEqual(result,[request]);
  assert.deepEqual(floatingPendingRequests(state,null),[]);
});

test("while awaiting input the last actual text reply replaces the empty suspended snapshot", () => {
  const conversation = {messages:[{role:"assistant",kind:"message",body:"Dernière vraie réponse"},{role:"assistant",kind:"message",body:""}]};
  assert.equal(floatingReply(conversation).body, "");
  assert.equal(floatingReply(conversation, true).body, "Dernière vraie réponse");
});

test("compact composer grows to its content, caps at 120px and shrinks after sending", () => {
  const element = {style:{},scrollHeight:20};
  resizeFloatingInput(element); assert.equal(element.style.height,"40px");
  element.scrollHeight = 80; resizeFloatingInput(element); assert.equal(element.style.height,"80px");
  element.scrollHeight = 200; resizeFloatingInput(element); assert.equal(element.style.height,"120px"); assert.equal(element.style.overflowY,"auto");
  element.scrollHeight = 20; resizeFloatingInput(element); assert.equal(element.style.height,"40px"); assert.equal(element.style.overflowY,"hidden");
});

test("overflow hint disappears at the bottom and tolerates fractional pixel rounding", () => {
  assert.equal(floatingHasMore({scrollHeight:900,clientHeight:300,scrollTop:0}),true);
  assert.equal(floatingHasMore({scrollHeight:900,clientHeight:300,scrollTop:600}),false);
  assert.equal(floatingHasMore({scrollHeight:900,clientHeight:300,scrollTop:599.5}),false);
  assert.equal(floatingHasMore(null),false);
});

test("a reconnected account follows provider.connected instead of the stale sign-in kind", () => {
  const expired = { state: "failed", error_kind: "signin_expired", error: "Votre session ChatGPT a expiré. Reconnectez-vous via Codex." };
  const missing = { state: "failed", error_kind: "signin", error: "La connexion à ChatGPT n’est plus valide. Reconnectez-vous via Codex." };
  for (const reply of [expired, missing]) {
    const view = floatingFailureState(reply, { providerConnected: true });
    assert.equal(view.title, "Compte ChatGPT connecté");
    assert.equal(view.alert, false);
    assert.doesNotMatch(`${view.title} ${view.detail}`, /expiré|requise|Reconnectez/);
  }
  assert.equal(floatingFailureState(expired, { providerConnected: false }).title, "Votre session ChatGPT a expiré");
  assert.equal(floatingFailureState(missing).title, "Connexion ChatGPT requise");
  const other = floatingFailureState({ state: "failed", error_kind: "other", error: "Panne" }, { providerConnected: true });
  assert.equal(other.title, "La réponse a échoué.");
  assert.equal(other.alert, true);
  const source = readFileSync(new URL("./floating.jsx", import.meta.url), "utf8");
  assert.match(source, /provider\?\.connected === true/);
});

test("a floating detail stays neutral when the notice is not an alert", () => {
  const styles = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
  const css = readFileSync(new URL("./floating.css", import.meta.url), "utf8");
  const vars = Object.fromEntries([...styles.match(/:root\s*\{([^}]+)\}/)[1].matchAll(/(--[-\w]+)\s*:\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()]));
  const resolve = (value) => value.replace(/var\((--[-\w]+)\)/g, (_, name) => vars[name]);
  const notice = resolve(css.match(/\.floating-latest small, \.floating-notice\s*\{([^}]+)\}/)[1].match(/color:\s*([^;]+);/)[1].trim());
  const danger = resolve(css.match(/\.floating-error\s*\{([^}]+)\}/)[1].match(/color:\s*([^;]+);/)[1].trim());
  const expired = floatingFailureState({ state: "failed", error_kind: "signin_expired", error: "Votre session ChatGPT a expiré. Reconnectez-vous via Codex." });
  const failed = floatingFailureState({ state: "failed", error_kind: "other", error: "Panne" });
  const color = (view) => view.detailClass === "floating-error" ? danger : notice;
  assert.equal(expired.alert, false);
  assert.equal(expired.detailClass, "");
  assert.equal(color(expired), vars["--muted"]);
  assert.notEqual(color(expired), vars["--danger-text"]);
  assert.equal(failed.detailClass, "floating-error");
  assert.equal(color(failed), vars["--danger-text"]);
  const source = readFileSync(new URL("./floating.jsx", import.meta.url), "utf8");
  assert.match(source, /className=\{failure\.detailClass \|\| undefined\}/);
});

test("a floating panel without a session is not titled as a failed answer", () => {
  const expired = floatingFailureState({ state: "failed", error_kind: "signin_expired", error: "Votre session ChatGPT a expiré. Reconnectez-vous via Codex." });
  assert.equal(expired.alert, false);
  assert.equal(expired.title, "Votre session ChatGPT a expiré");
  assert.equal(expired.detail, "Votre session ChatGPT a expiré. Reconnectez-vous via Codex.");
  const missing = floatingFailureState({ state: "failed", error_kind: "signin", error: "La connexion à ChatGPT n’est plus valide. Reconnectez-vous via Codex." });
  assert.equal(missing.alert, false);
  assert.equal(missing.title, "Connexion ChatGPT requise");
  const failed = floatingFailureState({ state: "failed", error_kind: "other", error: "Panne" });
  assert.equal(failed.alert, true);
  assert.equal(failed.title, "La réponse a échoué.");
  assert.equal(failed.detail, "Panne");
  const ping = floatingFailureState({ state: "failed", error_kind: "other", error: "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez." });
  assert.equal(ping.alert, true);
  assert.equal(ping.detail, "OpenAI n’a envoyé aucun événement à temps. Réessayez.");
  assert.equal(floatingFailureState({ state: "echoed", error_kind: "signin" }), null);
  const source = readFileSync(new URL("./floating.jsx", import.meta.url), "utf8");
  assert.match(source, /floatingFailureState\(reply, \{ providerConnected: state\?\.provider\?\.connected === true \}\)/);
  assert.match(source, /failure\.alert \? " floating-failure" : ""/);
  assert.match(source, /<strong>\{failure\.title\}<\/strong>/);
});

test("compact layout keeps decisions outside the scroll region and disabled actions visibly muted", () => {
  const source = readFileSync(new URL("./floating.jsx", import.meta.url), "utf8");
  assert.match(source, /<\/div>\s*\{requests\.map\(\(request\) => <div className="floating-notice"/);
  assert.match(source, /floating-notice\$\{failure\.alert \? " floating-failure" : ""\}/);
  assert.match(source, /!inNativePanel && <button className="icon-button floating-close"/);
  assert.ok(!source.includes("reply?.body || reply?.error"));
  assert.match(source, /live && reply\?\.body && <small>/);
  const css = readFileSync(new URL("./floating.css", import.meta.url), "utf8");
  assert.match(css, /\.floating-composer \.primary-button:disabled \{ background: var\(--soft\); color: var\(--subtle\); filter: none;/);
  assert.match(css, /\.floating-panel footer button \{[^}]*margin-left: auto;/);
  assert.match(css, /\.setting-section > \.floating-settings-status \{ margin: 8px 0 0;/);
  assert.match(css, /-webkit-line-clamp: 2/);
  assert.ok(!/padding|border/.test(css.match(/\.floating-question \{([^}]+)\}/)[1]), "Clamp padding belongs outside the clipped text to avoid a partial third line");
  assert.match(css, /resize: none/);
});

test("native test builds compile out global registration and all event capture", () => {
  const source = readFileSync(new URL("../../native/floating.h", import.meta.url), "utf8");
  assert.match(source, /#if defined\(EZERBOT_RELEASE\)\n\s*\/\/ Carbon registers/);
  assert.ok(!/CGEventTapCreate|addGlobalMonitorForEvents|AXIsProcessTrusted|CGRequestListenEventAccess/.test(source));
  assert.match(source, /NSWindowCollectionBehaviorCanJoinAllSpaces/);
  assert.match(source, /initWithContentRect:NSMakeRect\(80, 80, 420, 560\)/);
  assert.match(source, /panel.minSize = NSMakeSize\(360, 520\)/);
});
