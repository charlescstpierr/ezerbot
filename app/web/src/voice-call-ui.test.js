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
  const limits = [];
  const calls = [], notices = [], events = [], listeners = {};
  let source = { status: "listening", lines: [], transcript: "", speaking: false };
  let lineGate = null;
  const held = [], holding = new Set();
  let script, microphone = "fake-usb";
  let startStatus = "listening", mode = false, dictation = false, meeting = false;
  const root = new Element("div"), send = new Element("button"), actions = new Element("div"); actions.className = "topbar-actions";
  actions.insertBefore = (child, reference) => { child.parent = actions; actions.children.splice(reference ? actions.children.indexOf(reference) : actions.children.length, 0, child); };
  root.append(actions, send);
  const state = { voice: { debug_fake: true, bots: { ezerbot: { voice: "fake-fr-CA", speed: 1.2, language: "fr", chosen: true } } }, voice_call: { available, debug_fake: true, active: null } };
  let context = { connected: true, bot: { id: "ezerbot", name: "Ezerbot" }, conversation: { id: "chat", messages: [] }, state };
  const bridge = async body => {
    calls.push({ debug: body.command, ...body });
    const action = script?.(body) ?? (holding.has(body.command) ? "hold" : "");
    if (action === "reject") throw new Error("refus");
    if (action !== "hold-reject") {
      if (body.command === "start") source = { status: startStatus, lines: [], transcript: "", speaking: false, muted: false, token: body.token };
      if (body.command === "mute" || body.command === "unmute") { source.muted = body.command === "mute"; if (source.muted) source.transcript = ""; }
      if (body.command === "interrupt") source.speaking = false;
      if (body.command === "cancel") { source.status = "idle"; source.muted = false; }
    }
    const reply = { ...source };
    if (action.startsWith("hold")) await new Promise(resolve => held.push({ command: body.command, resolve }));
    if (action === "hold-reject") throw new Error("refus");
    return reply;
  };
  const panels = {
    register(p) { panel = p; p.composer.mount(root, context); p.onState(state, context); },
    composerChanged() {}, notify(text) { notices.push(text); }, async refresh() { calls.push({ refresh: true }); },
    async api(path, body) {
      if (path === "/api/voice/call/debug") return bridge(body);
      calls.push({ path, ...body });
      if (path === "/api/voice/call/line" && lineGate) await lineGate();
      if (path === "/api/voice/call/start") return { call: { id: "call_one" }, secret: SECRET };
      return {};
    },
    meetingActive: () => meeting,
  };
  const window = {
    GrokbotPanels: panels, GrokbotChat: { voiceNeedsAction }, EzerbotIPhone: remote ? { remote: true } : undefined,
    EzerbotMicrophone: { current: () => microphone, fellBack(id) { notices.push("repli:" + id); } },
    EzerbotVoiceMode: { active: () => mode }, EzerbotDictation: { active: () => dictation, refresh() {} },
    addEventListener(name, fn) { listeners[name] = fn; }, dispatchEvent(event) { events.push(event.type); },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice-call.js", import.meta.url), "utf8"), {
    window,
    document: { createElement: tag => new Element(tag), getElementById: () => send, querySelector: () => actions, addEventListener(name, fn) { if (name === "keydown") keydown = fn; } },
    Event: class { constructor(type) { this.type = type; } },
    setTimeout(fn, ms) { if (ms === 100) { timer = fn; return 1; } limits.push(fn); return limits.length + 1; },
    clearTimeout(id) { if (id > 1) limits[id - 2] = null; else timer = null; },
  });
  return {
    root, calls, notices, events, window, get registered() { return Boolean(panel); }, get panel() { return panel; },
    get source() { return source; }, set startStatus(v) { startStatus = v; }, set lineGate(v) { lineGate = v; }, state,
    set mode(v) { mode = v; panel.onState(state, context); }, set dictation(v) { dictation = v; panel.onState(state, context); }, set meeting(v) { meeting = v; panel.onState(state, context); },
    get context() { return context; }, setContext(c) { context = c; panel.onState(state, c); },
    tick: async () => { await timer?.(); }, startTick: () => timer?.(),
    expire() { limits.forEach((fn, i) => { limits[i] = null; fn?.(); }); },
    hold(...commands) { holding.clear(); for (const command of commands) holding.add(command); },
    release(...commands) { if (!commands.length) holding.clear(); for (const late of held.splice(0)) if (!commands.length || commands.includes(late.command)) late.resolve(); else held.push(late); },
    set script(fn) { script = fn; }, set microphone(id) { microphone = id; }, keydown: e => keydown(e),
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

test("quitting while a line is still being written submits every line exactly once, in order", async () => {
  const f = fixture();
  await f.toggle().onclick();
  let release;
  f.lineGate = () => new Promise(resolve => { release = resolve; });
  f.source.lines = [{ role: "user", text: "Première." }];
  const polling = f.tick();
  await new Promise(r => setImmediate(r));
  f.source.lines = [{ role: "user", text: "Première." }, { role: "assistant", text: "Seconde." }];
  const quitting = all(f.root).find(x => x.textContent === "Quitter").onclick();
  await new Promise(r => setImmediate(r));
  f.lineGate = null; release();
  await polling; await quitting;
  assert.deepEqual(f.calls.filter(c => c.path === "/api/voice/call/line").map(c => [c.seq, c.text]), [[0, "Première."], [1, "Seconde."]]);
  assert.equal(f.calls.filter(c => c.path === "/api/voice/call/end").length, 1);
});

test("a call left open by a reload is ended once, so the next call can start", async () => {
  const f = fixture();
  f.state.voice_call.active = { id: "call_old", conversation_id: "chat" };
  f.setContext(f.context);
  f.setContext(f.context);
  await new Promise(r => setImmediate(r));
  assert.deepEqual(f.calls.filter(c => c.path === "/api/voice/call/end").map(c => c.id), ["call_old"]);
  assert.ok(f.calls.some(c => c.refresh), "le fil est relu pour montrer le transcript écrit");
});

const muteButton = f => cls(f.root, "voice-mute-toggle"), stateOf = f => cls(f.root, "voice-call-state").textContent;
const shortcut = (f, extra = {}) => {
  const event = { key: "M", metaKey: true, shiftKey: true, prevented: false, preventDefault() { this.prevented = true; }, stopImmediatePropagation() {}, ...extra };
  f.keydown(event);
  return new Promise(r => setImmediate(() => r(event)));
};

test("the mute button sends mute, says the bot cannot hear and empties the live line; a second click listens again", async () => {
  const f = fixture();
  await f.toggle().onclick();
  const mute = muteButton(f);
  assert.equal(mute.attributes["aria-label"], "Couper le micro");
  cls(f.root, "voice-call-transcript-toggle").onclick();
  f.source.transcript = "Bonjour"; await f.tick();
  assert.match(textOf(cls(f.root, "voice-call-transcript")), /Bonjour/);
  await mute.onclick();
  assert.equal(f.calls.at(-1).debug, "mute"); assert.equal(f.calls.at(-1).token, "call_one");
  assert.equal(stateOf(f), "Micro coupé · le bot ne vous entend pas");
  assert.equal([mute.attributes["aria-pressed"], mute.attributes["aria-label"]].join(), "true,Réactiver le micro");
  assert.doesNotMatch(textOf(cls(f.root, "voice-call-transcript")), /Bonjour/);
  await f.tick(); assert.equal(stateOf(f), "Micro coupé · le bot ne vous entend pas", "l’écran suit l’état du pont à chaque sondage");
  await mute.onclick();
  assert.equal(f.calls.at(-1).debug, "unmute"); assert.match(stateOf(f), /^Appel en cours/);
  assert.equal(mute.attributes["aria-pressed"], "false");
});

test("⌘⇧M toggles the microphone during a call and does nothing without one", async () => {
  const f = fixture();
  let event = await shortcut(f);
  assert.equal(event.prevented, false); assert.equal(f.calls.filter(c => c.debug === "mute").length, 0);
  await f.toggle().onclick();
  event = await shortcut(f); assert.equal(event.prevented, true); assert.equal(f.source.muted, true);
  await shortcut(f); assert.equal(f.source.muted, false);
  for (const extra of [{ shiftKey: false }, { metaKey: false }, { altKey: true }, { ctrlKey: true }, { key: "N" }]) await shortcut(f, extra);
  assert.equal(f.calls.filter(c => c.debug === "mute" || c.debug === "unmute").length, 2, "seulement ⌘⇧M");
});

test("quitting the call and calling again listens: the mute never sticks", async () => {
  const f = fixture();
  await f.toggle().onclick(); await muteButton(f).onclick(); assert.equal(f.source.muted, true);
  await all(f.root).find(x => x.textContent === "Quitter").onclick();
  await f.toggle().onclick();
  assert.equal(f.source.muted, false); assert.equal(muteButton(f).attributes["aria-pressed"], "false"); assert.match(stateOf(f), /^Appel en cours/);
  await f.tick(); assert.match(stateOf(f), /^Appel en cours/);
});

test("a microphone gone while muted is announced when the microphone is turned back on", async () => {
  const f = fixture();
  await f.toggle().onclick(); f.source.microphone_fallback = true;
  await muteButton(f).onclick();
  assert.deepEqual(f.notices, [], "muted, nothing is listened to");
  await muteButton(f).onclick();
  assert.deepEqual(f.notices, ["repli:fake-usb"]);
});

test("an older bridge answer cannot undo a newer unmute: the screen never says muted while the bridge listens", async () => {
  const f = fixture();
  await f.toggle().onclick(); await muteButton(f).onclick();
  assert.equal(muteButton(f).attributes["aria-pressed"], "true");
  f.hold("status"); const late = f.startTick(); f.hold();
  await muteButton(f).onclick(); assert.equal(f.source.muted, false); assert.equal(muteButton(f).attributes["aria-pressed"], "false");
  f.release(); await late;
  assert.equal(muteButton(f).attributes["aria-pressed"], "false", "l’état lu pendant la sourdine est plus ancien que la réactivation");
  assert.match(stateOf(f), /^Appel en cours/);
});

test("a gesture while a mute command is in flight is ignored: one command, and the screen follows its answer", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.hold("mute", "unmute"); const click = muteButton(f).onclick(); const key = shortcut(f);
  f.release(); await click; await key;
  assert.deepEqual(f.calls.map(c => c.debug).filter(c => c === "mute" || c === "unmute"), ["mute"]);
  assert.equal(f.source.muted, true); assert.equal(muteButton(f).attributes["aria-pressed"], "true"); assert.equal(stateOf(f), "Micro coupé · le bot ne vous entend pas");
});
test("a card waiting for a click and a muted microphone are both said", async () => {
  const f = fixture();
  await f.toggle().onclick();
  f.context.conversation.messages.push({ id: "approval", kind: "approval", payload: { status: { status: "pending" } } });
  await f.tick(); assert.equal(stateOf(f), "En attente de vous · répondez dans la carte");
  await muteButton(f).onclick();
  assert.match(stateOf(f), /carte/); assert.match(stateOf(f), /micro coupé/i); assert.equal(muteButton(f).attributes["aria-pressed"], "true");
  await f.tick(); assert.match(stateOf(f), /carte/); assert.match(stateOf(f), /micro coupé/i);
  await muteButton(f).onclick(); assert.equal(stateOf(f), "En attente de vous · répondez dans la carte");
});

