import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Exercise the actual shell router with a small DOM adapter, like delegation-ui.test.js.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const navigateSource = shell.slice(shell.indexOf("  async function navigateLink("), shell.indexOf("  async function copyLink("));
const guard = "Fermez l’éditeur en cours, puis rouvrez ce lien. Vos saisies sont conservées.";
const focusSource = shell.slice(shell.indexOf("  function focusLinkTarget("), shell.indexOf("  async function navigateLink("));

test("settings links reveal the section heading, even for a section taller than the dialog", () => {
  const calls = [], classes = new Set();
  let clearHighlight;
  const focus = runInNewContext(`(${focusSource})`, { setTimeout: (callback) => { clearHighlight = callback; } });
  const target = { scrollIntoView: (options) => calls.push(["scroll", options.block]),
    focus: (options) => calls.push(["focus", options.preventScroll]),
    classList: { add: (name) => classes.add(name), remove: (name) => classes.delete(name) } };
  focus(target);
  assert.equal(target.tabIndex, -1);
  assert.deepEqual(calls, [["scroll", "start"], ["focus", true]]);
  assert.equal(classes.has("deep-link-target"), true);
  clearHighlight();
  assert.equal(classes.has("deep-link-target"), false);
  assert.throws(() => focus(null), { message: "Cette section n’est pas disponible dans cette version d’Ezerbot." });
});

