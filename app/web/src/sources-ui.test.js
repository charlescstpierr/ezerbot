import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test, after } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
import react from "@vitejs/plugin-react";

const listeners = { addEventListener() {}, removeEventListener() {} };
globalThis.window = { ...listeners, matchMedia: () => ({ matches: false, ...listeners }), innerWidth: 820 };
globalThis.document = {
  documentElement: { dataset: { theme: "dark", accent: "citron" } },
  createElement() { return { getContext: () => ({ fillStyle: "#111111" }) }; },
  ...listeners,
};
globalThis.window.document = globalThis.document;
globalThis.getComputedStyle = () => ({ getPropertyValue: () => "#111111" });
globalThis.MutationObserver = class { observe() {} disconnect() {} };
globalThis.ResizeObserver = class { observe() {} disconnect() {} };

// Le bouton « Copier la réponse » ne montre pas son texte dans le HTML : on relève celui que Thread lui passe.
const copied = [];
globalThis.__copiedTexts = copied;
function recordCopiedText() {
  return {
    name: "record-copied-text",
    enforce: "pre",
    transform(code, id) {
      if (!id.endsWith("/src/ui.jsx")) return null;
      return code.replace("export function CopyIconButton({ text, label, onCopy }) {", "$&\n  if (label === \"Copier la réponse\") globalThis.__copiedTexts.push(text);");
    },
  };
}

// La prose passe par le vrai Streamdown ; seuls ProseMirror (mentions) et les cartes, sans lien avec les sources, sont laissés de côté.
function withoutMentionsAndCards() {
  return {
    name: "without-mentions-and-cards",
    enforce: "pre",
    transform(code, id) {
      if (!id.endsWith("/src/Thread.jsx")) return null;
      return code
        .replace('import { splitMentionText } from "./mentions.js";\n', "function splitMentionText() { return []; }\n")
        .replace(
          /const cards = new Map\(Object\.values\(import\.meta\.glob\("\.\/cards\/\*\.jsx", \{ eager: true \}\)\)\s*\.filter\(\(module\) => module\.kind && module\.Card\)\s*\.map\(\(module\) => \[module\.kind, module\.Card\]\)\);/,
          "const cards = new Map();",
        );
    },
  };
}

const vite = await createTestServer({ plugins: [withoutMentionsAndCards(), recordCopiedText(), react()] });
after(() => vite.close());
const { Thread } = await vite.ssrLoadModule("/src/Thread.jsx");

const { SourceCard, SourcePill, SourcesContext, SourcesLine, openSource, placeSourceCard } = await vite.ssrLoadModule("/src/sources.jsx");
const placement = { placeSourceCard };
const sources = await vite.ssrLoadModule("/src/sources.js");
const { exportMessage } = await vite.ssrLoadModule("/src/teach.js");

const mail = { n: 1, type: "courriel", titre: "Loyer d’octobre", auteur: "Marie Tremblay <marie@exemple.test>", date: "2026-10-01", compte: "moi@exemple.test", identifiant: "msg-18f3" };
const sheet = { n: 2, type: "feuille", titre: "Budget 2026", date: "2026-09-12T14:30:00Z", compte: "moi@exemple.test", identifiant: "sheet-77", plage: "lignes 1 à 40 (feuille entière)" };
const file = { n: 3, type: "bibliotheque", titre: "bail.pdf", identifiant: "file-9" };
const answer = (body, extra = {}) => ({
  id: "reply-1", role: "assistant", state: "echoed", body,
  payload: { bot_id: "bot-1", sources: [mail, sheet, file], citations: [], ...extra },
});

function thread(message, props = {}) {
  return renderToStaticMarkup(React.createElement(Thread, { messages: [message], engineConnected: true, ...props }));
}
function inContext(message, node, onOpenSource) {
  return renderToStaticMarkup(React.createElement(SourcesContext.Provider, { value: { message, onOpenSource } }, node));
}
const buttons = (html) => [...html.matchAll(/<button\b[^>]*>([\s\S]*?)<\/button>/g)].map((match) => match[1].replace(/<[^>]+>/g, "").trim());

test("[1] devient une pastille cliquable, le chiffre du texte restant à sa place", () => {
  const html = thread(answer("Le loyer est de 1 200 $ [1] et le budget le confirme [2, 3].", { citations: [{ n: 1, etat: "verifiee" }, { n: 2, etat: "verifiee" }, { n: 3, etat: "verifiee" }] }));
  const pills = [...html.matchAll(/<button type="button" class="gb-cite-pill"([^>]*)>(\d+)<\/button>/g)];
  assert.deepEqual(pills.map((pill) => pill[2]), ["1", "2", "3"]);
  assert.match(pills[0][1], /aria-label="Source 1 : Loyer d’octobre"/);
  assert.match(pills[0][1], /aria-expanded="false"/);
  assert.match(pills[0][1], /title="Loyer d’octobre · Marie Tremblay &lt;marie@exemple\.test&gt; · 1 oct\. 2026"/);
  assert.match(html, /Le loyer est de 1 200 \$ /);
  assert.equal(html.includes("sourcepill"), false);
  assert.equal(html.includes("[1]"), false);
});

