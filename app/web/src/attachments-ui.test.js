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

function panel(response, welcome = false) {
  const root = new Element("div");
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_ns, tag) => new Element(tag), getElementById: () => root };
  const ctx = { conversation: { id: "chat", messages: [] }, bot: { id: "bot" }, state: { conversations: [{ id: "chat", bot_id: "bot", messages: [] }], preferences: {}, provider: {}, attachments: { document_character_limit: 200000, document_payload_limit: 300000 } } };
  let registration;
  const requests = [], notifications = [];
  let uploadNonce = 0;
  let timerNonce = 0;
  const timers = new Map();
  const storage = new Map();
  const richDocument = {type:"doc",content:[{type:"paragraph",content:[{type:"botMention",attrs:{id:"grokbot",label:"Ezerbot"}},{type:"text",text:"Mon brouillon"}]}]};
  const panels = { register(value) { registration = value; }, composerChanged() {}, draft: () => welcome ? "Mon brouillon" : "", draftDocument: () => richDocument, notify: (text) => notifications.push(text),
    async api() {return {conversation:{id:"created-chat"}};}, async refresh() {},
    async ensureDraftConversation() {
      const conversation = {id: "created-chat", messages: []};
      storage.set(`grokbot.draft.${conversation.id}`, "Mon brouillon");
      storage.set(`grokbot.draft.${conversation.id}.document`, JSON.stringify(richDocument));
      registration.composer.transfer({from: "new:bot", to: conversation.id}, {...ctx, conversation});
      panels.openConversation(conversation.id);
      storage.delete("grokbot.draft.new");
      storage.delete("grokbot.draft.new.document");
      return conversation;
    },
    openConversation(id) {
      storage.set("grokbot.draft.new", "Mon brouillon");
      storage.set("grokbot.draft.new.document", JSON.stringify(richDocument));
      assert.equal(storage.get(`grokbot.draft.${id}.document`), JSON.stringify(richDocument));
      ctx.conversation={id,messages:[]}; registration.onState(ctx.state,ctx); return true;
    },
  };
  if (welcome) ctx.conversation = null;
  runInNewContext(readFileSync(new URL("../../ui/panels/attachments.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels, GrokbotChat: { formatAttachmentSize, documentBudgetMessage, createDocumentIcon: () => createDocumentIcon(document) }, addEventListener() {} },
    document, File, Uint8Array, btoa, AbortController, TypeError, SyntaxError, DOMException,
    localStorage: {setItem(key,value){storage.set(key,String(value));},removeItem(key){storage.delete(key);}},
    crypto: { randomUUID: () => `upload-${++uploadNonce}` },
    setTimeout: (callback, delay) => { const id = ++timerNonce; timers.set(id, {callback, delay}); return id; }, clearTimeout: (id) => timers.delete(id), MutationObserver: class { observe() {} },
    URL: { createObjectURL: () => "blob:test", revokeObjectURL() {} },
    fetch: async (url, options) => {
      requests.push({ url, options });
      const ordinal = requests.length;
      const value = await response?.(ordinal, url, options) || {};
      if (value.error instanceof Error) throw value.error;
      const status = value.status || 200;
      return { status, ok: status >= 200 && status < 300, json: async () => status >= 400 ? { error: value.message || "Ce PDF est chiffré." } : {
        id: `document_${ordinal.toString(16).padStart(64, "0")}`, kind: "document", byte_length: 54, page_label: "1 page estimée", text_characters: 54, context_characters: 1000, ...value.metadata,
      } };
    },
  });
  registration.composer.mount(root, ctx);
  return { root, ctx, requests, notifications, registration, storage, runTimer: (delay) => { const next = [...timers].find(([,timer]) => timer.delay === delay); assert.ok(next, `timer ${delay}`); timers.delete(next[0]); next[1].callback(); }, add: (files) => registration.composer.onFiles(files, null, ctx), blocked: () => registration.composer.blocked(ctx) };
}

