import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "node:test";
import { documentToolView } from "./generated-presentation.js";

const { launchFixture } = createRequire(import.meta.url)("../../ui/wrapper-chrome-fixture.cjs");

const component = readFileSync(new URL("./generated-document.jsx", import.meta.url), "utf8");
test("both document dialogs synchronize React state on native close, including Escape and shell navigation", () => {
  assert.equal((component.match(/ref=\{dialog\} onClose=\{onClose\}/g) || []).length, 2);
  assert.doesNotMatch(component, /onCancel=\{onClose\}/);
  assert.match(component, /view\.summary !== view\.failure\.message/);
  assert.equal(documentToolView({ call: { name: "document_action", arguments: { action: "save" } } }, { status: "running" }).note, "L’action est en cours sur votre Mac.");
});

// One conversation per format, so the thumbnails of paged and unpaged files are both measured.
const documents = [
  { title: "Document PDF", name: "Résumé", format: "pdf", content: "Été à Québec — Cœur : 12 €" },
  { title: "Document DOCX", name: "Compte rendu", format: "docx", content: "# Été à Québec\n- Cœur : 12 €" },
  { title: "Document PPTX", name: "Présentation", format: "pptx", content: "Été à Québec\nCœur : 12 €\n---\nDeuxième diapositive" },
  { title: "Document MD", name: "Notes", format: "md", content: "# Notes\nÉté à Québec" },
  { title: "Document CSV", name: "Budget", format: "csv", content: "[[\"Été\",\"Coût\"],[\"Cœur\",\"12 €\"]]" },
  { title: "Document SVG", name: "Schéma", format: "svg", content: "flowchart LR\nA[Été] -->|puis| B(Cœur)" },
];

test("rendered GeneratedDocument reopens after dialog.close() and survives incoming links", { timeout: 60000 }, async (t) => {
  const { url, evaluate } = await launchFixture(t, { documents });
  assert.match(url, /^http:\/\/127\.0\.0\.1:\d+\/?$/);
  // Same page functions as before, now run by CDP in the page of the isolated Chrome.
  const inPage = (source) => evaluate(`(${source})()`);
  await inPage(`() => { if (location.origin !== ${JSON.stringify(new URL(url).origin)}) throw new Error("La page ouverte n’est pas la fixture"); return true; }`);
  await inPage(`async () => {
    const pause = () => new Promise(resolve => setTimeout(resolve, 30));
    const expect = (value, message) => { if (!value) throw new Error(message); };
    const until = async (predicate, message) => {
      for (let n = 0; n < 150; n++) { if (predicate()) return; await pause(); }
      throw new Error(message);
    };
    await until(() => window.GrokbotPanels?.state?.conversations?.length, "Fixture non chargée");
    const panels = window.GrokbotPanels;
    const conversations = panels.state.conversations.filter(c => c.messages?.some(m => m.payload?.outcome?.output?.id?.startsWith("generated_")));
    const conversation = conversations[0];
    expect(conversation, "La fixture doit contenir un résultat create_document");
    for (const thread of conversations) {
      panels.openConversation(thread.id);
      const message = thread.messages.find(m => m.payload?.outcome?.output?.id?.startsWith("generated_"));
      await until(() => document.querySelector('[data-card-kind="generated-document"][data-message-id="' + CSS.escape(message.id) + '"]'), "Carte non rendue");
      const cards = [...document.querySelectorAll(".gb-generated-file")];
      await until(() => cards.every(card => { const image = card.querySelector("img"); return image?.complete && image.naturalWidth > 0; }), "Miniature non chargée");
      for (const card of cards) {
        const image = card.querySelector("img"), rect = image.getBoundingClientRect();
        const paged = /^(PDF|DOCX|PPTX|MD) ·/.test(card.querySelector(".gb-file-metadata").textContent);
        expect(rect.width > 2 && rect.height >= (paged ? 80 : 3), "Miniature invisible ou trop petite : " + card.querySelector("strong").textContent + " " + rect.width + "×" + rect.height);
      }
    }
    panels.openConversation(conversation.id);
    const firstMessage = conversation.messages.find(m => m.payload?.outcome?.output?.id?.startsWith("generated_"));
    await until(() => document.querySelector('[data-card-kind="generated-document"][data-message-id="' + CSS.escape(firstMessage.id) + '"]'), "GeneratedDocument non rendu");
    const saveButton = () => document.querySelector(".gb-generated-file .gb-file-actions button:last-child");
    const previewButton = () => document.querySelector(".gb-generated-file .gb-file-preview-button");
    const save = () => document.querySelector("dialog.gb-file-save:not(.gb-file-zoom)");
    const preview = () => document.querySelector("dialog.gb-file-zoom");
    for (const [button, dialog] of [[saveButton, save], [previewButton, preview]]) {
      for (let attempt = 0; attempt < 2; attempt++) {
        button().click();
        await until(() => dialog()?.open, "Le dialogue ne se rouvre pas");
        const previous = dialog(); previous.close();
        await until(() => !previous.isConnected, "close n’a pas réinitialisé l’état React");
      }
    }
    saveButton().click();
    await until(() => save()?.open && save().querySelector("input").value, "Destination non chargée");
    const input = save().querySelector("input");
    const destination = "/tmp/Destination à conserver.md";
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, destination);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await pause();
    window.EzerbotDeepLinks.receive("ezerbot://app/v1/thread?id=" + encodeURIComponent(conversation.id));
    await until(() => document.getElementById("link-error-dialog")?.open, "Le lien doit protéger la destination");
    expect(save()?.open && input.value === destination, "La destination saisie a été perdue");
    document.getElementById("link-error-dialog").close();
    save().close();
    await until(() => !save(), "Le dialogue d’enregistrement reste monté");
    previewButton().click();
    await until(() => preview()?.open, "Aperçu non ouvert");
    window.EzerbotDeepLinks.receive("ezerbot://app/v1/settings?id=appearance");
    await until(() => document.getElementById("settings-dialog")?.open && !preview(), "Le lien ne ferme pas proprement l’aperçu");
    document.getElementById("settings-dialog").close();
    previewButton().click();
    await until(() => preview()?.open, "Aperçu bloqué après le lien entrant");
    preview().close();
    await until(() => !preview(), "Aperçu toujours monté après close");
    return "dialog.close, re-ouverture, destination protégée et navigation : OK";
  }`);
});