function shellRouter({ open = [], botEdited = false, memoryEdited = false } = {}) {
  const dialogs = Object.fromEntries(["bot-dialog", "rename-dialog", "settings-dialog", "skills-dialog", "routines-dialog", "document-save", "document-zoom"].map((id) => [id, {
    id, open: open.includes(id), close() { this.open = false; },
  }]));
  const bots = [{ id: "grokbot" }, { id: "redacteur" }];
  const ctx = { chosenBotId: "redacteur", notices: [], text: null };
  const navigate = runInNewContext(`(${navigateSource})`, {
    $: (id) => id === "bot-extensions" ? { querySelector: (selector) => memoryEdited && selector === ".memory-editor:not([hidden])" ? {} : null } : dialogs[id],
    document: { querySelector(selector) {
      if (selector.includes("dialog.gb-file-save:not(.gb-file-zoom)[open]") && dialogs["document-save"].open) return dialogs["document-save"];
      return selector.split(",").map((part) => part.trim().match(/^[#.]([\w-]+)\[open\]$/)?.[1]).map((id) => dialogs[id]).find((dialog) => dialog?.open) || null;
    }, querySelectorAll: () => Object.values(dialogs).filter((dialog) => dialog.open) },
    botFormValues: () => ({ name: botEdited ? "Rédactrice" : "Rédacteur" }), botDialogInitial: JSON.stringify({ name: "Rédacteur" }),
    state: { conversations: [] }, botById: (id) => bots.find((bot) => bot.id === id), liveBots: () => bots,
    chosenBot: () => bots.find((bot) => bot.id === ctx.chosenBotId),
    stagedOnEmptyThread: () => false, sendExtras: () => ({}), selectedId: null, readPreference: () => "",
    previousLinkDrafts: () => [], saveLinkDrafts() {}, selectConversation() {}, draftKey: () => "draft",
    ui: { input: { value: "", focus() {} } }, writePreference() {}, saveDraft() {}, render() {},
    setComposerText: (text) => { ctx.text = text; }, notify: (message) => ctx.notices.push(message),
    set chosenBotId(value) { ctx.chosenBotId = value; }, get chosenBotId() { return ctx.chosenBotId; },
  });
  return { ctx, dialogs, navigate };
}

test("an incoming link never closes a rename in progress", async () => {
  const r = shellRouter({ open: ["rename-dialog"] });
  await assert.rejects(r.navigate({ kind: "settings", id: "shortcuts" }), { message: guard });
  assert.equal(r.dialogs["rename-dialog"].open, true);
});

test("an edited bot sheet blocks the link; an untouched one is simply replaced", async () => {
  const edited = shellRouter({ open: ["bot-dialog"], botEdited: true });
  await assert.rejects(edited.navigate({ kind: "conversation", id: "missing" }), { message: guard });
  assert.equal(edited.dialogs["bot-dialog"].open, true);

  const viewed = shellRouter({ open: ["bot-dialog"] });
  await assert.rejects(viewed.navigate({ kind: "conversation", id: "missing" }), { message: "Cette conversation est introuvable ou inaccessible sur ce Mac." });
  assert.equal(viewed.dialogs["bot-dialog"].open, false);
});

test("a memory correction inside an unchanged bot sheet remains protected", async () => {
  const r = shellRouter({ open: ["bot-dialog"], memoryEdited: true });
  await assert.rejects(r.navigate({ kind: "settings", id: "shortcuts" }), { message: guard });
  assert.equal(r.dialogs["bot-dialog"].open, true);
});

test("skill and routine editors block incoming navigation without closing their drafts", async () => {
  for (const id of ["skills-dialog", "routines-dialog"]) {
    const r = shellRouter({ open: [id] });
    await assert.rejects(r.navigate({ kind: "settings", id: "shortcuts" }), { message: guard });
    assert.equal(r.dialogs[id].open, true);
  }
});

test("an incoming link preserves a generated document destination but may close its preview", async () => {
  const saving = shellRouter({ open: ["document-save"] });
  saving.dialogs["document-save"].destination = "/tmp/Résumé à conserver.md";
  await assert.rejects(saving.navigate({ kind: "compose", text: "Autre brouillon" }), { message: guard });
  assert.equal(saving.dialogs["document-save"].open, true);
  assert.equal(saving.dialogs["document-save"].destination, "/tmp/Résumé à conserver.md");
  assert.equal(saving.ctx.text, null);
  const preview = shellRouter({ open: ["document-zoom"] });
  await preview.navigate({ kind: "compose", text: "Autre brouillon" });
  assert.equal(preview.dialogs["document-zoom"].open, false);
  assert.equal(preview.ctx.text, "Autre brouillon");
});

test("restoring an open bot menu never pulls the settings scroll back to it", () => {
  const source = shell.slice(shell.indexOf('      const menu = element("details", "bot-link-menu");'), shell.indexOf("      row.append(botAvatar(bot), copy, edit, menu);"));
  const build = (restored) => runInNewContext(`(() => { ${source}; return {menu, actions}; })()`, {
    bot: { id: "redacteur", name: "Rédacteur" }, openMenu: restored ? { dataset: { botId: "redacteur" } } : null,
    icon() {}, copyLink() {}, element: () => ({ dataset: {}, handlers: {}, reveals: 0,
      append() {}, setAttribute() {}, addEventListener(name, callback) { this.handlers[name] = callback; },
      scrollIntoView() { this.reveals++; } }),
  });
  const restored = build(true);
  restored.menu.handlers.toggle();
  assert.equal(restored.actions.reveals, 0);
  restored.menu.open = false; restored.menu.handlers.toggle();
  restored.menu.open = true; restored.menu.handlers.toggle();
  assert.equal(restored.actions.reveals, 1, "a later user opening is still revealed");
  const fresh = build(false);
  fresh.menu.open = true; fresh.menu.handlers.toggle();
  assert.equal(fresh.actions.reveals, 1);
});

test("a compose link without a bot keeps the bot the user chose", async () => {
  const r = shellRouter();
  await r.navigate({ kind: "compose", text: "Bonjour" });
  assert.equal(r.ctx.chosenBotId, "redacteur");
  assert.equal(r.ctx.text, "Bonjour");
  assert.deepEqual(r.ctx.notices, ["Texte placé dans le brouillon. Rien n’a été envoyé."]);

  await r.navigate({ kind: "compose", text: "Salut", agent: "grokbot" });
  assert.equal(r.ctx.chosenBotId, "grokbot");
});
