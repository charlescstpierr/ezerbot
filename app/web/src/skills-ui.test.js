import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// The skills panel uses the shell's DOM, not React. Exercise its actual list and
// editor rendering with the small DOM surface it needs; no browser dependency.
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.listeners = {}; this.value = ""; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; this.text = ""; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this[name] = value; }
  addEventListener(name, action) { this.listeners[name] = action; }
  querySelectorAll(selector) {
    const all = descendants(this);
    if (selector === "button") return all.filter((el) => el.tagName === "button");
    return [];
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  cloneNode() { return new Element(this.tagName); }
  remove() { this.removed = true; }
  showModal() { this.open = true; }
  close() { this.open = false; this.listeners.close?.(); }
  focus() {}
  setSelectionRange() {}
}
const descendants = (element) => [element, ...element.children.flatMap(descendants)];
const byClass = (element, name) => descendants(element).find((child) => child.className === name);

function panel(fileCount = 1, withDemonstration = false) {
  const root = new Element("div");
  const skill = { skill_id: "skill_internal_123", version: 1, name: "resume", display_name: "Résumé express", description: "En trois puces.", file_count: fileCount };
  const content = { ...skill, body: "Texte original.", editable_body: "Texte original.", files: [{ path: "exemple.md", content: "Texte annexe <script>non exécuté</script>" }] };
  let registration;
  const requests = [];
  const panels = {
    state: { skills: { by_bot: { bot: [skill] } }, demonstrations: { links: withDemonstration ? [{ skill_id: skill.skill_id, id: "demo" }] : [] } },
    register(value) { registration = value; },
    async api(path) { requests.push(path); return path === "/api/skills/import" ? { report: [{ status: { status: "written", skill_id: skill.skill_id } }] } : { versions: [{ ...skill, content }] }; },
    async refresh() {}, notify() {},
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels }, TextEncoder, btoa,
    document: { createElement: (tag) => new Element(tag), getElementById: () => root },
  });
  registration.botSection({ id: "bot" }, root);
  return { root, skill, panels, requests, registration };
}

test("skill list keeps the human title and version but not the internal id", () => {
  const { root, skill } = panel();
  const row = byClass(root, "skills-row");
  assert.equal(row.children[0].textContent, "Résumé expressEn trois puces.Version 1 · 1 annexe");
  assert.ok(!row.textContent.includes(skill.skill_id));
  // Keep the durable id for edit routing, not for display.
  assert.equal(byClass(row, "skills-row-actions").children[0].dataset.skillId, skill.skill_id);
  assert.equal(byClass(root, "skills-history-content").textContent, "");
});

test("skill actions share an aligned group and source links have distinct accessible names", () => {
  const { root } = panel(1, true);
  const actions = byClass(root, "skills-row-actions");
  assert.equal(actions.children.length, 3);
  assert.equal(actions.children[1]["aria-label"], "Voir la démonstration : Résumé express");
  assert.equal(actions.children[2].textContent, "Lancer");
  assert.equal(byClass(root, "skills-row").children[0].children.some(child => child.tagName === "button"), false);
});

test("Show the bot stays locked with other skill actions while an import is pending", async () => {
  const { root } = panel();
  const demonstrate = descendants(root).find(el => el.tagName === "button" && el.textContent === "Montrer au bot");
  const input = descendants(root).find(el => el.name === "skills-files");
  let release;
  input.files = [{ name: "note.md", size: 5, arrayBuffer: () => new Promise(resolve => { release = resolve; }) }];
  input.listeners.change();
  assert.equal(demonstrate.disabled, true);
  release(new TextEncoder().encode("Texte").buffer);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(demonstrate.disabled, false);
});

test("starting a demonstration gives persistent feedback inside the retained bot editor", () => {
  const { root, panels, registration } = panel();
  const demonstrate = descendants(root).find(el => el.tagName === "button" && el.textContent === "Montrer au bot");
  panels.state.demonstrations.session = { id: "demo", bot_id: "bot", status: "recording" };
  registration.onState();
  assert.equal(demonstrate.textContent, "Démonstration en cours…");
  assert.equal(demonstrate.title, "Ouvrir les contrôles de la démonstration");
  assert.equal(demonstrate.disabled, false);
});

