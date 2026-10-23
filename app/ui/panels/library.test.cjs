// Run with: node --test app/ui/panels/library.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag) {
    this.tagName = tag; this.children = []; this.attrs = {}; this.events = {}; this.hidden = false; this.className = "";
    this.classList = { add() {}, remove() {} };
    this.files = [];
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener(k, v) { this.events[k] = v; }
  click() {}
  querySelectorAll(selector) {
    const matches = (e) => selector.startsWith(".") ? e.className?.split(" ").includes(selector.slice(1)) : e.tagName === selector;
    return this.children.filter((c) => c instanceof Element).flatMap((c) => [...(matches(c) ? [c] : []), ...c.querySelectorAll(selector)]);
  }
  set textContent(text) { this.text = String(text); this.children = []; }
  get textContent() { return (this.text ?? "") + this.children.map((c) => c instanceof Element ? c.textContent : c).join(" "); }
}

const flush = () => new Promise((resolve) => setImmediate(resolve));
const file = (index) => ({ id: `f${index}`, name: `fichier-${index}.pdf`, format: "pdf", byte_length: 2048, added_at: 1790000000000 });

function harness(data, { upload } = {}) {
  const container = new Element("div");
  const calls = [];
  let panel;
  const panels = {
    api: async (path, body) => { calls.push([path, body]); return path === "/api/library" && body ? upload(body) : data; },
    notify() {},
    register(p) { panel = p; },
  };
  const document = { createElement: (tag) => new Element(tag) };
  const source = fs.readFileSync(`${__dirname}/library.js`, "utf8");
  vm.runInNewContext(source, { window: { GrokbotPanels: panels }, document, btoa: (s) => Buffer.from(s, "binary").toString("base64"), Uint8Array, Number, String, Date, Array, Math, Intl });
  const cleanup = panel.botSection({ id: "bot-a" }, container);
  return { container, calls, cleanup };
}

const text = (h) => h.container.textContent;

test("the panel states the per-file ceiling and the maximum number of files from the catalogue", async () => {
  const h = harness({ ceiling: "20 Mo", max_files: 12, files: [file(1), file(2)] });
  assert.match(text(h), /Plafond : 20 Mo par fichier · 0 sur 20 fichiers\./);
  await flush();
  assert.match(text(h), /Plafond : 20 Mo par fichier · 2 sur 12 fichiers\./);
  assert.match(text(h), /fichier-1\.pdf/);
  assert.match(text(h), /avec son nom et la page/);
  h.cleanup();
});

test("a full library refuses a new file before sending it, and says why", async () => {
  const files = Array.from({ length: 20 }, (_, index) => file(index));
  const h = harness({ ceiling: "20 Mo", max_files: 20, files });
  await flush();
  assert.match(text(h), /20 sur 20 fichiers/);
  const input = h.container.querySelectorAll("input")[0];
  input.files = [{ name: "nouveau.pdf", size: 10, arrayBuffer: async () => new ArrayBuffer(10) }];
  await input.events.change();
  assert.match(h.container.querySelectorAll(".library-error")[0].textContent, /20 fichiers au maximum/);
  assert.equal(h.calls.filter(([path, body]) => path === "/api/library" && body).length, 0);
  h.cleanup();
});

test("a file with an existing name still replaces when the library is full", async () => {
  const files = Array.from({ length: 20 }, (_, index) => file(index));
  const h = harness({ ceiling: "20 Mo", max_files: 20, files }, { upload: async () => ({ ceiling: "20 Mo", max_files: 20, files }) });
  await flush();
  const input = h.container.querySelectorAll("input")[0];
  input.files = [{ name: "fichier-3.pdf", size: 10, arrayBuffer: async () => new ArrayBuffer(10) }];
  await input.events.change();
  assert.equal(h.calls.filter(([path, body]) => path === "/api/library" && body).length, 1);
  assert.match(h.container.querySelectorAll(".library-status")[0].textContent, /remplace la version précédente/);
  h.cleanup();
});