test("le clic sur la pastille affiche le type, l’objet, l’expéditeur et la date", () => {
  const card = inContext(answer("Voir [1]."), React.createElement(SourcePill, { n: "1", defaultOpen: true }));
  assert.match(card, /aria-expanded="true"/);
  assert.match(card, /class="gb-source-card"/);
  assert.match(card, /Courriel/);
  assert.match(card, /<strong class="gb-source-title">Loyer d’octobre<\/strong>/);
  assert.match(card, /Expéditeur<\/span><span>Marie Tremblay &lt;marie@exemple\.test&gt;/);
  assert.match(card, /Date<\/span><span>1 oct\. 2026/);
  assert.deepEqual(buttons(card).slice(1), ["Ouvrir la source", "Fermer"]);
  const sheetCard = inContext(answer("Voir [2]."), React.createElement(SourcePill, { n: "2", defaultOpen: true }));
  assert.match(sheetCard, /Feuille de calcul/);
  assert.match(sheetCard, /Plage<\/span><span>lignes 1 à 40 \(feuille entière\)/);
});

test("[7] sans source est barré, gris, avec l’infobulle « Source introuvable »", () => {
  const html = thread(answer("Un chiffre inventé [7].", { citations: [{ n: 7, etat: "introuvable", libelle: "source introuvable" }] }));
  assert.match(html, /<span class="gb-cite gb-cite-missing" tabindex="0" title="Source introuvable" data-tip="Source introuvable" aria-label="7 : Source introuvable">\[7\]<\/span>/);
  assert.equal(html.includes("gb-cite-pill"), false);
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  assert.match(css, /\.gb-cite-missing\s*\{[^}]*line-through/);
  assert.match(css, /\.gb-cite-missing\s*\{[^}]*color:\s*var\(--muted\)/);
});

test("une réponse sans source garde son texte tel quel, et le code ne cite rien", () => {
  const plain = thread({ id: "r", role: "assistant", state: "echoed", body: "Tableau a[1] et [2](https://x.test).", payload: {} });
  assert.equal(plain.includes("gb-cite"), false);
  assert.equal(plain.includes("gb-sources"), false);
  const code = thread(answer("Dans `a [1]` rien, mais ici [1].\n\n```\n[2]\n```\n"));
  assert.equal((code.match(/class="gb-cite-pill"/g) || []).length, 1);
});