test("skill list shows a bounded companion-file count", () => {
  const { root } = panel(2);
  assert.ok(byClass(root, "skills-row").textContent.includes("Version 1 · 2 annexes"));
});

test("skill editor hides the id while still loading its exact version history", async () => {
  const { root, skill, requests } = panel();
  byClass(root, "skills-row-actions").children[0].listeners.click();
  await new Promise((resolve) => setImmediate(resolve));
  const dialog = byClass(root, "dialog skills-dialog");
  assert.equal(dialog.open, true);
  assert.ok(requests[0].includes(`skill_id=${skill.skill_id}`));
  const fields = byClass(dialog, "skills-fields");
  assert.equal(fields.children.find((child) => child.tagName === "p").textContent, "Version 1");
  assert.ok(!dialog.textContent.includes(skill.skill_id));
  assert.equal(fields.children.find((child) => child.tagName === "input").value, "Résumé express");
  assert.equal(byClass(dialog, "skills-history").children[1].children[0].textContent, "Version 1");
  const files = byClass(dialog, "skills-companion-files");
  assert.equal(files.hidden, false);
  assert.ok(files.textContent.includes("exemple.md"));
  assert.ok(files.textContent.includes("<script>non exécuté</script>"));
  assert.equal(requests.length, 1); // Bodies/files are loaded only on opening.
  const history = byClass(dialog, "skills-history");
  history.children[1].value = "1";
  history.children[1].listeners.change();
  const oldFiles = byClass(history, "skills-companion-files");
  assert.equal(oldFiles.children[1].tagName, "details");
  assert.equal(oldFiles.children[1].children[0].textContent, "exemple.md");
  assert.equal(oldFiles.children[1].children[1].textContent, "Texte annexe <script>non exécuté</script>");
  assert.ok(!byClass(history.children[2], "skills-version-preview").textContent.includes("exemple.md"));
});

function mount(options = {}) {
  const root = new Element("div");
  const skill = options.skill || { skill_id: "skill_internal_123", version: 2, name: "modele", display_name: "Modèle", description: "Suit le modèle.", file_count: 1 };
  const requests = [];
  let registration;
  const panels = {
    state: options.state || { skills: { by_bot: { bot: [skill] } }, demonstrations: { links: [] }, conversations: [] },
    register(value) { registration = value; },
    async api(path, body) {
      requests.push({ path, body });
      if (options.api) return options.api(path, body, requests);
      return { versions: [] };
    },
    async refresh() {
      requests.push({ path: "refresh" });
      if (options.refresh) await options.refresh(panels, requests);
    },
    notify(message) { options.notify?.(message); },
    openConversation(id) {
      const known = (panels.state.conversations || []).some((item) => item.id === id);
      requests.push({ path: "open", body: id, known });
      return known;
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels }, crypto: { randomUUID: () => "msg-lance" }, TextEncoder, btoa,
    document: { createElement: (tag) => new Element(tag), getElementById: () => root, querySelectorAll: () => [] },
  });
  registration.botSection({ id: "bot" }, root);
  return { root, skill, panels, requests, registration };
}

const textFile = (path, text) => {
  const bytes = new TextEncoder().encode(text);
  return {
    name: path.split("/").pop(),
    webkitRelativePath: path,
    size: bytes.byteLength,
    arrayBuffer: async () => bytes.slice().buffer,
  };
};

async function flushImport() {
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setImmediate(resolve));
}

