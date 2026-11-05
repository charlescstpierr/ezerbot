const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const { runInNewContext } = require("node:vm");

const source = readFileSync(`${__dirname}/app.js`, "utf8");
const registration = source.slice(source.indexOf("    register(panel) {"), source.indexOf("    openConversation(id, messageId) {"));
const availability = source.slice(source.indexOf("  const unavailablePanels ="), source.indexOf("  const panelFailed ="));
const unavailable = ["voice", "voice-conversation", "voice-call", "meetings", "demonstrations", "floating"];

function registry(shell) {
  const mounted = [];
  const notices = [];
  const panels = new Map();
  const rows = ["Nouvelle conversation", "Dicter un message"].map((text) => ({
    textContent: text, hidden: false, key: { textContent: "⌘ N" },
    querySelector() { return this.key; },
  }));
  const paragraph = { textContent: "Les conversations sont conservées sur ce Mac." };
  const nodes = {
    "settings-extensions": { before(node) { notices.push(node.textContent); } },
    "settings-shortcuts": { querySelectorAll() { return rows; } },
    "bot-dialog": { open: false },
  };
  const context = {
    window: { EzerbotShell: shell }, panels, state: {},
    document: { createElement() { return {}; }, querySelectorAll() { return [paragraph]; } },
    $: (id) => nodes[id],
    panelError() { throw new Error("Invalid registration"); },
    mountSection(panel) { mounted.push(panel.id); }, panelCall() {}, updateComposer() {},
    syncSidebarRail() {}, schedulePoll() {},
  };
  runInNewContext(`${availability}\nglobalThis.registry = {${registration}};`, context);
  return { register: context.registry.register, panels, mounted, notices, rows, paragraph };
}

test("Linux hides unavailable native controls before mounting, while ordinary panels still mount", () => {
  const app = registry({ platform: "linux", unavailablePanels: unavailable });
  for (const id of unavailable) assert.equal(app.register({ id }), false, id);
  for (const id of ["attachments", "forms", "engine"]) assert.equal(app.register({ id }), true, id);
  assert.deepEqual(app.mounted, ["attachments", "forms", "engine"]);
  assert.equal(app.notices.length, 1);
  assert.match(app.notices[0], /Indisponibles sous Linux/);
  assert.match(app.notices[0], /contrôle de l’écran/);
  assert.doesNotMatch(app.notices[0], /capture/);
  assert.equal(app.rows[1].hidden, true);
  assert.equal(app.rows[0].key.textContent, "Ctrl N");
  assert.match(app.paragraph.textContent, /sur cet ordinateur/);
});

test("macOS without a Linux capability object keeps its native panels and shortcuts", () => {
  const app = registry();
  for (const id of unavailable) assert.equal(app.register({ id }), true, id);
  assert.deepEqual(app.mounted, unavailable);
  assert.deepEqual(app.notices, []);
  assert.equal(app.rows[1].hidden, false);
  assert.equal(app.rows[0].key.textContent, "⌘ N");
  assert.match(app.paragraph.textContent, /sur ce Mac/);
});

function ratingsContext({ shell, webkit, read = async () => ({ value: "[]" }) }) {
  const calls = [];
  const rating = source.slice(source.indexOf("  async function openRatings() {"), source.indexOf("  let pendingRating = null;"));
  const context = {
    window: {
      EzerbotShell: shell, webkit,
      GrokbotChat: {
        createNativeRatingTable: async (bridge) => { await bridge.postMessage({ command: "read" }); return { table: "durable", bridge }; },
        createRatingTable: () => ({ table: "browser" }),
      },
    },
    localStorage: {},
    api: async (path, body) => { calls.push([path, body]); return body ? { ok: true } : read(); },
  };
  runInNewContext(`${rating}\nglobalThis.opened = async () => { await ratingsReady(); return ratings; };`, context);
  return { opened: context.opened, calls };
}

test("Linux routes ratings to the wrapper, whose storage outlives the shell's origin", async () => {
  const { opened, calls } = ratingsContext({ shell: { platform: "linux" } });
  const { table, bridge } = await opened();
  assert.equal(table, "durable");
  await bridge.postMessage({ command: "write", value: "[1]" });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [["/api/ratings", null], ["/api/ratings", { value: "[1]" }]]);
});

test("a failed first ratings read is retried instead of disabling ratings for the session", async () => {
  let reads = 0;
  const { opened } = ratingsContext({
    shell: { platform: "linux" },
    read: async () => { reads += 1; if (reads === 1) throw new Error("moteur indisponible"); return { value: "[]" }; },
  });
  const unavailable = await opened();
  assert.throws(() => unavailable.recordGood(), /Notation indisponible/);
  assert.equal((await opened()).table, "durable");
  assert.equal(reads, 2);
});

test("macOS keeps its native ratings bridge and a plain browser its local table", async () => {
  const native = { postMessage: async () => "[]" };
  const mac = await ratingsContext({ webkit: { messageHandlers: { ezerbotRatings: native } } }).opened();
  assert.equal(mac.bridge, native);
  assert.equal((await ratingsContext({}).opened()).table, "browser");
});

test("Linux demonstration module does not create controls before registration", () => {
  const script = readFileSync(`${__dirname}/panels/demonstrations.js`, "utf8");
  runInNewContext(script, {
    window: { EzerbotShell: { platform: "linux", unavailablePanels: unavailable } },
    document: new Proxy({}, { get() { throw new Error("demonstrations touched the document"); } }),
  });
});