test("« Sources (3) » est repliée sous la réponse et déplie les trois sources", () => {
  const closed = thread(answer("Trois sources [1] [2] [3]."));
  assert.match(closed, /<details class="gb-sources"><summary>/);
  assert.match(closed, /Sources <span class="gb-sources-count">3<\/span>/);
  assert.equal(closed.includes("<details class=\"gb-sources\" open"), false);
  assert.equal((closed.match(/class="gb-sources-icons"/g) || []).length, 1);
  const open = renderToStaticMarkup(React.createElement(SourcesLine, { message: answer("x"), defaultOpen: true }));
  assert.match(open, /<details class="gb-sources" open="">/);
  assert.equal((open.match(/<li data-source="/g) || []).length, 3);
  for (const text of ["Loyer d’octobre", "Budget 2026", "bail.pdf"]) assert.ok(open.includes(text), text);
  assert.equal((open.match(/Ouvrir la source/g) || []).length, 3);
  const none = renderToStaticMarkup(React.createElement(SourcesLine, { message: { payload: { sources: [] } } }));
  assert.equal(none, "");
});

test("« Ouvrir la source » appelle l’ouverture de la source cliquée, avec son identifiant", async () => {
  const calls = [];
  const message = answer("Voir [1].");
  const reason = await openSource(mail, async (source, from) => { calls.push([source.identifiant, from.id]); return { ok: true }; }, message);
  assert.equal(reason, null);
  assert.deepEqual(calls, [["msg-18f3", "reply-1"]]);
});

test("une source inaccessible affiche le message, et rien ne casse", async () => {
  const disconnected = await openSource(mail, async () => ({ ok: false, message: sources.DISCONNECTED }), answer("x"));
  assert.equal(disconnected, "Ce compte n’est plus connecté — reconnecte-le pour ouvrir la source.");
  assert.equal(await openSource(mail, async () => { throw new Error("réseau"); }, answer("x")), "La source n’a pas pu être ouverte.");
  assert.equal(await openSource(mail, undefined, answer("x")), sources.OPEN_UNAVAILABLE);
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  assert.match(css, /\.gb-source-problem\s*\{/);
});

test("l’ouverture choisit le bon lien, ou dit pourquoi rien ne s’ouvre", () => {
  const connected = [{ id: "gmail", status: "connected", account: "moi@exemple.test", accounts: [] }, { id: "drive", status: "connected", account: "moi@exemple.test", accounts: [] }];
  assert.equal(sources.sourceOpening(mail, { connectors: connected }).url, "https://mail.google.com/mail/u/moi%40exemple.test/#all/msg-18f3");
  assert.equal(sources.sourceOpening(sheet, { connectors: connected }).url, "https://docs.google.com/spreadsheets/d/sheet-77/edit?authuser=moi%40exemple.test");
  assert.deepEqual(sources.sourceOpening(file, { connectors: connected }), { library: "file-9" });
  const gone = [{ id: "gmail", status: "disconnected", accounts: [] }];
  assert.equal(sources.sourceOpening(mail, { connectors: gone }).problem, sources.DISCONNECTED);
  // Agenda et Drive lisent par Gmail : un Gmail connecté seul suffit, un autre compte ne suffit pas.
  const gmailOnly = [{ id: "gmail", status: "connected", account: "moi@exemple.test", accounts: [] }, { id: "drive", status: "disconnected", accounts: [] }];
  assert.ok(sources.sourceOpening(sheet, { connectors: gmailOnly }).url.startsWith("https://docs.google.com/spreadsheets/"));
  assert.ok(sources.sourceOpening({ n: 5, type: "evenement", titre: "Visite", date: "2026-10-12", compte: "moi@exemple.test", identifiant: "ev-1" }, { connectors: gmailOnly }).url.startsWith("https://calendar.google.com/"));
  const visit = sources.sourceOpening({ n: 5, type: "evenement", titre: "Visite", date: "2026-10-12", compte: "moi@exemple.test", identifiant: "ev1abc" }, { connectors: gmailOnly }).url;
  assert.equal(visit, `https://calendar.google.com/calendar/u/moi%40exemple.test/event?eid=${encodeURIComponent(Buffer.from("ev1abc moi@exemple.test").toString("base64").replace(/=+$/, ""))}`);
  assert.ok(sources.sourceOpening({ n: 6, type: "evenement", titre: "Sans compte", date: "2026-10-12", identifiant: "ev2" }, { connectors: [] }).problem);
  assert.equal(sources.sourceOpening(sheet, { connectors: [{ id: "gmail", status: "connected", account: "autre@exemple.test", accounts: [] }] }).problem, sources.DISCONNECTED);
  assert.equal(sources.sourceOpening(mail, { connectors: [{ id: "gmail", status: "connected", account: "autre@exemple.test", accounts: [] }] }).problem, sources.DISCONNECTED);
  assert.equal(sources.sourceOpening(mail, { connectors: [{ id: "gmail", status: "reconnect", account: "moi@exemple.test", accounts: [{ account: "moi@exemple.test", status: "connected" }] }] }).url.startsWith("https://mail.google.com/"), true);
  assert.equal(sources.sourceOpening({ ...mail, identifiant: undefined }, { connectors: connected }).problem, sources.NO_LINK);
  assert.equal(sources.sourceOpening({ n: 4, type: "piece_jointe", titre: "devis.docx", identifiant: "d" }, {}).problem, sources.ATTACHMENT_NO_LINK);
});

test("copier la réponse inclut la liste des sources, et l’export aussi", () => {
  const message = answer("Le loyer est de 1 200 $ [1], l’an passé 1 150 $ [7].", { citations: [{ n: 1, etat: "verifiee" }, { n: 7, etat: "introuvable", libelle: "source introuvable" }] });
  const notes = sources.textWithSources(message);
  assert.equal(notes, "Le loyer est de 1 200 $ [1], l’an passé 1 150 $ [7].\n\nSources :\n[1] Courriel : Loyer d’octobre · Marie Tremblay <marie@exemple.test> · 1 oct. 2026\n[7] Source introuvable");
  assert.equal(exportMessage(message).body, notes);
  const plain = { id: "r", role: "assistant", state: "echoed", body: "Bonjour [1].", payload: {} };
  assert.equal(exportMessage(plain), plain);
  assert.equal(sources.textWithSources({ ...message, body: "Rien à citer." }), "Rien à citer.");
  copied.length = 0;
  const html = thread(message);
  assert.ok(html.includes('aria-label="Copier la réponse"'));
  assert.deepEqual(copied, [notes]);
  // Une vraie réponse du moteur porte parts et run_id : orderThread la découpe, la copie garde les sources.
  copied.length = 0;
  const live = answer(message.body, { ...message.payload, run_id: "run-1", parts: [{ body: message.body }] });
  thread(live);
  assert.deepEqual(copied, [notes]);
});

test("le code n’est pas de la prose : clôtures ``` et ~~~, bloc indenté, mais une liste garde ses citations", () => {
  const code = "Avant [1].\n\n~~~js\nconst a = [2];\n~~~\n\n````\n[3]\n```\n[4]\n````\n\n    const b = [5];\n    [6]\n\nAprès [7].\n\n- point\n\n    suite du point [8]\n";
  assert.deepEqual(sources.citedNumbers(code), [1, 7, 8]);
  const marked = sources.citeMarkdown(code);
  assert.ok(marked.includes("const a = [2];") && marked.includes("    const b = [5];"));
  assert.equal((marked.match(/<sourcepill /g) || []).length, 3);
  const html = thread(answer(code, { citations: [] }));
  assert.equal(html.includes("&lt;sourcepill"), false);
  assert.equal((html.match(/class="gb-cite-pill"/g) || []).length, 1);
});

test("la carte se pose en fixe, sous la pastille ou au-dessus en bas de fenêtre, dans la fenêtre", () => {
  const { placeSourceCard } = placement;
  const card = { width: 292, height: 120 };
  const viewport = { width: 820, height: 560 };
  assert.deepEqual(placeSourceCard({ left: 100, top: 200, bottom: 216 }, card, viewport), { top: 222, left: 100 });
  assert.deepEqual(placeSourceCard({ left: 700, top: 200, bottom: 216 }, card, viewport), { top: 222, left: 520 });
  assert.deepEqual(placeSourceCard({ left: 100, top: 480, bottom: 496 }, card, viewport), { top: 354, left: 100 });
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  assert.match(css, /\.gb-source-card\[data-floating\]\s*\{[^}]*position:\s*fixed/);
  const open = inContext(answer("Voir [1]."), React.createElement(SourcePill, { n: "1", defaultOpen: true }));
  assert.match(open, /class="gb-source-card"/, "sans page, la carte reste dans le fil");
});

test("un lien de référence n’est pas une citation, mais [1][4] en reste deux", () => {
  const body = "Voir [1][documentation] et [guide][2], puis [3] et [3][4].\n\n[documentation]: https://exemple.test/doc\n[2]: https://exemple.test/guide\n";
  assert.deepEqual(sources.citedNumbers(body), [3, 4]);
  const marked = sources.citeMarkdown(body);
  assert.ok(marked.includes("[1][documentation]") && marked.includes("[guide][2]"));
  assert.equal((marked.match(/<sourcepill /g) || []).length, 3);
  assert.deepEqual(sources.citedNumbers("Voir [1][4]. Et [7]\n\n[7]: https://x.test"), [1, 4]);
});

test("une destination de lien, d’image ou une URL garde ses crochets, la prose voisine garde ses citations", () => {
  const body = "[Doc](https://exemple.test/files/[1]/report) et ![Plan](https://exemple.test/p?x[2]=3) puis https://exemple.test/a/[3]/b et [Autre](https://exemple.test/(x)/[4]) enfin [5] et [6](non fermé";
  assert.deepEqual(sources.citedNumbers(body), [5]);
  const marked = sources.citeMarkdown(body);
  for (const kept of ["files/[1]/report", "p?x[2]=3", "a/[3]/b", "(x)/[4]"]) assert.ok(marked.includes(kept), kept);
  assert.equal((marked.match(/<sourcepill /g) || []).length, 1);
  assert.deepEqual(sources.citedNumbers("Voir (https://a.test/x) [7] et [8]."), [7, 8]);
});

test("la carte et la ligne restent lisibles en clair comme en sombre, au clavier", () => {
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  const block = css.slice(css.indexOf("/* Sources citées"));
  assert.equal(/#[0-9a-f]{3,8}\b/i.test(block.replace(/box-shadow:[^;]*;/g, "")), false, "couleurs par variables, donc adaptées aux deux thèmes");
  assert.match(block, /\.gb-cite-pill:focus-visible/);
  assert.match(block, /\.gb-sources summary:focus-visible/);
  const card = inContext(answer("Voir [1]."), React.createElement(SourceCard, { source: mail, message: answer("x"), onClose() {}, id: "carte" }));
  assert.match(card, /role="group" aria-label="Source 1"/);
});
