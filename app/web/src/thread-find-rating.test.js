import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { cycleFind, findCountLabel, findInRendered, findInThread, highlightOccurrence } from "./thread-find.js";
import { createRatingTable, ratingLabels } from "./thread-rating.js";

const rare = "xyloglotte";

test("la recherche annonce le nombre et circule sans tenir compte de la casse", () => {
  const messages = [
    { id: "m-user", body: `As-tu vu ${rare} ?` },
    { id: "m-bot", body: `Oui, ${rare} revient, et encore ${rare}.` },
  ];
  const matches = findInThread(messages, "XYLOGLOTTE", (message) => message.body);
  assert.equal(matches.length, 3);
  assert.deepEqual(matches.map((match) => [match.messageId, match.indexInMessage]), [
    ["m-user", 0], ["m-bot", 0], ["m-bot", 1],
  ]);
  assert.equal(findCountLabel(3, 0), "1 sur 3 résultats");
  assert.equal(findCountLabel(0, 0), "Aucun résultat");
  assert.equal(cycleFind(0, 3, -1), 2);
  assert.equal(cycleFind(2, 3, 1), 0);
  assert.deepEqual(findInThread(messages, "   "), []);
});

test("Fermer n’écrit rien, l’envoi est dans l’export, et le module ne parle pas au réseau", () => {
  const saved = new Map();
  const storage = {
    getItem: (key) => saved.get(key) ?? null,
    setItem: (key, value) => saved.set(key, value),
  };
  const table = createRatingTable(storage, { now: () => "2026-10-21T12:00:00.000Z", createId: () => "note-1" });
  assert.deepEqual(table.list(), []);
  const good = table.recordGood({ conversationId: "conv-1", messageId: "m-bot" });
  assert.equal(good.vote, "bon");
  const sent = table.recordBad({ conversationId: "conv-1", messageId: "m-bot", motif: "pas-demande", details: "Il invente une date." });
  assert.equal(sent.motif, ratingLabels.reasons[0].label);
  assert.equal(sent.details, "Il invente une date.");
  const exported = table.exportDocument();
  assert.equal(exported.notations.length, 2);
  assert.equal(exported.notations[1].motif, "Pas ce que j’ai demandé");
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
    createElement: (tag) => new Element(tag),
    createTextNode: (value) => new TextNode(value),
  };
  return document;
}

function article({ id, name, time = "12:00", body, buttons = [] }) {
  const document = createDocument();
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

test("une réponse terminée propose Bon résultat et Mauvais résultat, pas pendant l’écriture", () => {
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  const footer = source.slice(source.indexOf("!streaming && (copyText || message.state === \"stopped\")"));
  assert.match(footer, /onRateGood && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateGood\(message\)\}>Bon résultat<\/button>/);
  assert.match(footer, /onRateBad && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateBad\(message\)\}>Mauvais résultat<\/button>/);
  const bundle = readFileSync(new URL("../../ui/chat.js", import.meta.url), "utf8");
  assert.match(bundle, /Bon résultat/);
  assert.match(bundle, /Mauvais résultat/);
  assert.match(bundle, /findInThread/);
  assert.match(bundle, /createRatingTable/);
});