test("welcome attachment handoff preserves rich text and clears both home draft keys", async () => {
  const p = panel(undefined, true);
  p.add([new File(["texte"], "notes.txt")]); await flush();
  assert.equal(p.ctx.conversation.id,"created-chat");
  assert.equal(p.requests.length,1);
  assert.ok(p.storage.has("grokbot.draft.created-chat.document"));
  assert.equal([...p.storage.keys()].some(key=>key.startsWith("grokbot.draft.new")),false);
});

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
  const p = panel((n) => n === 1 ? new Promise((resolve) => { release = resolve; }) : {});
  p.add([new File(["texte"], "retiré.txt")]); await flush();
  assert.equal(p.requests.length, 1);
  assert.equal(p.blocked(), true);
  byClass(p.root, "attachment-remove").click();
  assert.equal(p.requests[0].options.signal.aborted, true);
  assert.equal(p.blocked(), false);
  assert.equal(p.requests.length, 2);
  assert.equal(p.requests[1].url, "/api/storage/drafts");
  assert.deepEqual(JSON.parse(p.requests[1].options.body).cancelled_uploads, ["upload-1"]);
  assert.equal(p.requests[1].options.keepalive, true);
  release({}); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 0);
  assert.equal(p.registration.sendExtras(p.ctx).attachments.length, 0);
  // A late completion repeats the durable cancellation, never revives the chip.
  assert.equal(p.requests.length, 3);
  assert.equal(p.requests[2].url, "/api/storage/drafts");
  assert.deepEqual(JSON.parse(p.requests[2].options.body).cancelled_uploads, ["upload-1"]);
});

test("uploads carry durable draft identity, a stable token and the original image name", async () => {
  const id = `attachment_${"a".repeat(64)}`;
  const p = panel(() => ({ metadata: { id } }));
  p.add([new File(["image"], "schéma été.png", { type: "image/png" })]); await flush();
  const query = new URL(p.requests[0].url, "http://localhost").searchParams;
  assert.equal(query.get("draft_id"), "chat");
  assert.equal(query.get("upload_id"), "upload-1");
  assert.equal(query.get("name"), "schéma été.png");
  assert.equal(p.registration.sendExtras(p.ctx).attachments[0], id);
  const documents = panel(); documents.add([new File(["texte"], "notes.txt")]); await flush();
  const body = JSON.parse(documents.requests[0].options.body);
  assert.equal(body.draft_id, "chat"); assert.equal(body.upload_id, "upload-1"); assert.equal(body.name, "notes.txt");
});

test("state restores named image and document drafts once, without resurrecting a removed piece", async () => {
  const p = panel();
  const image = { id: `attachment_${"b".repeat(64)}`, name: "schéma conservé.png", url: "/api/attachments/image", byte_length: 42 };
  const document = { id: `document_${"c".repeat(64)}`, kind: "document", name: "notes.txt", byte_length: 54, text_characters: 54, context_characters: 1000 };
  p.ctx.state.storage = { drafts: { chat: [image, document] } };
  p.registration.onState(p.ctx.state, p.ctx);
  const chips = allByClass(p.root, "attachment-chip");
  assert.equal(chips.length, 2); assert.match(chips[0].textContent, /schéma conservé.png/); assert.match(chips[1].textContent, /notes.txt/);
  assert.deepEqual(Array.from(p.registration.sendExtras(p.ctx).attachments), [image.id, document.id]);
  byClass(chips[0], "attachment-remove").click(); await flush();
  const removal = JSON.parse(p.requests[0].options.body);
  assert.deepEqual(removal.removed, [image.id]);
  p.registration.onState(p.ctx.state, p.ctx);
  assert.equal(allByClass(p.root, "attachment-chip").length, 1);
});

test("discarding a finished task's attachment draft aborts pending uploads and clears its tray", async () => {
  let release;
  const p = panel(() => new Promise((resolve) => { release = resolve; }));
  p.add([new File(["texte"], "brouillon.txt")]); await flush();
  p.registration.composer.discard(p.ctx);
  assert.equal(p.requests[0].options.signal.aborted, true);
  assert.equal(p.registration.sendExtras(p.ctx).attachments.length, 0);
  release({}); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 0);
});

