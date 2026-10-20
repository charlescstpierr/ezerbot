import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { floatingConversations, floatingReply, floatingPendingRequests } from "./floating-view.js";

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
  const window = { GrokbotPanels: host, webkit: { messageHandlers: { ezerbotFloating: { async postMessage(body) { calls.push(body); return { status }; } } } } };
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
  assert.match(f.status.textContent, /aucun raccourci global/);
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

test("native test builds compile out global registration and all event capture", () => {
  const source = readFileSync(new URL("../../native/floating.h", import.meta.url), "utf8");
  assert.match(source, /#if defined\(EZERBOT_RELEASE\)\n\s*\/\/ Carbon registers/);
  assert.ok(!/CGEventTapCreate|addGlobalMonitorForEvents|AXIsProcessTrusted|CGRequestListenEventAccess/.test(source));
  assert.match(source, /NSWindowCollectionBehaviorCanJoinAllSpaces/);
});
