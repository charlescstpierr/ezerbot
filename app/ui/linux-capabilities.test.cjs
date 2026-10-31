const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const { runInNewContext } = require("node:vm");

const source = readFileSync(`${__dirname}/app.js`, "utf8");
const registration = source.slice(source.indexOf("    register(panel) {"), source.indexOf("    openConversation(id, messageId) {"));
const availability = source.slice(source.indexOf("  const unavailablePanels ="), source.indexOf("  const panelFailed ="));
const unavailable = ["voice", "voice-conversation", "voice-call", "meetings", "screen", "demonstrations", "floating"];

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

test("Linux screen and demonstration modules do not create controls before registration", () => {
  for (const panel of ["screen", "demonstrations"]) {
    const script = readFileSync(`${__dirname}/panels/${panel}.js`, "utf8");
    runInNewContext(script, {
      window: { EzerbotShell: { platform: "linux", unavailablePanels: unavailable } },
      document: new Proxy({}, { get() { throw new Error(`${panel} touched the document`); } }),
    });
  }
});