for (const kind of ["image", "document"]) for (const acknowledgeBeforeUpload of [false, true]) {
  test(`T13 ${kind} reselected during send survives old acknowledgement, upload first ${!acknowledgeBeforeUpload}`, async () => {
    const id = `${kind === "image" ? "attachment" : "document"}_${"d".repeat(64)}`;
    let complete;
    let uploads = 0;
    const p = panel((_n, url) => {
      if (url === "/api/storage/drafts") return {};
      uploads++;
      if (uploads === 2 && acknowledgeBeforeUpload) return new Promise(resolve => { complete = resolve; });
      return {metadata: {id}};
    });
    const file = () => new File(["sample"], kind === "image" ? "sample.png" : "sample.txt", {type: kind === "image" ? "image/png" : "text/plain"});
    p.add([file()]); await flush();
    const extras = p.registration.sendExtras(p.ctx);
    byClass(p.root, "attachment-remove").click(); await flush();
    p.add([file()]); await flush();
    p.ctx.state.conversations[0].messages.push({id: "old-send"});
    p.registration.composer.onSent({messageId: "old-send", extras}, p.ctx);
    if (complete) { complete({metadata: {id}}); await flush(); }
    assert.equal(allByClass(p.root, "attachment-chip").length, 1);
    assert.deepEqual(Array.from(p.registration.sendExtras(p.ctx).attachments), [id]);
    assert.deepEqual(JSON.parse(JSON.stringify(p.registration.sendExtras(p.ctx).attachment_selections)), {[id]: ["upload-2"]});
  });
}

test("T14 acknowledgement from another thread consumes only captured attachment occurrences", async () => {
  const p = panel();
  p.add([new File(["first"], "first.txt")]); await flush();
  const extras = p.registration.sendExtras(p.ctx);
  p.add([new File(["next"], "next.txt")]); await flush();
  const next = p.registration.sendExtras(p.ctx).attachments[1];
  p.ctx.state.conversations[0].messages.push({id: "sent"});
  p.ctx.conversation = {id: "other", messages: []};
  p.registration.onState(p.ctx.state, p.ctx);
  p.registration.composer.onSent({messageId: "sent", extras}, p.ctx);
  p.ctx.conversation = p.ctx.state.conversations[0];
  p.registration.onState(p.ctx.state, p.ctx);
  assert.deepEqual(Array.from(p.registration.sendExtras(p.ctx).attachments), [next]);
});

test("T18 first delayed disk restore does not resurrect a removed file", async () => {
  const id = `document_${"e".repeat(64)}`;
  const p = panel(() => ({metadata: {id}}));
  p.registration.onState(p.ctx.state, p.ctx);
  p.add([new File(["sample"], "sample.txt")]); await flush();
  byClass(p.root, "attachment-remove").click(); await flush();
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: ["upload-1"]}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  assert.equal(allByClass(p.root, "attachment-chip").length, 0);
});

test("T18 delayed restore waits for a recent import and preserves its occurrence", async () => {
  const id = `document_${"e".repeat(64)}`;
  let complete;
  const p = panel(() => new Promise(resolve => { complete = resolve; }));
  p.registration.onState(p.ctx.state, p.ctx);
  p.add([new File(["sample"], "sample.txt")]); await flush();
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: ["old-upload"]}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  assert.equal(allByClass(p.root, "attachment-chip").length, 1);
  complete({metadata: {id}}); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 1);
  assert.match(byClass(p.root, "attachment-chip").textContent, /sample.txt/);
  assert.deepEqual(JSON.parse(JSON.stringify(p.registration.sendExtras(p.ctx).attachment_selections)), {[id]: ["upload-1"]});
});

for (const removed of [false, true]) test(`T15 preparation failure retains the image unless removed ${removed}`, async () => {
  let complete;
  const id = `attachment_${"f".repeat(64)}`;
  const p = panel((_n, url) => url === "/api/attachments/precaption"
    ? new Promise(resolve => {complete = resolve;}) : {metadata: {id}});
  p.add([new File(["sample"], "sample.png", {type: "image/png"})]); await flush();
  p.runTimer(1000); await flush();
  const preparation = p.requests.find(request => request.url === "/api/attachments/precaption");
  assert.ok(preparation);
  if (removed) byClass(p.root, "attachment-remove").click();
  complete({status: 500, message: "Préparation indisponible"}); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, removed ? 0 : 1);
  assert.equal(p.registration.sendExtras(p.ctx).attachments.length, removed ? 0 : 1);
  if (removed) assert.equal(preparation.options.signal.aborted, true);
  else {
    assert.equal(p.blocked(), false);
    assert.equal(byClass(p.root, "attachment-retry").hidden, false);
    assert.match(byClass(p.root, "attachment-status").textContent, /indisponible/);
  }
});

