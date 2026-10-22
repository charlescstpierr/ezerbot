import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { clearFindMarks, cycleFind, findCountLabel, findInRendered, highlightOccurrence, matchRanges, shouldRecenterFind } from "./thread-find.js";
import { createRatingTable, ratingLabels } from "./thread-rating.js";

const rare = "xyloglotte";

test("la recherche annonce le nombre et circule sans tenir compte de la casse", () => {
  assert.equal(matchRanges(`Oui, ${rare} revient, et encore ${rare}.`, "XYLOGLOTTE").length, 2);
  assert.deepEqual(matchRanges("   ", "   "), []);
  assert.equal(findCountLabel(3, 0), "1 sur 3 résultats");
  assert.equal(findCountLabel(0, 0), "Aucun résultat");
  assert.equal(cycleFind(0, 3, -1), 2);
  assert.equal(cycleFind(2, 3, 1), 0);
});

test("une réponse avec du texte avant et après un appel compte chaque fragment affiché", () => {
  const document = createDocument();
  const thread = document.createElement("div");
  for (const [id, body] of [["reply1:part:0", `Avant ${rare}`], ["tool", "carte"], ["reply1", `Après ${rare}`]]) {
    const fragment = article({ id, name: "Ezerbot", body, document });
    thread.append(fragment);
  }
  thread.ownerDocument = document;
  const found = findInRendered(thread, rare);
  assert.deepEqual(found.map((match) => [match.messageId, match.indexInMessage]), [["reply1:part:0", 0], ["reply1", 0]]);
  const mark = highlightOccurrence(thread.querySelector("[data-message-id=\"reply1:part:0\"]"), rare, 0);
  assert.equal(mark.textContent, rare);
  assert.equal(mark.parentNode.textContent, `Avant ${rare}`);
});

test("le texte d’un lien Markdown rendu est compté, pas les boutons du corps", () => {
  const document = createDocument();
  const root = article({ id: "m-bot", name: "Ezerbot", body: `Voir ${rare} et `, document });
  const bubble = root.querySelector("[data-find-body]");
  const link = document.createElement("button");
  link.setAttribute("data-streamdown", "link");
  link.textContent = rare;
  const copy = document.createElement("button");
  copy.textContent = rare;
  bubble.append(link, copy);
  const found = findInRendered(root, rare);
  assert.deepEqual(found.map((match) => match.indexInMessage), [0, 1]);
  const mark = highlightOccurrence(root, rare, 1);
  assert.equal(mark.parentNode, link);
});

test("une longue réponse à beaucoup de correspondances se traite en temps linéaire", () => {
  const text = "Les petites phrases de la prose française. ".repeat(5000);
  const start = performance.now();
  const matches = matchRanges(text, "e");
  assert.ok(matches.length > 20000);
  assert.deepEqual(matches[0], { start: 1, end: 2 });
  assert.ok(performance.now() - start < 2000, "la recherche rescanne tous les spans à chaque occurrence");
});

