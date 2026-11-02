const { test } = require("node:test");
const assert = require("node:assert/strict");
const { mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { launchFixture } = require("./wrapper-chrome-fixture.cjs");

async function until(read) {
  const end = Date.now() + 20000;
  while (Date.now() < end) {
    if (await read()) return;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.fail("draft condition never became true");
}
async function open(t, databaseDirectory) {
  const app = await launchFixture(t, { databaseDirectory, navigate: false });
  await app.cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: 'window.EzerbotShell = { platform: "linux", unavailablePanels: [] };' });
  await app.cdp.send("Page.navigate", { url: app.url });
  await until(() => app.evaluate('Boolean(window.GrokbotPanels?.connected && document.querySelector(".gb-editor")?.isContentEditable)'));
  return app;
}
async function type(app, text) {
  await app.evaluate('document.querySelector(".gb-editor").focus()');
  await app.cdp.send("Input.insertText", { text });
}
const read = app => app.evaluate('GrokbotPanels.api("/api/drafts").then(data => data.drafts)');

test("two shell starts keep rich drafts and attachment handoff on the same database", { timeout: 90000 }, async t => {
  const directory = mkdtempSync(join(tmpdir(), "ezerbot-draft-reopen-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  let id, document, homeDocument, text, firstUrl;
  await t.test("type, hand off through a document and acknowledge both targets", async t => {
    const app = await open(t, directory); firstUrl = app.url;
    await type(app, "@");
    await until(() => app.evaluate('Boolean(document.querySelector(".gb-mention-menu [role=option]"))'));
    await app.evaluate('document.querySelector(".gb-mention-menu [role=option]").click()');
    await type(app, "texte durable du fil");
    document = await app.evaluate('GrokbotPanels.draftDocument()');
    text = await app.evaluate('GrokbotPanels.draft()');
    assert.ok(JSON.stringify(document).includes('"botMention"'));
    await app.evaluate('GrokbotPanels.refresh()');
    await app.evaluate(`(() => {
      const transfer = new DataTransfer();
      transfer.items.add(new File(["pièce durable"], "preuve.txt", { type: "text/plain" }));
      document.getElementById("composer-form").dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    })()`);
    await until(() => app.evaluate('Boolean(document.querySelector(".attachment-chip[data-status=ready]"))')).catch(async error => {
      assert.fail(`${error.message}: ${await app.evaluate('JSON.stringify({ chips: document.querySelector(".attachment-list")?.textContent, toast: document.getElementById("toast").textContent, hint: document.getElementById("composer-hint").textContent, conversations: GrokbotPanels.state.conversations.map(c => c.id) })')}`);
    });
    id = await app.evaluate('GrokbotPanels.state.conversations[0].id');
    assert.equal(await app.evaluate('GrokbotPanels.draft()'), text);
    await until(async () => (await read(app)).some(d => d.conversation_id === id && d.text === text));
    await app.evaluate('document.getElementById("new-chat").click()');
    assert.equal(await app.evaluate('GrokbotPanels.draft()'), "");
    assert.equal((await read(app)).some(d => d.conversation_id === null && d.text), false);
    await type(app, "texte durable accueil");
    homeDocument = await app.evaluate('GrokbotPanels.draftDocument()');
    await until(async () => (await read(app)).some(d => d.conversation_id === null && d.text === "texte durable accueil"));
  });
  await t.test("reopen with fresh incognito storage, then delete the thread", async t => {
    const app = await open(t, directory);
    assert.notEqual(app.url, firstUrl, "the wrapper still uses a random port");
    assert.equal(await app.evaluate('localStorage.getItem("grokbot.draft.new")'), null);
    assert.equal(await app.evaluate('GrokbotPanels.draft()'), "texte durable accueil");
    assert.deepEqual(await app.evaluate('GrokbotPanels.draftDocument()'), homeDocument);
    await app.evaluate(`GrokbotPanels.openConversation(${JSON.stringify(id)})`);
    assert.equal(await app.evaluate('GrokbotPanels.draft()'), text);
    assert.deepEqual(await app.evaluate('GrokbotPanels.draftDocument()'), document);
    assert.ok(await app.evaluate('Boolean(document.querySelector(".attachment-chip"))'));
    await app.evaluate('document.getElementById("confirm-delete").click()');
    await until(async () => !(await read(app)).some(d => d.conversation_id === id));
    assert.equal(await app.evaluate('GrokbotPanels.draft()'), "texte durable accueil");
    assert.deepEqual(await app.evaluate('GrokbotPanels.draftDocument()'), homeDocument);
    assert.equal(await app.evaluate('JSON.stringify(GrokbotPanels.state).includes("texte durable")'), false);
  });
});