test("the skills section counts the SKL-3 scenario and can record the shortcut", () => {
  const { root, registration } = mount();
  assert.match(root.textContent, /Scénario SKL-3 · 0\/4/);
  for (const label of ["Import du dossier", "Annexe utilisable", "Version précédente restaurable", "Raccourci ⌘⇧W"]) {
    assert.ok(root.textContent.includes(label), label);
  }
  assert.equal(root.textContent.includes("Publier"), false);
  registration.noteShortcut("bot");
  assert.match(root.textContent, /Scénario SKL-3 · 1\/4/);
  assert.equal(descendants(root).find((el) => el.dataset.point === "shortcut").dataset.done, "true");
});

test("an annex import report stays readable and checks the folder point", async () => {
  const { root } = mount({
    api(path, body) {
      if (path !== "/api/skills/import") return { versions: [] };
      assert.deepEqual(Array.from(body.files, (file) => String(file.path)), ["dossier/SKILL.md", "dossier/modele.md"]);
      return { report: [
        { status: { status: "written", skill_id: "skill_internal_123" } },
        { status: { status: "annex", skill_id: "skill_internal_123" } },
      ] };
    },
  });
  const input = descendants(root).find((el) => el.name === "skills-files");
  input.files = [textFile("dossier/SKILL.md", "---\nname: Modèle\n---\nCorps"), textFile("dossier/modele.md", "Réponds exactement : Le modèle annexe est suivi.")];
  input.listeners.change();
  await flushImport();
  const annex = descendants(root).find((el) => el.dataset.status === "annex");
  assert.ok(annex, "la ligne modele.md doit rester un statut annexe");
  assert.match(annex.textContent, /Annexe conservée/);
  assert.equal(descendants(root).find((el) => el.dataset.point === "import").dataset.done, "true");
});

test("restoring a previous version brings its annex back without closing the editor", async () => {
  const versions = [
    { skill_id: "skill_internal_123", version: 1, content: { name: "modele", display_name: "Modèle", description: "Ancienne", body: "Premier corps", editable_body: "Premier corps", files: [{ path: "modele.md", content: "Ancienne annexe" }] } },
    { skill_id: "skill_internal_123", version: 2, content: { name: "modele", display_name: "Modèle", description: "Nouvelle", body: "Second corps", editable_body: "Second corps", files: [{ path: "modele.md", content: "Nouvelle annexe" }] } },
  ];
  let restored = false;
  const { root, requests } = mount({
    api(path, body) {
      if (path === "/api/skills/restore") {
        restored = true;
        assert.equal(body.version, 1);
        assert.equal(body.expected_version, 2);
        versions.unshift({ skill_id: body.skill_id, version: 3, content: { ...versions[0].content } });
        return { skill: versions[0] };
      }
      return { versions: versions.slice() };
    },
  });
  byClass(root, "skills-row-actions").children[0].listeners.click();
  await flushImport();
  const history = byClass(root, "skills-history");
  history.children[1].value = "1";
  history.children[1].listeners.change();
  const restore = descendants(history).find((el) => el.tagName === "button" && el.textContent === "Restaurer cette version");
  assert.ok(restore, "la version précédente doit pouvoir être restaurée");
  restore.listeners.click();
  await flushImport();
  assert.equal(restored, true);
  assert.equal(byClass(root, "dialog skills-dialog").open, true);
  assert.match(byClass(root, "skills-companion-files").textContent, /Ancienne annexe/);
  assert.equal(descendants(root).find((el) => el.dataset.point === "restore").dataset.done, "true");
  assert.ok(requests.some((call) => call.path === "/api/skills/restore"));
});