test("T18 restored occurrence tokens are captured and released without removing a newer selection", async () => {
  const p = panel();
  const id = `document_${"f".repeat(64)}`;
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: ["persisted-upload"]}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  assert.deepEqual(JSON.parse(JSON.stringify(p.registration.sendExtras(p.ctx).attachment_selections)), {[id]: ["persisted-upload"]});
  byClass(p.root, "attachment-remove").click(); await flush();
  const release = JSON.parse(p.requests[0].options.body);
  assert.deepEqual(release.cancelled_uploads, ["persisted-upload"]);
  assert.deepEqual(release.removed, []);
});

test("T18 retry after a delayed restore keeps the user's recent file and releases the older occurrence", async () => {
  const id = `document_${"9".repeat(64)}`;
  let uploads = 0;
  const p = panel((_n, url) => {
    if (url === "/api/storage/drafts") return {};
    return ++uploads === 1 ? {error: new TypeError("offline")} : {metadata: {id}};
  });
  p.registration.onState(p.ctx.state, p.ctx);
  p.add([new File(["sample"], "sample.txt")]); await flush();
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: ["old-upload"]}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  byClass(allByClass(p.root, "attachment-chip")[0], "attachment-retry").click(); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 1);
  assert.match(byClass(p.root, "attachment-chip").textContent, /sample.txt/);
  assert.deepEqual(JSON.parse(JSON.stringify(p.registration.sendExtras(p.ctx).attachment_selections)), {[id]: ["upload-1"]});
  const release = p.requests.filter(request => request.url === "/api/storage/drafts").map(request => JSON.parse(request.options.body));
  assert.ok(release.some(request => request.cancelled_uploads.includes("old-upload")));
  assert.ok(release.every(request => !request.cancelled_uploads.includes("upload-1")));
});

test("T18 delayed disk restore preserves the file reselected after an explicit removal", async () => {
  const id = `document_${"8".repeat(64)}`;
  const p = panel(() => ({metadata: {id}}));
  p.registration.onState(p.ctx.state, p.ctx);
  p.add([new File(["sample"], "sample.txt")]); await flush();
  byClass(p.root, "attachment-remove").click(); await flush();
  p.add([new File(["sample"], "sample.txt")]); await flush();
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: ["upload-1"]}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  assert.equal(allByClass(p.root, "attachment-chip").length, 1);
  assert.match(byClass(p.root, "attachment-chip").textContent, /sample.txt/);
  assert.deepEqual(JSON.parse(JSON.stringify(p.registration.sendExtras(p.ctx).attachment_selections)), {[id]: ["upload-2"]});
});

test("T15 a failed durable removal leaves a visible explanation", async () => {
  const p = panel((_n, url) => url === "/api/storage/drafts" ? {error: new TypeError("offline")} : {});
  p.add([new File(["sample"], "sample.txt")]); await flush();
  byClass(p.root, "attachment-remove").click(); await flush();
  assert.equal(allByClass(p.root, "attachment-chip").length, 0);
  assert.match(p.notifications.at(-1), /brouillon reste conservé sur disque/);
});

test("T18 removing a restored legacy file captures its tokenless occurrence", async () => {
  const p = panel();
  const id = `document_${"7".repeat(64)}`;
  p.ctx.state.storage = {drafts: {chat: [{id, kind: "document", name: "sample.txt", byte_length: 6, upload_tokens: []}]}};
  p.registration.onState(p.ctx.state, p.ctx);
  byClass(p.root, "attachment-remove").click(); await flush();
  const removal = JSON.parse(p.requests[0].options.body);
  assert.deepEqual(removal.removed, [id]);
  assert.deepEqual(removal.removed_selections, {[id]: []});
});
