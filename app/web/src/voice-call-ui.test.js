import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test, { after } from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";
import { voiceNeedsAction } from "./voice-conversation.js";

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.className = ""; this.dataset = {}; this.attributes = {}; this.textContent = ""; this.scrollHeight = 300; this.scrollTop = 0; }
  append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  before(child) { this.parent.append(child); }
  setAttribute(k, v) { this.attributes[k] = String(v); }
}
const all = n => [n, ...n.children.flatMap(all)];
const cls = (n, c) => all(n).find(x => x.className.split(" ").includes(c));
const textOf = n => all(n).map(x => x.textContent).join(" ");
// A made-up ephemeral secret, assembled so no scanner takes it for a real key.
const SECRET = ["ezc", "fake", "never", "visible", "0123456789"].join("_");

function fixture({ available = true, remote = false } = {}) {
  let panel, timer, keydown;
  const calls = [], notices = [], events = [], listeners = {};
  let source = { status: "listening", lines: [], transcript: "", speaking: false };
  let startStatus = "listening", mode = false, dictation = false, meeting = false;
  const root = new Element("div"), send = new Element("button"), actions = new Element("div"); actions.className = "topbar-actions";
  actions.insertBefore = (child, reference) => { child.parent = actions; actions.children.splice(reference ? actions.children.indexOf(reference) : actions.children.length, 0, child); };
  root.append(actions, send);
  const state = { voice: { debug_fake: true, bots: { ezerbot: { voice: "fake-fr-CA", speed: 1.2, language: "fr", chosen: true } } }, voice_call: { available, debug_fake: true, active: null } };
  let context = { connected: true, bot: { id: "ezerbot", name: "Ezerbot" }, conversation: { id: "chat", messages: [] }, state };
  const bridge = async body => {
    calls.push({ debug: body.command, ...body });
    if (body.command === "start") source = { status: startStatus, lines: [], transcript: "", speaking: false, token: body.token };
    if (body.command === "interrupt") source.speaking = false;
    if (body.command === "cancel") source.status = "idle";
    return { ...source };
  };
  const panels = {
    register(p) { panel = p; p.composer.mount(root, context); p.onState(state, context); },
    composerChanged() {}, notify(text) { notices.push(text); }, async refresh() { calls.push({ refresh: true }); },
    async api(path, body) {
      if (path === "/api/voice/call/debug") return bridge(body);
      calls.push({ path, ...body });
      if (path === "/api/voice/call/start") return { call: { id: "call_one" }, secret: SECRET };
      return {};
    },
    meetingActive: () => meeting,
  };
  const window = {
    GrokbotPanels: panels, GrokbotChat: { voiceNeedsAction }, EzerbotIPhone: remote ? { remote: true } : undefined,
    EzerbotMicrophone: { current: () => "fake-usb", fellBack() {} },
    EzerbotVoiceMode: { active: () => mode }, EzerbotDictation: { active: () => dictation, refresh() {} },
    addEventListener(name, fn) { listeners[name] = fn; }, dispatchEvent(event) { events.push(event.type); },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice-call.js", import.meta.url), "utf8"), {
    window,
    document: { createElement: tag => new Element(tag), getElementById: () => send, querySelector: () => actions, addEventListener(name, fn) { if (name === "keydown") keydown = fn; } },
    Event: class { constructor(type) { this.type = type; } },
    setTimeout(fn) { timer = fn; return 1; }, clearTimeout() { timer = null; },
  });
  return {
    root, calls, notices, events, window, get registered() { return Boolean(panel); }, get panel() { return panel; },
    get source() { return source; }, set startStatus(v) { startStatus = v; },
    set mode(v) { mode = v; panel.onState(state, context); }, set dictation(v) { dictation = v; panel.onState(state, context); }, set meeting(v) { meeting = v; panel.onState(state, context); },
    get context() { return context; }, setContext(c) { context = c; panel.onState(state, c); },
    tick: async () => { await timer?.(); }, keydown: e => keydown(e),
    toggle: () => cls(root, "voice-call-toggle"), listeners,
  };
}