class Element {
  constructor(tag) { Object.assign(this, { tag, children: [], attributes: {}, className: "", textContent: "", classList: { toggle() {} }, hidden: false, open: false }); }
  append(...nodes) { this.children.push(...nodes); }
  prepend(...nodes) { this.children.unshift(...nodes); }
  after(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(name, value) { this.attributes[name] = value; }
  removeAttribute(name) { delete this.attributes[name]; }
  matches() { return false; }
  showModal() { this.open = true; }
  close() { this.open = false; }
  get text() { return [this.textContent, ...this.children.map(child => child.text)].join(" "); }
  buttons() { return [...(this.tag === "button" ? [this] : []), ...this.children.flatMap(child => child.buttons())]; }
}

function screenPanel(shell) {
  const nodes = new Map();
  const select = key => { if (!nodes.has(key)) nodes.set(key, new Element("div")); return nodes.get(key); };
  const registered = [];
  const posted = [];
  const host = { state: { bots: [{ id: "b", name: "Bot" }] }, register(panel) { registered.push(panel); }, notify() {}, api: async (path, body) => { posted.push([path, body]); }, refresh: async () => {} };
  const context = {
    window: { EzerbotShell: shell, GrokbotPanels: host, addEventListener() {} },
    document: { createElement: tag => new Element(tag), querySelector: select, getElementById: select, addEventListener() {} },
    HTMLDialogElement: class {}, setTimeout() {}, clearTimeout() {}, Date,
  };
  runInNewContext(readFileSync(`${__dirname}/panels/screen.js`, "utf8"), context);
  const [panel] = registered;
  const settings = new Element("section");
  panel.settingsSection(settings);
  const conversation = { conversation: { id: "c", kind: "chat", title: "Fil" }, bot: { id: "b" } };
  const open = (session = null) => {
    panel.onState({ screen: { bots: [{ bot_id: "b", enabled: true }], session } }, conversation);
    const entry = select(".topbar-actions").children[0];
    entry.onclick();
    return { entry, dialog: select("panels").children[0], banner: select(".topbar").children[0] };
  };
  return { panel, settings, open, posted };
}

test("the Linux share panel names what it does and promises nothing it cannot keep", () => {
  const { panel, settings, open } = screenPanel({ platform: "linux", unavailablePanels: unavailable });
  assert.equal(panel.id, "screen");
  const { entry, dialog } = open();
  assert.equal(entry.textContent, "Partage d’écran");
  const consent = dialog.text;
  assert.match(consent, /voit l’écran et n’effectue aucun geste/);
  for (const text of [consent, settings.text, dialog.buttons().map(button => button.text).join(" ")]) {
    assert.doesNotMatch(text, /Mac|Réglages Système|Échap|Accessibilité/);
  }
  assert.ok(!settings.buttons().length || settings.buttons().every(button => !/Ouvrir/.test(button.text)));
});

test("a Linux session whose banner is not proven never claims the bot sees the screen", () => {
  const { open } = screenPanel({ platform: "linux", unavailablePanels: unavailable });
  const session = { id: 9, bot_id: "b", conversation_id: "c", expires_at: Date.now() + 60000, paused: true, unproven: true };
  const { banner, dialog } = open(session);
  for (const text of [banner.text, dialog.text]) {
    assert.match(text, /Partage en pause : bandeau non garanti visible/);
    assert.doesNotMatch(text, /voit l’écran/);
  }
  assert.ok(dialog.buttons().some(button => button.text.includes("Reprendre")));
  assert.ok(dialog.buttons().some(button => button.text.includes("Révoquer")));
});

test("a Linux session the person paused says so without blaming the banner", () => {
  const { open } = screenPanel({ platform: "linux", unavailablePanels: unavailable });
  const session = { id: 9, bot_id: "b", conversation_id: "c", expires_at: Date.now() + 60000, paused: true, unproven: false };
  const { banner } = open(session);
  assert.match(banner.text, /Ezerbot en pause/);
  assert.doesNotMatch(banner.text, /bandeau/);
});

test("the Linux consent card names both sessions where the share stays paused", () => {
  const { open } = screenPanel({ platform: "linux", unavailablePanels: unavailable });
  assert.match(open().dialog.text, /Avec un compositeur.*reste en pause\. Sous Wayland, il n’est pas encore possible/);
});

test("a live Linux session shows what the bot does and keeps the stop control", () => {
  const { open, posted } = screenPanel({ platform: "linux", unavailablePanels: unavailable });
  const session = { id: 9, bot_id: "b", conversation_id: "c", expires_at: Date.now() + 60000, paused: false };
  const { banner, dialog } = open(session);
  assert.match(banner.text, /Ezerbot voit l’écran/);
  assert.doesNotMatch(banner.text + dialog.text, /Mac|Échap|contrôle/);
  dialog.buttons().find(button => button.text.includes("Révoquer")).onclick();
  assert.deepEqual(JSON.parse(JSON.stringify(posted)), [["/api/screen/revoke", { id: 9 }]]);
});

test("macOS keeps its panel wording", () => {
  const { settings, open } = screenPanel();
  const { entry, dialog } = open();
  assert.equal(entry.textContent, "Écran du Mac");
  assert.match(dialog.text, /Échap arrête tout/);
  assert.match(settings.text, /Réglages Système/);
});