test("launching a skill sends it and checks the usable-annex point", async () => {
  const { root, requests } = mount({
    state: { skills: { by_bot: { bot: [{ skill_id: "skill_internal_123", version: 1, name: "modele", display_name: "Modèle", description: "Suit le modèle.", file_count: 1 }] } }, demonstrations: { links: [] }, conversations: [{ id: "conv-1", bot_id: "bot", kind: "chat" }] },
    api(path, body) {
      if (path === "/api/send") return { conversation: { id: body.conversation_id, messages: [{ id: body.message_id, role: "user", body: body.body }, { id: "reply", role: "assistant", body: "Le modèle annexe est suivi." }] } };
      if (String(path).startsWith("/api/skills/versions")) {
        return { versions: [{ version: 1, deleted: false, content: { files: [{ path: "modele.md", content: "Réponds exactement : Le modèle annexe est suivi." }] } }] };
      }
      return { versions: [] };
    },
  });
  const launch = descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer");
  assert.ok(launch, "chaque skill doit pouvoir être lancée");
  launch.listeners.click();
  await flushImport();
  const send = requests.find((call) => call.path === "/api/send");
  assert.ok(send, "lancer poste /api/send");
  assert.equal(send.body.apply_skill_id, "skill_internal_123");
  assert.match(send.body.body, /Lance la skill « Modèle »/);
  const open = requests.find((call) => call.path === "open");
  assert.equal(open?.known, true);
  assert.equal(open.body, "conv-1");
  assert.equal(descendants(root).find((el) => el.dataset.point === "annex").dataset.done, "true");
});

