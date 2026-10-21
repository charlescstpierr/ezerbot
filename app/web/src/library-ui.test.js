import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

class Element {
  constructor(tag) {
    this.tagName = tag;
    this.children = [];
    this.className = "";
    this.value = "";
    this.hidden = false;
    this.listeners = {};
    this.files = [];
  }
  append(...items) {
    for (const child of items) {
      child.parent = this;
      this.children.push(child);
    }
  }
  replaceChildren(...items) {
    this.children = [];
    this.text = "";
    this.append(...items);
  }
  set textContent(text) {
    this.text = text;
    this.children = [];
  }
  get textContent() {
    return (this.text || "") + this.children.map((child) => child.textContent).join("");
  }
  setAttribute(name, value) { this[name] = value; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { this.clicks = (this.clicks || 0) + 1; }
  get classList() {
    const el = this;
    return {
      add(name) {
        const parts = new Set((el.className || "").split(/\s+/).filter(Boolean));
        parts.add(name);
        el.className = [...parts].join(" ");
      },
      remove(name) {
        el.className = (el.className || "").split(/\s+/).filter((part) => part && part !== name).join(" ");
      },
    };
  }
}
const walk = (root) => [root, ...root.children.flatMap(walk)];
const byClass = (root, name) => walk(root).find((el) => el.className?.split(/\s+/).includes(name));
const flush = () => new Promise((resolve) => setImmediate(resolve));

const button = (root, text) => walk(root).find((el) => el.tagName === "button" && el.textContent === text);
const ADDED = Date.UTC(2026, 9, 21, 18, 4);

async function mount(bot, initial = []) {
  const requests = [];
  const bodies = [];
  let files = initial.map((file) => ({ ...file }));
  const root = new Element("div");
  const panels = {
    register(value) { this.registration = value; },
    notify() {},
    async api(path, body) {
      requests.push(path);
      bodies.push(body);
      if (body === undefined) return { ceiling: "20 Mo", files };
      if (path === "/api/library/remove") {
        files = files.filter((file) => file.id !== body.id);
        return { ceiling: "20 Mo", files };
      }
      const previous = files.some((file) => file.name === body.name);
      files = [
        ...files.filter((file) => file.name !== body.name),
        { id: "document_test", name: body.name, format: "md", byte_length: 28, added_at: ADDED },
      ];
      return { ceiling: "20 Mo", files, replaced: previous };
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/library.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels },
    btoa, encodeURIComponent,
    document: { createElement: (tag) => new Element(tag) },
  });
  const cleanup = panels.registration.botSection(bot, root);
  await flush();
  return { root, requests, bodies, cleanup };
}

test("an unsaved bot asks to be saved and still announces the ceiling", async () => {
  const panel = await mount(null);
  assert.match(panel.root.textContent, /Enregistrez d’abord ce bot/);
  assert.match(panel.root.textContent, /20 Mo/);
  assert.equal(panel.requests.length, 0);
});

test("dropping tarifs.md lists the file and a bad or oversized file never calls the API", async () => {
  const panel = await mount({ id: "bot-a", name: "Alpha" });
  try {
    assert.match(panel.root.textContent, /20 Mo/);
    assert.match(panel.root.textContent, /Aucun fichier pour l’instant/);
    assert.deepEqual(panel.requests, ["/api/library?bot_id=bot-a"]);
    const zone = byClass(panel.root, "library-drop");
    const file = {
      name: "tarifs.md",
      size: 24,
      async arrayBuffer() { return new TextEncoder().encode("Le tarif X est de 42 euros.").buffer; },
    };
    await zone.listeners.drop({
      preventDefault() {},
      dataTransfer: { files: [file] },
    });
    await flush();
    assert.equal(panel.bodies.at(-1).name, "tarifs.md");
    assert.match(panel.root.textContent, /tarifs\.md est dans la bibliothèque/);
    assert.match(byClass(panel.root, "library-list").textContent, /tarifs\.md/);
    assert.equal(panel.requests.filter((path) => path === "/api/library").length, 1);

    await zone.listeners.drop({
      preventDefault() {},
      dataTransfer: { files: [{ name: "photo.png", size: 4, async arrayBuffer() { throw new Error("lu"); } }] },
    });
    assert.match(byClass(panel.root, "library-error").textContent, /Format non accepté/);
    assert.equal(panel.requests.filter((path) => path === "/api/library").length, 1);

    await zone.listeners.drop({
      preventDefault() {},
      dataTransfer: { files: [{ name: "gros.pdf", size: 20 * 1024 * 1024 + 1 }] },
    });
    assert.match(byClass(panel.root, "library-error").textContent, /20 Mo/);
    assert.equal(panel.requests.filter((path) => path === "/api/library").length, 1);
  } finally {
    panel.cleanup();
  }
});

test("the list shows name, size and date, confirms removal, and replaces the same name", async () => {
  const panel = await mount({ id: "bot-a", name: "Alpha" }, [
    { id: "document_old", name: "tarifs.md", format: "md", byte_length: 28, added_at: ADDED },
  ]);
  try {
    const row = byClass(panel.root, "library-row");
    assert.match(row.textContent, /tarifs\.md/);
    assert.match(row.textContent, /28 octets/);
    assert.match(row.textContent, /2026/);
    const zone = byClass(panel.root, "library-drop");
    const file = {
      name: "tarifs.md",
      size: 28,
      async arrayBuffer() { return new TextEncoder().encode("Le tarif X est de 99 euros.").buffer; },
    };
    await zone.listeners.drop({ preventDefault() {}, dataTransfer: { files: [file] } });
    await flush();
    assert.match(panel.root.textContent, /tarifs\.md remplace la version précédente/);
    let rows = walk(panel.root).filter((el) => el.className?.split(/\s+/).includes("library-row"));
    assert.equal(rows.length, 1);

    const remove = button(panel.root, "Retirer");
    assert.equal(remove.type, "button");
    remove.listeners.click();
    const confirm = byClass(panel.root, "library-confirm");
    assert.equal(confirm.hidden, false);
    assert.match(confirm.textContent, /tarifs\.md/);
    assert.match(confirm.textContent, /ne le citeront plus/);
    assert.equal(panel.requests.filter((path) => path === "/api/library/remove").length, 0);
    button(panel.root, "Annuler").listeners.click();
    assert.equal(confirm.hidden, true);
    assert.match(byClass(panel.root, "library-list").textContent, /tarifs\.md/);

    button(panel.root, "Retirer").listeners.click();
    await button(panel.root, "Retirer le fichier").listeners.click();
    await flush();
    assert.equal(panel.bodies.at(-1).id, "document_test");
    assert.match(panel.root.textContent, /tarifs\.md a été retiré/);
    assert.match(panel.root.textContent, /Aucun fichier pour l’instant/);
  } finally {
    panel.cleanup();
  }
});
