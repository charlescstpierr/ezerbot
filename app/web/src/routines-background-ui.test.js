import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute() {}
  get textContent() { return (this.text || "") + this.children.map(n => n.textContent).join(""); }
  set textContent(value) { this.text = value; this.children = []; }
}
const all = node => [node, ...node.children.flatMap(all)];
function fixture(data) {
  const section = new Element("section"), calls = [];
  let registration, closed = false, opened = false, target;
  const host = { state: { routines_background: data }, register(r) { registration = r; r.settingsSection(section); }, async api(path, body) { calls.push([path, body]); return { conversation_id: "approvals", approval_message_id: "approval-target" }; }, async refresh() {}, openConversation(id, messageId) { opened = id; target = messageId; }, notify() {} };
  runInNewContext(readFileSync(new URL("../../ui/panels/routines-background.js", import.meta.url), "utf8"), { window: { GrokbotPanels: host }, document: { createElement: tag => new Element(tag), getElementById: () => ({ close() { closed = true; } }) } });
  return { section, host, calls, input: () => all(section).find(n => n.tagName === "input"), update(data) { host.state.routines_background = data; registration.onState(host.state); }, closed: () => closed, opened: () => opened, target: () => target };
}
test("off by default; switch queues the normal approval conversation, never optimistic installation", async () => {
  const f = fixture({ available: true, enabled: false, installed: false });
  assert.equal(f.input().checked, false);
  assert.match(f.section.textContent, /Exécuter les routines quand Ezerbot est fermé/);
  f.input().checked = true; await f.input().onchange();
  assert.equal(f.calls[0][0], "/api/routines/background");
  assert.equal(f.calls[0][1].enabled, true);
  assert.equal(f.input().checked, false);
  assert.equal(f.closed(), true); assert.equal(f.opened(), "approvals");
  assert.equal(f.target(), "approval-target");
});
test("persisted installation, last execution and errors are projected accessibly", () => {
  const f = fixture({ available: true, enabled: true, installed: true, last_run: Date.now(), error: "Échec simulé" });
  assert.equal(f.input().checked, true); assert.match(f.section.textContent, /Agent : installé/);
  assert.doesNotMatch(f.section.textContent, /Désactivé par défaut/);
  assert.match(f.section.className, /routines-background-settings/);
  assert.equal(all(f.section).find(n => n.role === "status").className, "routines-agent-status");
  assert.match(f.section.textContent, /Dernière exécution :/);
  assert.equal(all(f.section).find(n => n.role === "alert").textContent, "Échec simulé");
  f.update({ available: true, enabled: false, installed: false });
  assert.equal(f.input().checked, false); assert.match(f.section.textContent, /Agent : non installé/);
});
test("debug safety refusal disables switch and a failed request preserves saved preference", async () => {
  const f = fixture({ available: false, unavailable_reason: "Installation système désactivée." });
  assert.equal(f.input().disabled, true);
  assert.match(f.section.textContent, /Installation système désactivée/);
  f.update({ available: true, enabled: false });
  f.host.api = async () => { throw new Error("Refus de configuration"); };
  f.input().checked = true; await f.input().onchange();
  assert.equal(f.input().checked, false); assert.match(f.section.textContent, /Refus de configuration/);
});
test("the agent setting says its first pass after the wake catches up the slots due during the outage", () => {
  const f = fixture({ available: true, enabled: true, installed: true });
  assert.match(f.section.textContent, /À son premier passage après le réveil, l’agent rattrape les créneaux dus pendant l’extinction ou la veille du Mac, dans l’ordre, même si Ezerbot est resté fermé/);
  assert.doesNotMatch(f.section.textContent, /même fenêtre de rattrapage/);
});