test("the usable-annex point follows the reply, not only a successful send", async () => {
  const annex = "Réponds exactement : Le modèle annexe est suivi.";
  const { root } = mount({
    state: { skills: { by_bot: { bot: [{ skill_id: "skill_internal_123", version: 1, name: "modele", display_name: "Modèle", description: "Suit le modèle.", file_count: 1 }] } }, demonstrations: { links: [] }, conversations: [{ id: "conv-1", bot_id: "bot", kind: "chat" }] },
    api(path) {
      if (String(path).startsWith("/api/skills/versions")) {
        return { versions: [{ version: 1, deleted: false, content: { files: [{ path: "modele.md", content: annex }] } }] };
      }
      if (path === "/api/send") {
        return { conversation: { id: "conv-1", messages: [{ role: "assistant", body: "Bien reçu. Ton message est conservé dans cette conversation." }] } };
      }
      return { versions: [] };
    },
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer").listeners.click();
  await flushImport();
  assert.equal(descendants(root).find((el) => el.dataset.point === "annex").dataset.done, "false");
});

test("a folder import skips a symlink and a path that leaves the folder", async () => {
  const requests = [];
  const { root } = mount({
    api(path, body) {
      if (path === "/api/skills/import") requests.push(body.files.map((file) => file.path));
      return { report: body.files.map(() => ({ status: { status: "written", skill_id: "s" } })) };
    },
  });
  const input = descendants(root).find((el) => el.name === "skills-files");
  const link = textFile("dossier/lien.md", "secret hors dossier");
  link.symlink = true;
  input.files = [
    textFile("dossier/SKILL.md", "---\nname: Modèle\n---\nCorps"),
    textFile("dossier/modele.md", "annexe"),
    link,
    textFile("dossier/../secret.md", "échappé"),
  ];
  input.listeners.change();
  await flushImport();
  const sent = requests.flat();
  assert.ok(sent.includes("dossier/SKILL.md"));
  assert.ok(sent.includes("dossier/modele.md"));
  assert.equal(sent.includes("dossier/lien.md"), false);
  assert.equal(sent.includes("dossier/../secret.md"), false);
});

test("launching a skill reveals the reply once the new conversation is loaded", async () => {
  const { root, requests } = mount({
    api(path, body) {
      if (path === "/api/conversations") return { conversation: { id: "conv-new", bot_id: "bot", kind: "chat", messages: [] } };
      if (path === "/api/send") return { conversation: { id: body.conversation_id, messages: [{ role: "assistant", body: "Le modèle annexe est suivi." }] } };
      return { versions: [] };
    },
    refresh(panels, calls) {
      const sent = calls.find((call) => call.path === "/api/send");
      if (!sent) return;
      panels.state.conversations = [{ id: sent.body.conversation_id, bot_id: "bot", kind: "chat", messages: [{ role: "assistant", body: "Le modèle annexe est suivi." }] }];
    },
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer").listeners.click();
  await flushImport();
  const openAt = requests.findIndex((call) => call.path === "open");
  const refreshAt = requests.findIndex((call) => call.path === "refresh");
  assert.ok(refreshAt !== -1 && openAt !== -1 && refreshAt < openAt, "la conversation créée doit être rechargée avant d’être affichée");
  assert.equal(requests[openAt].known, true);
  assert.equal(requests[openAt].body, "conv-new");
  assert.equal(requests.find((call) => call.path === "/api/send").body.apply_skill_id, "skill_internal_123");
});

test("an absolute path is not uploaded", async () => {
  const requests = [];
  const { root } = mount({
    api(path, body) {
      if (path === "/api/skills/import") requests.push(body.files.map((file) => file.path));
      return { report: body.files.map(() => ({ status: { status: "written", skill_id: "s" } })) };
    },
  });
  const input = descendants(root).find((el) => el.name === "skills-files");
  input.files = [
    textFile("dossier/SKILL.md", "---\nname: Modèle\n---\nCorps"),
    textFile("/etc/secret.md", "hors racine"),
    textFile("\\windows\\secret.md", "hors racine"),
  ];
  input.listeners.change();
  await flushImport();
  const sent = requests.flat();
  assert.ok(sent.includes("dossier/SKILL.md"));
  assert.equal(sent.includes("/etc/secret.md"), false);
  assert.equal(sent.includes("\\windows\\secret.md"), false);
  const refused = descendants(root).filter((el) => el.dataset.status === "skipped");
  assert.equal(refused.length, 2);
  assert.match(refused[0].textContent, /chemin/);
});

test("companions stay local when SKILL.md cannot be prepared", async () => {
  const requests = [];
  const { root } = mount({
    api(path, body) {
      if (path === "/api/skills/import") requests.push(body.files.map((file) => file.path));
      return { report: body.files.map(() => ({ status: { status: "written", skill_id: "s" } })) };
    },
  });
  const input = descendants(root).find((el) => el.name === "skills-files");
  const skill = textFile("dossier/SKILL.md", "---\nname: Modèle\n---\nCorps");
  skill.arrayBuffer = async () => { throw new Error("read"); };
  const oversized = textFile("autre/SKILL.md", "---\nname: Large\n---\nCorps");
  oversized.size = 8 * 1024 * 1024 + 1;
  input.files = [skill, textFile("dossier/modele.md", "annexe"), oversized, textFile("autre/modele.md", "annexe large")];
  input.listeners.change();
  await flushImport();
  const sent = requests.flat();
  assert.equal(sent.includes("dossier/SKILL.md"), false);
  assert.equal(sent.includes("dossier/modele.md"), false);
  assert.equal(sent.includes("autre/SKILL.md"), false);
  assert.equal(sent.includes("autre/modele.md"), false);
});

const annexPhrase = "Réponds exactement : Le modèle annexe est suivi.";
const launchState = (conversations) => ({
  skills: { by_bot: { bot: [{ skill_id: "skill_internal_123", version: 1, name: "modele", display_name: "Modèle", description: "Suit le modèle.", file_count: 1 }] } },
  demonstrations: { links: [] },
  conversations,
});
const launchApi = (path, body) => {
  if (path === "/api/conversations") return { conversation: { id: "conv-new", bot_id: "bot", kind: "chat", messages: [] } };
  if (path === "/api/send") return { conversation: { id: body.conversation_id, messages: [{ id: body.message_id, role: "user", body: body.body }, { id: "reply", role: "assistant", body: "Le modèle annexe est suivi." }] } };
  if (String(path).startsWith("/api/skills/versions")) {
    return { versions: [{ version: 1, deleted: false, content: { files: [{ path: "modele.md", content: annexPhrase }] } }] };
  }
  return { versions: [] };
};

test("a routine is not reused and a group still receives the launch", async () => {
  const { root, requests } = mount({
    state: launchState([
      { id: "routine-1", bot_id: "bot", kind: "routine" },
      { id: "group-1", bot_id: "bot", kind: "group" },
    ]),
    api: launchApi,
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer").listeners.click();
  await flushImport();
  const send = requests.find((call) => call.path === "/api/send");
  assert.equal(send.body.conversation_id, "group-1");
  assert.equal(requests.some((call) => call.path === "/api/conversations"), false);
});

test("only a routine means a new chat is created", async () => {
  const { root, requests } = mount({
    state: launchState([{ id: "routine-1", bot_id: "bot", kind: "routine" }]),
    api: launchApi,
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer").listeners.click();
  await flushImport();
  assert.equal(requests.find((call) => call.path === "/api/send").body.conversation_id, "conv-new");
  assert.equal(requests.some((call) => call.path === "/api/conversations"), true);
});

test("the usable-annex point waits for the final reply", async () => {
  const phrase = "Le modèle annexe est suivi.";
  const streaming = [
    { id: "old", role: "assistant", body: phrase },
    { id: "msg-lance", role: "user", body: "Lance" },
    { id: "reply", role: "assistant", state: "streaming", body: phrase },
  ];
  const { root, panels, registration } = mount({
    state: launchState([{ id: "conv-1", bot_id: "bot", kind: "chat", messages: [] }]),
    api(path, body) {
      if (path === "/api/send") return { conversation: { id: body.conversation_id, messages: streaming } };
      return launchApi(path, body);
    },
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Lancer").listeners.click();
  await flushImport();
  assert.equal(descendants(root).find((el) => el.dataset.point === "annex").dataset.done, "false");
  panels.state.conversations = [{
    id: "conv-1",
    bot_id: "bot",
    kind: "chat",
    messages: [
      { id: "old", role: "assistant", body: phrase },
      { id: "msg-lance", role: "user", body: "Lance" },
      { id: "reply", role: "assistant", state: "complete", body: phrase },
    ],
  }];
  registration.onState();
  assert.equal(descendants(root).find((el) => el.dataset.point === "annex").dataset.done, "true");
});

test("a skill folder stays in one import request when the batch is nearly full", async () => {
  const requests = [];
  const { root } = mount({
    api(path, body) {
      if (path === "/api/skills/import") requests.push(body.files.map((file) => file.path));
      return { report: body.files.map(() => ({ status: { status: "written", skill_id: "s" } })) };
    },
  });
  const encoder = new TextEncoder();
  const overhead = encoder.encode(JSON.stringify({ bot_id: "bot", files: [] })).length;
  const uploadSize = (path, bytes) => encoder.encode(JSON.stringify({ path, base64: Buffer.from(bytes).toString("base64") })).length;
  const skillText = "---\nname: Modèle\n---\nCorps";
  const modeleText = `Réponds exactement : Le modèle annexe est suivi.\n${"x".repeat(1800)}`;
  const skillSize = uploadSize("dossier/SKILL.md", encoder.encode(skillText));
  const modeleSize = uploadSize("dossier/modele.md", encoder.encode(modeleText));
  const target = 8 * 1024 * 1024 - overhead - 1 - skillSize - 8;
  let low = 1;
  let high = target;
  let pad = "a";
  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    const size = uploadSize("pad.txt", encoder.encode("a".repeat(mid)));
    if (size <= target) { pad = "a".repeat(mid); low = mid + 1; }
    else high = mid - 1;
  }
  const padSize = uploadSize("pad.txt", encoder.encode(pad));
  assert.equal(overhead + padSize + 1 + skillSize <= 8 * 1024 * 1024, true);
  assert.equal(overhead + padSize + 1 + skillSize + 1 + modeleSize > 8 * 1024 * 1024, true);
  const input = descendants(root).find((el) => el.name === "skills-files");
  input.files = [textFile("pad.txt", pad), textFile("dossier/SKILL.md", skillText), textFile("dossier/modele.md", modeleText)];
  input.listeners.change();
  await flushImport();
  assert.ok(requests.some((paths) => paths.includes("dossier/SKILL.md") && paths.includes("dossier/modele.md")), `le dossier doit rester dans un seul envoi, paquets ${JSON.stringify(requests.map((paths) => paths.filter((path) => path !== "pad.txt")))}`);
});

test("a protected folder entry is explained without importing it", async () => {
  const folder = "/tmp/dossier-accorde";
  const { root } = mount({
    state: {
      skills: { by_bot: { bot: [] } },
      demonstrations: { links: [] },
      conversations: [],
      local: { folders: [folder] },
    },
    api(path) {
      if (path !== "/api/skills/import-folder") return { versions: [] };
      return { report: [{ file_name: "donnees/secret.md", status: { status: "skipped", reason: "app-data" } }] };
    },
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Importer un dossier").listeners.click();
  await flushImport();
  const row = descendants(root).find((el) => el.dataset.status === "skipped");
  assert.match(row.textContent, /donnees\/secret\.md/);
  assert.match(row.textContent, /données d’Ezerbot/);
});

test("the folder button does not ask the browser to walk a directory", () => {
  const source = readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8");
  assert.equal(source.includes("webkitdirectory"), false);
  assert.equal(source.includes("skills-directory"), false);
});

test("Importer un dossier posts a granted folder and shows a real symlink skip", async () => {
  const folder = "/tmp/dossier-accorde";
  const { root, requests } = mount({
    state: {
      skills: { by_bot: { bot: [] } },
      demonstrations: { links: [] },
      conversations: [],
      local: { folders: [folder] },
    },
    api(path, body) {
      if (path !== "/api/skills/import-folder") return { versions: [] };
      assert.equal(body.bot_id, "bot");
      assert.equal(body.path, folder);
      assert.equal(body.files, undefined);
      assert.equal(JSON.stringify(body).includes("base64"), false);
      return {
        report: [
          { file_name: "dossier/SKILL.md", status: { status: "written", skill_id: "s" } },
          { file_name: "dossier/modele.md", status: { status: "annex", skill_id: "s" } },
          { file_name: "dossier/lien.md", status: { status: "skipped", reason: "symlink" } },
        ],
      };
    },
  });
  assert.equal(descendants(root).some((el) => el.name === "skills-directory"), false);
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Importer un dossier").listeners.click();
  await flushImport();
  assert.equal(requests.some((call) => call.path === "/api/skills/import"), false);
  assert.equal(requests.filter((call) => call.path === "/api/skills/import-folder").length, 1);
  const skipped = descendants(root).find((el) => el.dataset.status === "skipped");
  assert.match(skipped.textContent, /dossier\/lien\.md/);
  assert.match(skipped.textContent, /lien symbolique/);
  assert.equal(descendants(root).find((el) => el.dataset.point === "import").dataset.done, "true");
});

test("Importer un dossier asks for a granted folder before reading anything", () => {
  let note = "";
  const { root, requests } = mount({ notify(message) { note = message; } });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Importer un dossier").listeners.click();
  assert.equal(note, "Accordez d’abord ce dossier dans les fichiers locaux.");
  assert.equal(requests.some((call) => String(call.path).includes("import")), false);
});

test("several granted folders are chosen before the folder import", async () => {
  const first = "/tmp/premier";
  const second = "/tmp/second";
  const { root, requests } = mount({
    state: {
      skills: { by_bot: { bot: [] } },
      demonstrations: { links: [] },
      conversations: [],
      local: { folders: [first, second] },
    },
    api(path, body) {
      if (path !== "/api/skills/import-folder") return { versions: [] };
      return { report: [{ file_name: "dossier/SKILL.md", status: { status: "written", skill_id: "s" } }] };
    },
  });
  descendants(root).find((el) => el.tagName === "button" && el.textContent === "Importer un dossier").listeners.click();
  assert.equal(requests.some((call) => call.path === "/api/skills/import-folder"), false);
  descendants(root).find((el) => el.tagName === "button" && el.textContent === second).listeners.click();
  await flushImport();
  assert.equal(requests.find((call) => call.path === "/api/skills/import-folder").body.path, second);
});