test("the call is a separate button that says the audio is sent to the provider and not kept", async () => {
  const f = fixture();
  assert.equal(f.toggle().attributes["aria-label"], "Appel vocal");
  assert.equal(f.toggle().parent.className, "topbar-actions", "dans l’en-tête, pas dans la barre du compositeur");
  await f.toggle().onclick();
  const bar = cls(f.root, "voice-call-bar");
  assert.equal(bar.hidden, false);
  assert.equal(cls(f.root, "voice-call-notice").textContent, "L’audio de cet appel est transmis au fournisseur de voix le temps de l’appel. Il n’est pas gardé sur le Mac.");
  assert.doesNotMatch(textOf(f.root), /reste sur ce Mac/, "le mode voix garde sa phrase, pas l’appel");
});

test("the call opens with the chosen microphone and the voice of the bot, and the secret is never displayed", async () => {
  const f = fixture();
  await f.toggle().onclick();
  const start = f.calls.find(c => c.debug === "start");
  assert.equal(start.token, "call_one");
  assert.equal(start.microphone, "fake-usb");
  assert.deepEqual([start.voice, start.speed, start.language], ["fake-fr-CA", 1.2, "fr"]);
  assert.equal(start.secret, SECRET, "le fournisseur reçoit le secret une fois");
  f.source.lines = [{ role: "user", text: "Bonjour" }];
  await f.tick();
  assert.ok(!textOf(f.root).includes(SECRET));
  assert.ok(!f.notices.join(" ").includes(SECRET));
  assert.ok(!JSON.stringify(f.calls.filter(c => c.path && c.path !== "/api/voice/call/start")).includes(SECRET), "le secret n’est jamais renvoyé au Mac");
});

test("the transcript fills during the call, hides and shows, and each line is written to the Mac once", async () => {
  const f = fixture();
  await f.toggle().onclick();
  const toggle = cls(f.root, "voice-call-transcript-toggle"), transcript = cls(f.root, "voice-call-transcript");
  assert.equal(toggle.textContent, "Afficher le transcript");
  assert.equal(transcript.hidden, true);
  f.source.lines = [{ role: "user", text: "Bonjour" }, { role: "assistant", text: "Je vous écoute." }];
  await f.tick();
  toggle.onclick();
  assert.equal(toggle.textContent, "Masquer le transcript");
  assert.equal(transcript.hidden, false);
  assert.match(textOf(transcript), /Bonjour/);
  assert.match(textOf(transcript), /Je vous écoute\./);
  toggle.onclick();
  assert.equal(transcript.hidden, true);
  await f.tick();
  assert.deepEqual(f.calls.filter(c => c.path === "/api/voice/call/line").map(c => [c.seq, c.role, c.text]), [[0, "user", "Bonjour"], [1, "assistant", "Je vous écoute."]]);
});

test("quitting stops the session, writes the last lines, ends the call and refreshes the thread", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.source.lines = [{ role: "user", text: "Dernière phrase." }];
  await all(f.root).find(x => x.textContent === "Quitter").onclick();
  const order = f.calls.filter(c => c.debug === "cancel" || c.path === "/api/voice/call/end" || c.path === "/api/voice/call/line" || c.refresh).map(c => c.debug || c.path || "refresh");
  assert.deepEqual(order, ["/api/voice/call/line", "cancel", "/api/voice/call/end", "refresh"]);
  assert.equal(cls(f.root, "voice-call-bar").hidden, true);
  assert.equal(f.window.EzerbotVoiceCall.active(), false);
  assert.equal(f.panel.wantsPolling(), false);
});

test("Escape and the cut button stop the bot's voice and nothing else", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.source.speaking = true; await f.tick();
  assert.equal(cls(f.root, "voice-call-state").textContent.includes("Échap"), true);
  let prevented = false;
  f.keydown({ key: "Escape", preventDefault() { prevented = true; }, stopImmediatePropagation() {} });
  assert.equal(prevented, true);
  await new Promise(r => setImmediate(r));
  assert.equal(f.source.speaking, false);
  assert.equal(f.calls.filter(c => c.debug === "interrupt").length, 1);
  assert.equal(f.window.EzerbotVoiceCall.active(), true, "Échap ne quitte pas l’appel");
});

