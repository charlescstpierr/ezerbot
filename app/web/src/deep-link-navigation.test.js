import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Exercise the actual shell router with a small DOM adapter, like delegation-ui.test.js.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const navigateSource = shell.slice(shell.indexOf("  async function navigateLink("), shell.indexOf("  async function copyLink("));
const guard = "Fermez l’éditeur en cours, puis rouvrez ce lien. Vos saisies sont conservées.";

function shellRouter({ open = [], botEdited = false } = {}) {
  const dialogs = Object.fromEntries(["bot-dialog", "rename-dialog", "settings-dialog"].map((id) => [id, {
    id, open: open.includes(id), close() { this.open = false; },
  }]));
  const bots = [{ id: "grokbot" }, { id: "redacteur" }];
  const ctx = { chosenBotId: "redacteur", notices: [], text: null };
  const navigate = runInNewContext(`(${navigateSource})`, {
    $: (id) => dialogs[id],
    document: { querySelector(selector) {
      return selector.split(",").map((part) => part.trim().match(/^#([\w-]+)\[open\]$/)?.[1]).map((id) => dialogs[id]).find((dialog) => dialog?.open) || null;
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

test("a compose link without a bot keeps the bot the user chose", async () => {
  const r = shellRouter();
  await r.navigate({ kind: "compose", text: "Bonjour" });
  assert.equal(r.ctx.chosenBotId, "redacteur");
  assert.equal(r.ctx.text, "Bonjour");
  assert.deepEqual(r.ctx.notices, ["Texte placé dans le brouillon. Rien n’a été envoyé."]);

  await r.navigate({ kind: "compose", text: "Salut", agent: "grokbot" });
  assert.equal(r.ctx.chosenBotId, "grokbot");
});