const settle = () => new Promise(r => setImmediate(r));
function* orders(items) { if (items.length < 2) { yield items; return; } for (const [i, x] of items.entries()) for (const rest of orders(items.filter((_, j) => j !== i))) yield [x, ...rest]; }
test("whatever the outcome, lateness and order of the bridge answers, the screen ends on the bridge's muted and the next gesture sends the right command", async () => {
  let runs = 0;
  for (const startMuted of [false, true]) for (const mode of ["", "hold", "reject", "hold-reject"]) for (const tick of ["", "before", "during"]) for (const gesture of ["", "click", "key"]) for (const second of ["", "reject"]) for (const order of orders([...tick ? ["status"] : [], ...mode.startsWith("hold") ? ["command"] : []])) {
    const f = fixture(), name = JSON.stringify({ startMuted, mode, tick, gesture, second, order });
    await f.toggle().onclick(); if (startMuted) await muteButton(f).onclick();
    let sent = 0, reads = 0;
    f.script = body => body.command === "status" ? (++reads === 1 && tick ? "hold" : "") : body.command === "mute" || body.command === "unmute" ? (++sent === 1 ? mode : second) : "";
    const pending = [];
    if (tick === "before") pending.push(f.startTick());
    pending.push(muteButton(f).onclick());
    if (gesture === "click") pending.push(muteButton(f).onclick());
    if (gesture === "key") pending.push(shortcut(f));
    if (tick === "during") pending.push(f.startTick());
    await settle();
    for (const late of order) { f.release(...late === "status" ? ["status"] : ["mute", "unmute"]); await settle(); }
    f.release(); await Promise.all(pending); await settle();
    const muted = f.source.muted;
    assert.equal(muteButton(f).attributes["aria-pressed"], String(muted), name);
    assert.match(stateOf(f), muted ? /^Micro coupé/ : /^Appel en cours/, name);
    f.script = undefined; const before = f.calls.length; await muteButton(f).onclick();
    assert.equal(f.calls[before].debug, muted ? "unmute" : "mute", name);
    assert.equal(f.source.muted, !muted, name); assert.equal(muteButton(f).attributes["aria-pressed"], String(!muted), name);
    runs++;
  }
  assert.ok(runs > 150, String(runs));
});

