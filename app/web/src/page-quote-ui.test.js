import assert from "node:assert/strict";
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

// Même allègement que sources-ui.test.js : ni ProseMirror (mentions) ni cartes, sans lien avec la puce.
function withoutMentionsAndCards() {
  return {
    name: "without-mentions-and-cards",
    enforce: "pre",
    transform(code, id) {
      if (!id.endsWith("/src/Thread.jsx")) return null;
      return code
        .replace('import { splitMentionText } from "./mentions.js";\n', "function splitMentionText(text) { return [{ text }]; }\n")
        .replace(
          /const cards = new Map\(Object\.values\(import\.meta\.glob\("\.\/cards\/\*\.jsx", \{ eager: true \}\)\)\s*\.filter\(\(module\) => module\.kind && module\.Card\)\s*\.map\(\(module\) => \[module\.kind, module\.Card\]\)\);/,
          "const cards = new Map();",
        );
    },
  };
}

const vite = await createTestServer({ plugins: [withoutMentionsAndCards(), react()] });
after(() => vite.close());
const { Thread } = await vite.ssrLoadModule("/src/Thread.jsx");
const { PageQuote } = await vite.ssrLoadModule("/src/page-quote.jsx");

const anchor = { page_id: "page_forfaits", title: "Trois forfaits", passage: "Forfait B" };
const sent = (extra = {}) => ({ id: "m1", role: "user", body: "Mets-le à 21 €", state: "accepted", created_at: "2026-10-24T10:00:00Z", payload: {}, ...extra });
const thread = (message, props = {}) => renderToStaticMarkup(React.createElement(Thread, { messages: [message], engineConnected: true, ...props }));
const chips = (html) => [...html.matchAll(/<button type="button" class="gb-quote-chip"([^>]*)><span class="gb-quote-text">([^<]*)<\/span><\/button>/g)];

test("la bulle d’un message envoyé avec une ancre montre la puce « Page « Titre » — « passage » » au-dessus du texte", () => {
  const html = thread(sent({ payload: { page_anchor: anchor } }));
  const [chip] = chips(html);
  assert.equal(chips(html).length, 1);
  assert.equal(chip[2], "Page « Trois forfaits » — « Forfait B »");
  assert.match(chip[1], /title="Page « Trois forfaits » — « Forfait B »"/);
  assert.match(chip[1], /aria-label="Ouvrir Page « Trois forfaits » — « Forfait B »"/);
  assert.ok(html.indexOf("gb-quote-chip") < html.indexOf("gb-bubble"), "la puce est avant la bulle");
  assert.match(html, /Mets-le à 21 €/);
});

test("un envoi en cours porte l’ancre sans titre : la puce garde le passage, sans « Page «  » vide", () => {
  const html = thread(sent({ state: "queued", payload: {}, page_anchor: { page_id: "page_forfaits", source_version: "a".repeat(64), passage: "Forfait B" } }));
  assert.equal(chips(html)[0][2], "Page — « Forfait B »");
});

test("un message sans ancre n’a pas de puce", () => {
  assert.equal(chips(thread(sent())).length, 0);
  assert.equal(chips(thread(sent({ payload: { page_anchor: null } }))).length, 0);
});

test("le clic sur la puce demande d’ouvrir la Page de l’ancre", () => {
  const opened = [];
  const chip = PageQuote({ anchor, onOpenPage: (id) => opened.push(id) });
  chip.props.onClick();
  assert.deepEqual(opened, ["page_forfaits"]);
});

test("le passage s’affiche sur une ligne, sans retours à la ligne ni tabulations", () => {
  const html = thread(sent({ payload: { page_anchor: { ...anchor, passage: "Forfait B\t19 €\nForfait C" } } }));
  assert.equal(chips(html)[0][2], "Page « Trois forfaits » — « Forfait B 19 € Forfait C »");
});
