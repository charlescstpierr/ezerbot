import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { createDocumentIcon, documentIconPaths, formatAttachmentSize, documentBudgetMessage } from "./attachment-presentation.js";

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.attrs = {}; this.listeners = {}; this.hidden = false; }
  append(...children) { for (const child of children) { child.remove(); child.parent = this; this.children.push(child); } }
  insertBefore(child, before) { child.remove(); child.parent = this; const index = this.children.indexOf(before); this.children.splice(index < 0 ? this.children.length : index, 0, child); }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter((item) => item !== this); this.parent = null; }
  set textContent(value) { this.text = String(value); this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this.attrs[name] = value; }
  removeAttribute(name) { delete this.attrs[name]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { this.listeners.click?.(); }
}
const descendants = (element) => [element, ...element.children.flatMap(descendants)];
const byClass = (element, className) => descendants(element).find((item) => item.className === className);
const allByClass = (element, className) => descendants(element).filter((item) => item.className === className);
const flush = () => new Promise((resolve) => setImmediate(resolve));

function panel(response) {
  const root = new Element("div");
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_ns, tag) => new Element(tag), getElementById: () => root };
  const ctx = { conversation: { id: "chat", messages: [] }, bot: { id: "bot" }, state: { preferences: {}, provider: {}, attachments: { document_character_limit: 200000, document_payload_limit: 300000 } } };
  let registration;
  const requests = [], notifications = [];
  const panels = { register(value) { registration = value; }, composerChanged() {}, draft: () => "", notify: (text) => notifications.push(text) };
  runInNewContext(readFileSync(new URL("../../ui/panels/attachments.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels, GrokbotChat: { formatAttachmentSize, documentBudgetMessage, createDocumentIcon: () => createDocumentIcon(document) }, addEventListener() {} },
    document, File, Uint8Array, btoa, AbortController, TypeError, SyntaxError, DOMException,
    setTimeout: () => 1, clearTimeout() {}, MutationObserver: class { observe() {} },
    URL: { createObjectURL: () => "blob:test", revokeObjectURL() {} },
    fetch: async (url, options) => {
      requests.push({ url, options });
      const ordinal = requests.length;
      const value = await response?.(ordinal) || {};
      if (value.error instanceof Error) throw value.error;
      const status = value.status || 200;
      return { status, ok: status >= 200 && status < 300, json: async () => status >= 400 ? { error: value.message || "Ce PDF est chiffré." } : {
        id: `document_${ordinal.toString(16).padStart(64, "0")}`, kind: "document", byte_length: 54, page_label: "1 page estimée", text_characters: 54, context_characters: 1000, ...value.metadata,
      } };
    },
  });
  registration.composer.mount(root, ctx);
  return { root, ctx, requests, notifications, registration, add: (files) => registration.composer.onFiles(files, null, ctx), blocked: () => registration.composer.blocked(ctx) };
}

test("shared attachment sizes use French o / Ko / Mo formatting", () => {
  for (const [bytes, expected] of [[0, "0 o"], [54, "54 o"], [502, "502 o"], [1024, "1 Ko"], [1536, "1,5 Ko"], [34918, "34,1 Ko"], [1024 * 1024, "1 Mo"], [21 * 1024 * 1024, "21 Mo"]]) assert.equal(formatAttachmentSize(bytes), expected);
});

test("composer renders the shared document outline", async () => {
  const p = panel(); p.add([new File(["bonjour"], "notes.txt")]); await flush();
  const frame = byClass(p.root, "attachment-document-icon");
  const svg = frame.children[0];
  assert.equal(svg.tagName, "svg");
  assert.equal(svg.attrs.width, "22"); assert.equal(svg.attrs.height, "26");
  assert.deepEqual(svg.children.map((path) => path.attrs.d), documentIconPaths);
});

