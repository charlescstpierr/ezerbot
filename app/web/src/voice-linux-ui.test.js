import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

class Element {
  constructor(tag) { this.tagName = tag; this.className = ""; this.children = []; this.dataset = {}; this.attributes = {}; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(key, value) { this.attributes[key] = String(value); }
}
const descendants = root => [root, ...root.children.flatMap(descendants)];
const flush = () => new Promise(resolve => setImmediate(resolve));
const phrase = "Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut.";
const listed = [{ id: "systeme", name: "Micro du système" }, { id: "usb", name: "Microphone USB (test)" }];

function fixture({ shell = { platform: "linux" }, route, chosen = "" } = {}) {
  const state = { voice: { microphone: chosen, drafts: [] } };
  const ctx = { state, conversation: { id: "chat" }, bot: { id: "grokbot" } };
  const calls = [], notices = [];
  let registration;
  const host = {
    register(panel) { registration = panel; },
    composerChanged() {},
    async api(path, body) { calls.push({ path, body }); return route(path, body); },
    async refresh() {},
    notify(text) { notices.push(text); },
  };
  const document = { documentElement: { lang: "fr" }, createElement: tag => new Element(tag), getElementById: () => null, createTextNode: text => Object.assign(new Element("text"), { textContent: text }) };
  const window = { GrokbotPanels: host, EzerbotShell: shell, addEventListener() {} };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice.js", import.meta.url), "utf8"), { window, document, crypto: { randomUUID: () => "token" }, setTimeout() {}, clearTimeout() {} });
  const section = new Element("div");
  registration.settingsSection(section, ctx);
  const options = () => descendants(section).filter(el => el.className.split(" ").includes("voice-microphone-option"));
  const texts = () => descendants(section).map(el => el.textContent).filter(Boolean);
  return { window, registration, ctx, calls, notices, options, texts, saved: () => calls.filter(call => call.path === "/api/voice/microphone").map(call => ({ ...call.body })) };
}

const route = (microphones) => async (path) => { if (path === "/api/voice/microphones") return { microphones }; return { ok: true }; };

test("the Linux shell lists the microphones from the wrapper route and labels the default row for the system", async () => {
  const f = fixture({ route: route(listed), chosen: "usb" });
  await flush();
  assert.deepEqual(f.calls.map(call => [call.path, call.body]), [["/api/voice/microphones", undefined]]);
  assert.deepEqual(f.options().map(el => el.dataset.id), ["", "systeme", "usb"]);
  assert.deepEqual(f.options().map(el => el.dataset.selected), ["false", "false", "true"]);
  assert.ok(f.texts().includes("Microphone par défaut du système"));
  assert.ok(!f.texts().includes("Microphone par défaut du Mac"));
  assert.deepEqual(f.saved(), []);
});

test("the Linux shell has no dictation controls and the panel survives state and dictation calls without them", async () => {
  const f = fixture({ route: route(listed) });
  await flush();
  assert.equal(f.registration.composer, undefined);
  f.registration.onState(f.ctx.state, f.ctx);
  f.window.EzerbotDictation.refresh();
  f.window.EzerbotDictation.cancel();
  assert.equal(f.window.EzerbotDictation.active(), false);
  assert.deepEqual(f.registration.sendExtras().attachments.length, 0);
});

test("a chosen microphone that disappeared is forgotten with the fallback phrase", async () => {
  const f = fixture({ route: route([listed[0]]), chosen: "usb" });
  await flush(); await flush();
  assert.ok(f.texts().includes(phrase));
  assert.deepEqual(f.saved(), [{ id: "" }]);
  assert.deepEqual([...f.notices], [phrase]);
});

test("an unreachable audio server keeps the choice and says the list is unavailable", async () => {
  const unavailable = Object.assign(new Error("Aucun serveur audio ne répond."), { status: 503 });
  const f = fixture({ chosen: "usb", route: async () => { throw unavailable; } });
  await flush(); await flush();
  assert.ok(f.texts().some(text => text.startsWith("La liste des microphones est indisponible")));
  assert.ok(!f.texts().includes(phrase));
  assert.deepEqual(f.saved(), []);
  assert.equal(f.notices.length, 0);
});

test("outside Linux the default row stays the Mac's and the route is never called", async () => {
  const f = fixture({ shell: null, route: route(listed) });
  await flush();
  assert.equal(f.calls.length, 0);
  assert.ok(f.texts().includes("Microphone par défaut du Mac"));
  assert.ok(f.texts().includes("La dictée est disponible dans l’application Mac Ezerbot."));
});

test("the unavailable notice of a first load made before the state arrived is cleared once the list loads", async () => {
  const f = fixture({ shell: null, route: async () => ({ microphones: listed, fake: true }) });
  assert.ok(f.texts().includes("La dictée est disponible dans l’application Mac Ezerbot."));
  f.ctx.state.voice.debug_fake = true;
  f.registration.onState(f.ctx.state, f.ctx);
  await flush();
  assert.deepEqual(f.options().map(el => el.dataset.id), ["", "systeme", "usb"]);
  assert.ok(!f.texts().includes("La dictée est disponible dans l’application Mac Ezerbot."));
});