test("Fermer n’écrit rien, l’envoi est dans l’export, et le module ne parle pas au réseau", () => {
  const saved = new Map();
  const storage = {
    getItem: (key) => saved.get(key) ?? null,
    setItem: (key, value) => saved.set(key, value),
  };
  const table = createRatingTable(storage, { now: () => "2026-10-21T12:00:00.000Z", createId: () => "note-1" });
  assert.deepEqual(table.list(), []);
  const good = table.recordGood({ conversationId: "conv-1", messageId: "m-bot", runId: "run-avant" });
  assert.equal(good.vote, "bon");
  assert.equal(good.message_id, "m-bot");
  assert.equal(good.run_id, "run-avant");
  const sent = table.recordBad({ conversationId: "conv-1", messageId: "m-bot", runId: "run-avant", motif: "pas-demande", details: "Il invente une date." });
  assert.equal(sent.motif, ratingLabels.reasons[0].label);
  assert.equal(sent.details, "Il invente une date.");
  assert.equal(sent.run_id, "run-avant");
  const exported = table.exportDocument();
  assert.equal(exported.notations.length, 2);
  assert.equal(exported.notations[1].motif, "Pas ce que j’ai demandé");
  assert.equal(exported.notations[1].run_id, "run-avant");
  assert.equal(exported.notations[1].message_id, "m-bot");
  assert.equal(JSON.stringify(exported).includes("voteGrokBotFeedback"), false);
  const source = readFileSync(new URL("./thread-rating.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /fetch\s*\(/);
  assert.equal(ratingLabels.title, "Comment s’améliorer ?");
  assert.deepEqual(ratingLabels.reasons.map((item) => item.label), ["Pas ce que j’ai demandé", "Inexact", "Qualité faible", "Trop lent"]);
});

function createDocument() {
  function matches(element, selector) {
    return selector.split(",").some((part) => {
      const sel = part.trim();
      if (sel.startsWith(".")) return element.className.split(/\s+/).includes(sel.slice(1));
      if (sel.startsWith("[") && sel.endsWith("]")) {
        const body = sel.slice(1, -1);
        const eq = body.indexOf("=");
        if (eq < 0) return element.attrs.has(body);
        return element.attrs.get(body.slice(0, eq)) === body.slice(eq + 1).replaceAll('"', "");
      }
      const dot = sel.indexOf(".");
      if (dot > 0) return element.tagName === sel.slice(0, dot).toUpperCase() && element.className.split(/\s+/).includes(sel.slice(dot + 1));
      return element.tagName === sel.toUpperCase();
    });
  }
  class Element {
    constructor(tag) {
      this.nodeType = 1;
      this.tagName = tag.toUpperCase();
      this.className = "";
      this.attrs = new Map();
      this.childNodes = [];
      this.parentNode = null;
      this.ownerDocument = document;
    }
    setAttribute(name, value) { this.attrs.set(name, String(value)); }
    getAttribute(name) { return this.attrs.get(name) ?? null; }
    removeAttribute(name) { this.attrs.delete(name); }
    append(...items) {
      for (const item of items) {
        const node = typeof item === "string" ? document.createTextNode(item) : item;
        node.parentNode = this;
        this.childNodes.push(node);
      }
    }
    get textContent() { return this.childNodes.map((child) => child.textContent).join(""); }
    set textContent(value) { this.childNodes = []; this.append(value); }
    get parentElement() { return this.parentNode?.nodeType === 1 ? this.parentNode : this.parentNode?.parentElement ?? null; }
    get firstChild() { return this.childNodes[0] ?? null; }
    insertBefore(node, before) {
      const previous = node.parentNode;
      if (previous) {
        const old = previous.childNodes.indexOf(node);
        if (old >= 0) previous.childNodes.splice(old, 1);
      }
      node.parentNode = this;
      const index = before ? this.childNodes.indexOf(before) : -1;
      if (index < 0) this.childNodes.push(node);
      else this.childNodes.splice(index, 0, node);
    }
    removeChild(node) {
      const index = this.childNodes.indexOf(node);
      if (index >= 0) this.childNodes.splice(index, 1);
      node.parentNode = null;
    }
    querySelectorAll(selector) {
      const found = [];
      const visit = (node) => {
        for (const child of node.childNodes) {
          if (child.nodeType !== 1) continue;
          if (matches(child, selector)) found.push(child);
          visit(child);
        }
      };
      visit(this);
      return found;
    }
    querySelector(selector) { return this.querySelectorAll(selector)[0] ?? null; }
  }
  class TextNode {
    constructor(value) {
      this.nodeType = 3;
      this.nodeValue = String(value);
      this.parentNode = null;
      this.ownerDocument = document;
    }
    get textContent() { return this.nodeValue; }
    get parentElement() { return this.parentNode?.nodeType === 1 ? this.parentNode : this.parentNode?.parentElement ?? null; }
    replaceWith(...nodes) {
      const parent = this.parentNode;
      const index = parent.childNodes.indexOf(this);
      for (const node of nodes) node.parentNode = parent;
      parent.childNodes.splice(index, 1, ...nodes);
      this.parentNode = null;
    }
  }
  const document = {
    defaultView: { CSS: { highlights: new Map() }, Highlight: class { constructor(range) { this.range = range; } } },
    createRange() {
      let first, last, start, end;
      return {
        setStart(node, offset) { first = node; start = offset; },
        setEnd(node, offset) { last = node; end = offset; },
        toString() { return first === last ? first.nodeValue.slice(start, end) : ""; },
      };
    },
    createElement: (tag) => new Element(tag),
    createTextNode: (value) => new TextNode(value),
  };
  return document;
}

function article({ id, name, time = "12:00", body, buttons = [], document = createDocument() }) {
  const root = document.createElement("article");
  if (id) root.setAttribute("data-message-id", id);
  const header = document.createElement("header");
  header.className = "gb-meta";
  const strong = document.createElement("strong");
  strong.textContent = name;
  const clock = document.createElement("time");
  clock.textContent = time;
  header.append(strong, clock);
  const bubble = document.createElement("div");
  bubble.className = "gb-bubble";
  bubble.setAttribute("data-find-body", "");
  bubble.append(body);
  root.append(header, bubble);
  if (buttons.length) {
    const footer = document.createElement("footer");
    footer.className = "gb-actions";
    for (const label of buttons) {
      const button = document.createElement("button");
      button.textContent = label;
      footer.append(button);
    }
    root.append(footer);
  }
  root.ownerDocument = document;
  return root;
}

test("vous dans « je vous remercie » surligne le corps, pas l’en-tête Vous", () => {
  const root = article({ id: "m-user", name: "Vous", body: "je vous remercie", buttons: ["Bon résultat"] });
  const found = findInRendered(root, "VOUS");
  assert.equal(found.length, 1);
  assert.equal(found[0].messageId, "m-user");
  assert.equal(findCountLabel(found.length, found[0].indexInMessage), "1 sur 1 résultat");
  const mark = highlightOccurrence(root, "vous", 0);
  assert.equal(mark.textContent, "vous");
  assert.equal(mark.parentNode.className, "gb-bubble");
  assert.equal(root.querySelector("strong").textContent, "Vous");
  assert.equal(root.querySelector("time").textContent, "12:00");
  assert.equal(root.querySelector("button").textContent, "Bon résultat");
  assert.equal(root.querySelector("button").querySelector("mark"), null);
});

test("bot dans « Le bot répond. » ignore le nom Ezerbot et les boutons de notation", () => {
  const root = article({
    id: "m-bot",
    name: "Ezerbot",
    body: "Le bot répond.",
    buttons: ["Bon résultat", "Mauvais résultat"],
  });
  root.querySelector(".gb-bubble").className = "gb-markdown";
  const found = findInRendered(root, "bot");
  assert.equal(found.length, 1);
  assert.equal(found[0].messageId, "m-bot");
  assert.equal(found[0].indexInMessage, 0);
  const mark = highlightOccurrence(root, "bot", 0);
  assert.equal(mark.textContent, "bot");
  assert.equal(mark.parentNode.className, "gb-markdown");
  assert.equal(root.querySelector("strong").textContent, "Ezerbot");
  assert.deepEqual([...root.querySelectorAll("button")].map((button) => button.textContent), ["Bon résultat", "Mauvais résultat"]);
  assert.equal(highlightOccurrence(root, "résultat", 0), null);
});

test("le surlignage retrouve café NFD et İ dans le texte affiché", () => {
  const cafe = "café".normalize("NFD");
  const sentence = `Un ${cafe} serré.`;
  const root = article({ id: "m-cafe", name: "Ezerbot", body: sentence });
  const found = findInRendered(root, "CAFÉ");
  assert.equal(found.length, 1);
  assert.equal(sentence.slice(found[0].start, found[0].end), cafe);
  const mark = highlightOccurrence(root, "café", 0);
  assert.equal(mark.textContent, cafe);
  assert.equal(mark.textContent.normalize("NFC"), "café");
  assert.equal(root.querySelector("[data-find-body]").textContent.normalize("NFC"), "Un café serré.");

  const istanbul = article({ id: "m-i", name: "Vous", time: "İl y a une heure", body: "İstanbul" });
  const dotted = findInRendered(istanbul, "İ");
  assert.equal(dotted.length, 1);
  assert.equal("İstanbul".slice(dotted[0].start, dotted[0].end), "İ");
  const capital = highlightOccurrence(istanbul, "İ", 0);
  assert.equal(capital.textContent, "İ");
  assert.equal(istanbul.querySelector("[data-find-body]").textContent, "İstanbul");
  assert.equal(istanbul.querySelector("time").textContent, "İl y a une heure");
});

test("un rafraîchissement du fil ne recentre plus l’occurrence déjà affichée", () => {
  const hit = { query: "xyloglotte", messageId: "m-bot", indexInMessage: 1 };
  const opened = shouldRecenterFind(null, hit);
  assert.equal(opened.recenter, true);
  const refresh = shouldRecenterFind(opened.identity, { ...hit });
  assert.equal(refresh.recenter, false);
  const next = shouldRecenterFind(refresh.identity, { ...hit, indexInMessage: 0 });
  assert.equal(next.recenter, true);
  const message = shouldRecenterFind(next.identity, { query: "xyloglotte", messageId: "m-user", indexInMessage: 0 });
  assert.equal(message.recenter, true);
  const query = shouldRecenterFind(message.identity, { query: "autre", messageId: "m-user", indexInMessage: 0 });
  assert.equal(query.recenter, true);
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  const effect = source.slice(source.indexOf("useLayoutEffect(() => {"), source.indexOf("Tooltip.Provider"));
  assert.match(effect, /findInRendered\(root, find\.query\)/);
  assert.match(effect, /onFindMatches\?\.\(matches\)/);
  assert.match(effect, /shouldRecenterFind\(findAnchor\.current,/);
  assert.doesNotMatch(effect, /if \(mark\) mark\.scrollIntoView/);
  assert.match(effect, /if \(mark && recenter\) mark\.scrollIntoView/);
});

test("une réponse terminée propose Bon résultat et Mauvais résultat, pas pendant l’écriture", () => {
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  const footer = source.slice(source.indexOf("!streaming && (copyText || message.state === \"stopped\")"));
  assert.match(footer, /onRateGood && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateGood\(message\)\}>Bon résultat<\/button>/);
  assert.match(footer, /onRateBad && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateBad\(message\)\}>Mauvais résultat<\/button>/);
  const bundle = readFileSync(new URL("../../ui/chat.js", import.meta.url), "utf8");
  assert.match(bundle, /Bon résultat/);
  assert.match(bundle, /Mauvais résultat/);
  assert.match(bundle, /findInRendered/);
  assert.doesNotMatch(bundle, /findInThread/);
  assert.match(bundle, /createRatingTable/);
});

test("la recherche préserve le nœud que le renderer continue à mettre à jour", () => {
  const root = article({ id: "stream", name: "Ezerbot", body: " hello" });
  const body = root.querySelector("[data-find-body]");
  const text = body.firstChild;
  const bold = root.ownerDocument.createElement("strong");
  bold.textContent = "début";
  body.insertBefore(bold, text);
  highlightOccurrence(root, "hello");
  assert.equal(text.parentNode, body);
  assert.equal(root.ownerDocument.defaultView.CSS.highlights.has("gb-find-current"), true);
  clearFindMarks(root);
  assert.equal(root.ownerDocument.defaultView.CSS.highlights.has("gb-find-current"), false);
  text.nodeValue = " hello world";
  assert.equal(body.textContent, "début hello world");
});

test("les votes et détails survivent à une nouvelle session native sans localStorage", async () => {
  const { createNativeRatingTable } = await import("./thread-rating.js");
  let durable = null;
  const bridge = () => ({ async postMessage(body) {
    if (body.command === "read") return durable;
    durable = body.value;
    return true;
  } });
  const first = await createNativeRatingTable(bridge());
  await first.recordGood({ conversationId: "c", messageId: "m1" });
  await first.recordBad({ conversationId: "c", messageId: "m2", motif: "inexact", details: "Une date inventée" });
  const reopened = await createNativeRatingTable(bridge());
  assert.deepEqual(reopened.exportDocument().notations.map(({ vote, details }) => ({ vote, details })), [
    { vote: "bon", details: "" }, { vote: "mauvais", details: "Une date inventée" },
  ]);
});

test("les paragraphes et les sauts de ligne séparent les mots, le texte en ligne reste collé", () => {
  const document = createDocument();
  const root = article({ id: "m-bot", name: "Ezerbot", body: "", document });
  const bubble = root.querySelector("[data-find-body]");
  bubble.childNodes = [];
  const paragraph = (...parts) => {
    const node = document.createElement("p");
    node.append(...parts);
    return node;
  };
  const bold = document.createElement("strong");
  bold.textContent = "wor";
  const line = document.createElement("br");
  bubble.append(paragraph("Hello"), paragraph(bold, "ld"), paragraph("un", line, "deux"));
  assert.equal(findInRendered(root, "hello world").length, 1);
  assert.equal(findInRendered(root, "helloworld").length, 0);
  assert.equal(findInRendered(root, "un deux").length, 1);
  assert.equal(findInRendered(root, "undeux").length, 0);
  const mark = highlightOccurrence(root, "hello world", 0);
  assert.ok(mark);
  assert.equal(root.ownerDocument.defaultView.CSS.highlights.has("gb-find-current"), true);
});

test("les retours à la ligne que Streamdown laisse entre les paragraphes valent une espace", () => {
  const document = createDocument();
  const root = article({ id: "m-bot", name: "Ezerbot", body: "", document });
  const bubble = root.querySelector("[data-find-body]");
  bubble.childNodes = [];
  const paragraph = (value) => {
    const node = document.createElement("p");
    node.append(value);
    return node;
  };
  bubble.append(paragraph("Hello"), "\n", paragraph("world"), "\n");
  assert.equal(findInRendered(root, "hello world").length, 1);
  assert.equal(findInRendered(root, "helloworld").length, 0);
  assert.ok(highlightOccurrence(root, "hello world", 0));
});

test("sans Custom Highlights, la sélection native montre l’occurrence et se retire", () => {
  const root = article({ id: "m-bot", name: "Ezerbot", body: `Oui ${rare}` });
  const view = root.ownerDocument.defaultView;
  delete view.CSS;
  delete view.Highlight;
  const ranges = [];
  view.getSelection = () => ({
    get rangeCount() { return ranges.length; },
    getRangeAt: (index) => ranges[index],
    removeAllRanges() { ranges.length = 0; },
    addRange(range) { ranges.push(range); },
  });
  const mark = highlightOccurrence(root, rare, 0);
  assert.equal(mark.textContent, rare);
  assert.equal(ranges.length, 1);
  clearFindMarks(root);
  assert.equal(ranges.length, 0);
});

test("sans Custom Highlights, la frappe dans le champ de recherche garde la sélection", () => {
  const root = article({ id: "m-bot", name: "Ezerbot", body: `Oui ${rare}` });
  const document = root.ownerDocument;
  const view = document.defaultView;
  delete view.CSS;
  delete view.Highlight;
  const ranges = [];
  view.getSelection = () => ({
    get rangeCount() { return ranges.length; },
    getRangeAt: (index) => ranges[index],
    removeAllRanges() { ranges.length = 0; },
    addRange(range) { ranges.push(range); },
  });
  document.activeElement = { matches: (selector) => selector.split(", ").includes("input") };
  const mark = highlightOccurrence(root, rare, 0);
  assert.equal(mark.textContent, rare);
  assert.equal(ranges.length, 0);
  document.activeElement = null;
  highlightOccurrence(root, rare, 0);
  assert.equal(ranges.length, 1);
});

test("sans Custom Highlights, un contour montre le message actif même pendant la frappe", () => {
  const root = article({ id: "m-bot", name: "Ezerbot", body: `Oui ${rare}` });
  const document = root.ownerDocument;
  const view = document.defaultView;
  delete view.CSS;
  delete view.Highlight;
  view.getSelection = () => ({ rangeCount: 0, removeAllRanges() {}, addRange() {} });
  document.activeElement = { matches: (selector) => selector.split(", ").includes("input") };
  const body = root.querySelector("[data-find-body]");
  assert.ok(highlightOccurrence(root, rare, 0));
  assert.equal(body.getAttribute("data-find-active"), "");
  clearFindMarks(root);
  assert.equal(body.getAttribute("data-find-active"), null);
});

test("un vote que le Mac refuse n’entre ni dans l’export ni dans l’essai suivant", async () => {
  const { createNativeRatingTable } = await import("./thread-rating.js");
  const writes = [];
  let calls = 0;
  const table = await createNativeRatingTable({ async postMessage(body) {
    if (body.command === "read") return null;
    calls += 1;
    if (calls === 1) throw new Error("refusé");
    writes.push(JSON.parse(body.value));
    return true;
  } });
  await assert.rejects(table.recordGood({ conversationId: "c", messageId: "m1" }));
  assert.deepEqual(table.exportDocument().notations, []);
  const [first, second] = await Promise.all([
    table.recordGood({ conversationId: "c", messageId: "m1" }),
    table.recordBad({ conversationId: "c", messageId: "m2", motif: "inexact", details: "" }),
  ]);
  assert.deepEqual(table.exportDocument().notations.map((row) => row.message_id), ["m1", "m2"]);
  assert.deepEqual(writes.map((rows) => rows.length), [1, 2]);
  assert.equal(first.message_id, "m1");
  assert.equal(second.vote, "mauvais");
});

test("une formule rendue ne compte que sa partie visible, pas le MathML caché ni l’annotation TeX", () => {
  const document = createDocument();
  const root = article({ id: "m-bot", name: "Ezerbot", body: "Soit ", document });
  const bubble = root.querySelector("[data-find-body]");
  const katex = document.createElement("span");
  katex.className = "katex";
  const hidden = document.createElement("span");
  hidden.className = "katex-mathml";
  hidden.append("x", "x");
  const visible = document.createElement("span");
  visible.className = "katex-html";
  visible.append("x");
  katex.append(hidden, visible);
  bubble.append(katex);
  const found = findInRendered(root, "x");
  assert.equal(found.length, 1);
  const mark = highlightOccurrence(root, "x", 0);
  assert.equal(mark.parentNode, visible);
});