test("a card waiting for a click holds the voice and the call resumes once it is decided", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.context.conversation.messages.push({ id: "card", kind: "approval", payload: { status: { status: "pending" } } });
  await f.tick();
  assert.equal(f.calls.filter(c => c.debug === "pause").length, 1);
  assert.match(cls(f.root, "voice-call-state").textContent, /carte/);
  await f.tick();
  assert.equal(f.calls.filter(c => c.debug === "pause").length, 1, "le sondage ne redemande pas la pause");
  f.context.conversation.messages[0].payload.status.status = "allowed";
  await f.tick();
  assert.equal(f.calls.filter(c => c.debug === "resume").length, 1);
});

test("the call and the other microphone gestures refuse each other", async () => {
  const f = fixture();
  f.mode = true;
  assert.equal(f.toggle().disabled, true);
  f.mode = false; f.dictation = true;
  assert.equal(f.toggle().disabled, true);
  f.dictation = false; f.meeting = true;
  assert.equal(f.toggle().disabled, true);
  f.meeting = false;
  assert.equal(f.toggle().disabled, false);
  f.mode = true; await f.toggle().onclick();
  assert.equal(f.calls.filter(c => c.path === "/api/voice/call/start").length, 0, "rien n’est demandé au Mac");
  f.mode = false;
  await f.toggle().onclick();
  assert.equal(f.window.EzerbotVoiceCall.active(), true);
  assert.ok(f.events.includes("grokbot:voice-call-change"), "les autres gestes sont prévenus");
});

test("a provider that holds the microphone or refuses the secret ends the call at once, in French", async () => {
  const f = fixture();
  f.startStatus = "microphone_busy";
  await f.toggle().onclick();
  assert.equal(cls(f.root, "voice-call-state").textContent, "Une autre capture utilise déjà le microphone. Arrêtez-la avant de continuer.");
  assert.equal(f.calls.filter(c => c.path === "/api/voice/call/end").length, 1, "l’appel ouvert côté Mac est refermé");
  assert.equal(f.window.EzerbotVoiceCall.active(), false);
  cls(f.root, "voice-call-state").parent.children.find(c => c.textContent === "Fermer").onclick();
  f.startStatus = "secret_refused";
  await f.toggle().onclick();
  assert.match(cls(f.root, "voice-call-state").textContent, /refusé d’ouvrir l’appel/);
});

test("without a provider the button is disabled and says why, and a conversation is needed", async () => {
  const f = fixture({ available: false });
  assert.equal(f.toggle().disabled, true);
  assert.match(f.toggle().title, /aucun fournisseur de voix/);
  const g = fixture();
  g.setContext({ ...g.context, conversation: null });
  assert.equal(g.toggle().disabled, true);
  assert.match(g.toggle().title, /Ouvrez une conversation/);
});

test("the phone has no call button", () => {
  const f = fixture({ remote: true });
  assert.equal(f.registered, false);
});

test("switching conversation ends the call and keeps the lines already written", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.source.lines = [{ role: "user", text: "Salut" }];
  f.setContext({ ...f.context, conversation: { id: "other", messages: [] } });
  await new Promise(r => setImmediate(r)); await new Promise(r => setImmediate(r));
  assert.ok(f.calls.some(c => c.path === "/api/voice/call/end"));
  assert.deepEqual(f.calls.filter(c => c.path === "/api/voice/call/line").map(c => c.text), ["Salut"]);
});

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error", optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/voice-call.jsx");

test("the thread card is marked Appel vocal, keeps the transcript and offers no audio", () => {
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "ezerbot", name: "Ezerbot" }] } } };
  try {
    const html = renderToStaticMarkup(React.createElement(Card, { message: { id: "call_one", payload: { label: "Appel vocal", bot_id: "ezerbot", status: "ended", lines: [{ role: "user", text: "Bonjour" }, { role: "assistant", text: "Je vous écoute." }] } } }));
    assert.match(html, /Appel vocal/);
    assert.match(html, /Appel avec Ezerbot/);
    assert.match(html, /Bonjour/);
    assert.match(html, /Je vous écoute\./);
    assert.doesNotMatch(html, /<audio|audio_url|\.wav/);
    assert.doesNotMatch(html, /interrompu/);
    const cut = renderToStaticMarkup(React.createElement(Card, { message: { id: "c", payload: { status: "interrupted", lines: [] } } }));
    assert.match(cut, /Appel interrompu/);
    assert.match(cut, /Aucune parole/);
  } finally { delete globalThis.window; }
});