test("a mute answered after the call ended changes nothing in the next call", async () => {
  for (const mode of ["hold", "hold-reject"]) {
    const f = fixture(); await f.toggle().onclick();
    f.script = body => body.command === "mute" ? mode : ""; const click = muteButton(f).onclick();
    await all(f.root).find(x => x.textContent === "Quitter").onclick(); await f.toggle().onclick();
    f.release(); await click; await settle();
    assert.equal(muteButton(f).attributes["aria-pressed"], "false", mode); assert.match(stateOf(f), /^Appel en cours/);
    f.script = undefined; await muteButton(f).onclick(); assert.equal(f.calls.at(-1).debug, "mute", mode); assert.equal(f.source.muted, true);
  }
});

test("a status that carries the microphone fallback announces it, once, for the microphone asked at the start", async () => {
  const f = fixture(); f.microphone = "usb-A"; await f.toggle().onclick();
  await muteButton(f).onclick(); f.source.microphone_fallback = true;
  f.microphone = "usb-B";
  f.hold("unmute"); const unmute = muteButton(f).onclick(); await f.tick(); f.release(); await unmute; await f.tick();
  assert.deepEqual(f.notices, ["repli:usb-A"], "the choice saved meanwhile is not the one that was missing");
});

test("a command whose answer never comes ends at its limit: the screen follows the bridge again and the gestures come back", async () => {
  const f = fixture(); await f.toggle().onclick(); await muteButton(f).onclick();
  f.hold("unmute"); const lost = muteButton(f).onclick(); await settle();
  assert.equal(f.source.muted, false, "the bridge did unmute; its answer is withheld");
  for (let i = 0; i < 3; i++) await f.tick();
  assert.equal(muteButton(f).attributes["aria-pressed"], "true", "inside the limit the command is still waited for");
  f.expire();
  for (let i = 0; i < 10; i++) await f.tick();
  assert.equal(muteButton(f).attributes["aria-pressed"], "false"); assert.match(stateOf(f), /^Appel en cours/);
  await muteButton(f).onclick(); assert.equal(f.calls.at(-1).debug, "mute"); assert.equal(f.source.muted, true);
  f.release(); await lost; await settle();
  assert.equal(muteButton(f).attributes["aria-pressed"], "true", "the abandoned answer says nothing any more");
});

test("the microphone fallback is announced even when the answer that carries it is an error", async () => {
  for (const carrier of ["command", "status"]) {
    const f = fixture(); f.microphone = "usb-A"; await f.toggle().onclick(); await muteButton(f).onclick();
    f.source.microphone_fallback = true; f.source.status = "mic_denied";
    if (carrier === "command") await muteButton(f).onclick(); else { f.source.muted = false; await f.tick(); }
    assert.deepEqual(f.notices, ["repli:usb-A"], carrier);
  }
});