test("empty and oversized files leave persistent, terminal refusal chips without reading or uploading", () => {
  const p = panel();
  p.add([{ name: "vide.txt", size: 0, type: "text/plain" }, { name: "énorme.pdf", size: 21 * 1024 * 1024, type: "application/pdf" }]);
  assert.equal(p.requests.length, 0);
  const chips = allByClass(p.root, "attachment-chip"); assert.equal(chips.length, 2);
  assert.match(chips[0].textContent, /vide.txt est vide/);
  assert.match(chips[1].textContent, /limite de 20 Mo \(21 Mo\)/);
  assert.ok(chips.every((chip) => byClass(chip, "attachment-retry").hidden));
  assert.equal(p.blocked(), true);
  chips.forEach((chip) => byClass(chip, "attachment-remove").click());
  assert.equal(p.blocked(), false);
});

test("validation refusals hide Retry; transient HTTP failures and native network failures retain it", async () => {
  for (const status of [400, 403, 413, 415, 422]) {
    const p = panel(() => ({ status })); p.add([new File(["pdf"], "test.pdf")]); await flush();
    assert.equal(byClass(p.root, "attachment-retry").hidden, true, `HTTP ${status}`);
    assert.equal(p.blocked(), true);
  }
  for (const failure of [{ status: 500 }, { status: 408 }, { status: 429 }, { error: new TypeError("Failed to fetch") }]) {
    const p = panel((n) => n === 1 ? failure : {}); p.add([new File(["texte"], "test.txt")]); await flush();
    assert.equal(byClass(p.root, "attachment-retry").hidden, false);
    assert.equal(p.blocked(), true);
    byClass(p.root, "attachment-retry").click(); await flush();
    assert.equal(p.requests.length, 2);
    assert.equal(p.blocked(), false);
    assert.equal(byClass(p.root, "attachment-retry").hidden, true);
  }
});

test("truncation and assumed encoding have a warning marker, not success-only styling", async () => {
  for (const metadata of [{ truncated: true, note: "Texte tronqué : 100\u202f000 caractères." }, { encoding_note: "Encodage Windows-1252 supposé.", note: "Encodage Windows-1252 supposé." }]) {
    const p = panel(() => ({ metadata })); p.add([new File(["texte"], "notes.txt")]); await flush();
    assert.equal(byClass(p.root, "attachment-chip").dataset.warning, "true");
    assert.equal(byClass(p.root, "attachment-status").textContent, metadata.note);
  }
});

test("combined-document limits keep a persistent explanation and unblock after files are removed", async () => {
  const p = panel(() => ({ metadata: { text_characters: 100000, context_characters: 101000 } }));
  p.add([1, 2, 3, 4].map((n) => new File(["texte"], `annexe-${n}.md`))); await flush();
  const notice = byClass(p.root, "attachment-limit");
  assert.equal(notice.hidden, false); assert.match(notice.textContent, /200\u202f000 caractères/);
  assert.equal(p.blocked(), true);
  allByClass(p.root, "attachment-remove").slice(0, 2).forEach((button) => button.click());
  assert.equal(notice.hidden, true); assert.equal(p.blocked(), false);
  p.ctx.state.attachments.document_character_limit = 24000;
  p.registration.onState(p.ctx.state, p.ctx);
  assert.equal(notice.hidden, false); assert.equal(p.blocked(), true);
});

test("escaped-document volume also blocks a single text that would overflow its context", async () => {
  const p = panel(() => ({ metadata: { text_characters: 70000, context_characters: 421000 } }));
  p.add([new File(["texte"], "angles.txt")]); await flush();
  assert.equal(p.blocked(), true);
  assert.equal(byClass(p.root, "attachment-limit").hidden, false);
});

test("removing an in-flight document aborts its request and ignores a late completion", async () => {
  let release;
  const p = panel(() => new Promise((resolve) => { release = resolve; }));
  p.add([new File(["texte"], "retiré.txt")]); await flush();
  assert.equal(p.requests.length, 1);
  assert.equal(p.blocked(), true);
  byClass(p.root, "attachment-remove").click();
  assert.equal(p.requests[0].options.signal.aborted, true);
  assert.equal(p.blocked(), false);
  release({}); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 0);
  assert.equal(p.registration.sendExtras(p.ctx).attachments.length, 0);
  assert.equal(p.requests.length, 1);
});
