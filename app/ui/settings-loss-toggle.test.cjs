// Run with: node --test app/ui/settings-loss-toggle.test.cjs
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");

const pageSource = readFileSync(`${__dirname}/index.html`, "utf8");
const DEV_FLAG = "#ezerbot-dev-simulate-loss";

class Element {
  constructor(tag, document) {
    this.ownerDocument = document;
    this.localName = String(tag || "div").toLowerCase();
    this.childNodes = [];
    this.attributes = {};
    this.parentElement = null;
  }
  get textContent() { return this.childNodes.map((node) => node.textContent ?? "").join(""); }
  setAttribute(name, value) {
    const key = String(name).toLowerCase();
    this.attributes[key] = String(value);
    if (key === "id") this.ownerDocument.ids.set(this.attributes.id, this);
  }
  getAttribute(name) { return this.attributes[String(name).toLowerCase()] ?? null; }
  removeAttribute(name) {
    const key = String(name).toLowerCase();
    if (key === "id") this.ownerDocument.ids.delete(this.attributes.id);
    delete this.attributes[key];
  }
  get hidden() { return Object.hasOwn(this.attributes, "hidden"); }
  set hidden(value) {
    if (value) this.setAttribute("hidden", "");
    else this.removeAttribute("hidden");
  }
  append(node) { node.parentElement = this; this.childNodes.push(node); }
}

function parsePage() {
  const document = {
    ids: new Map(),
    createElement: (tag) => new Element(tag, document),
    createTextNode: (text) => ({ textContent: String(text), parentElement: null }),
  };
  document.documentElement = new Element("html", document);
  document.body = new Element("body", document);
  document.documentElement.append(document.body);
  const voidTags = new Set("area base br col embed hr img input link meta param source track wbr".split(" "));
  const stack = [document.documentElement];
  const tokens = pageSource.match(/<!--[\s\S]*?-->|<!doctype[^>]*>|<\/[^>]+>|<(?:[^>"']|"[^"]*"|'[^']*')*>|[^<]+/gi) || [];
  for (const token of tokens) {
    if (token.startsWith("<!--") || token.startsWith("<!")) continue;
    if (token.startsWith("</")) { if (stack.length > 1) stack.pop(); continue; }
    if (!token.startsWith("<")) {
      if (token.trim()) stack.at(-1).append(document.createTextNode(token));
      continue;
    }
    const body = token.slice(1, token.endsWith("/>") ? -2 : -1).trim();
    const name = /^[\w-]+/.exec(body)[0].toLowerCase();
    const parent = stack.at(-1);
    const node = name === "html" ? document.documentElement : name === "body" ? document.body : document.createElement(name);
    const attrs = body.slice(name.length);
    for (const match of attrs.matchAll(/([\w:-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|(\S+)))?/g)) {
      node.setAttribute(match[1], match[2] ?? match[3] ?? match[4] ?? "");
    }
    if (node !== document.documentElement && node !== parent) parent.append(node);
    if (name === "body") stack.push(document.body);
    else if (name !== "html" && !token.endsWith("/>") && !voidTags.has(name)) stack.push(node);
  }
  document.getElementById = (id) => document.ids.get(id) || null;
  return document;
}

function styleText(document) {
  const chunks = [];
  const walk = (node) => {
    if (node.localName === "style") chunks.push(node.textContent);
    for (const child of node.childNodes || []) walk(child);
  };
  walk(document.documentElement);
  return chunks.join("\n");
}

// The shipped shell forbids inline scripts (script-src 'self'). Only the page
// stylesheet can hide the drill, and only its :target rule can reveal it.
function hiddenByStyle(document, hash) {
  const css = styleText(document);
  const hidden = new Set();
  for (const match of css.matchAll(/#([\w-]+)\s*\{[^}]*display\s*:\s*none/g)) {
    const id = match[1];
    const revealed = hash === `#${id}` && new RegExp(`#${id}:target\\s*\\{[^}]*display\\s*:\\s*(?!none)`).test(css);
    if (!revealed) hidden.add(id);
  }
  return hidden;
}

function concealed(node, hiddenIds) {
  for (let current = node; current; current = current.parentElement) {
    if (current.hidden) return true;
    if (hiddenIds.has(current.attributes?.id)) return true;
  }
  return !node;
}

function visibleText(node, hiddenIds) {
  if (!node || node.localName === "script" || node.localName === "style") return "";
  if (concealed(node, hiddenIds)) return "";
  if (!node.localName) return node.textContent || "";
  return (node.childNodes || []).map((child) => visibleText(child, hiddenIds)).join("");
}

function openShippedApp({ hash = "" } = {}) {
  const document = parsePage();
  const hiddenIds = hiddenByStyle(document, hash);
  return {
    document,
    shown: (node) => !concealed(node, hiddenIds),
    text: (node) => visibleText(node, hiddenIds),
  };
}

test("the shipped settings hide the outage drill unless the explicit dev flag is set", () => {
  const production = openShippedApp();
  const control = production.document.getElementById("simulate-loss");
  assert.ok(control, "le contrôle reste dans la page pour le script livré");
  assert.equal(production.shown(control), false);
  assert.doesNotMatch(production.text(production.document.getElementById("settings-dialog")), /Tester une coupure/);
  assert.doesNotMatch(production.text(production.document.body), /Tester une coupure/);

  const ignored = openShippedApp({ hash: "#ezerbot-dev=1" });
  assert.equal(ignored.shown(ignored.document.getElementById("simulate-loss")), false);
  assert.doesNotMatch(ignored.text(ignored.document.body), /Tester une coupure/);

  const development = openShippedApp({ hash: DEV_FLAG });
  assert.equal(development.shown(development.document.getElementById("simulate-loss")), true);
  assert.match(development.text(development.document.getElementById("settings-dialog")), /Tester une coupure/);
});
