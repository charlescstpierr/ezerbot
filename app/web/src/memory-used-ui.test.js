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

const vite = await createTestServer({ plugins: [withoutMentionsAndCards(), react()] });
after(() => vite.close());
const { Thread } = await vite.ssrLoadModule("/src/Thread.jsx");
const { MemoryUsedPill, openMemory } = await vite.ssrLoadModule("/src/memory-used.jsx");
const { MEMORY_UNAVAILABLE, memoryUsedLabel, recalledIds } = await vite.ssrLoadModule("/src/memory-used.js");

const answer = (extra = {}, state = "echoed") => ({ id: "reply-1", role: "assistant", state, body: "Votre café est noir.", payload: { bot_id: "bot-1", ...extra } });
const thread = (message, props = {}) => renderToStaticMarkup(React.createElement(Thread, { messages: [message], engineConnected: true, ...props }));
const pills = (html) => [...html.matchAll(/<button type="button" class="gb-memory-used"[^>]*>(?:<svg[\s\S]*?<\/svg>)?([^<]*)<\/button>/g)].map((match) => match[1]);

test("1 souvenir rappelé : « 1 souvenir utilisé » ; 2 : « 2 souvenirs utilisés »", () => {
  assert.deepEqual(pills(thread(answer({ recalled_fact_ids: ["f1"] }))), ["1 souvenir utilisé"]);
  assert.deepEqual(pills(thread(answer({ recalled_fact_ids: ["f1", "f2"] }))), ["2 souvenirs utilisés"]);
  assert.equal(memoryUsedLabel(3), "3 souvenirs utilisés");
});

test("aucune pastille sans souvenir rappelé : liste vide, champ absent, valeurs inutilisables", () => {
  for (const payload of [{ recalled_fact_ids: [] }, {}, { recalled_fact_ids: null }, { recalled_fact_ids: "f1" }, { recalled_fact_ids: [null, "", "  ", 7] }]) {
    const html = thread(answer(payload));
    assert.equal(html.includes("gb-memory-used"), false, JSON.stringify(payload));
  }
  assert.equal(thread({ id: "u1", role: "user", state: "sent", body: "Bonjour", payload: { recalled_fact_ids: ["f1"] } }).includes("gb-memory-used"), false);
});

test("une réponse encore en cours n’affiche pas la pastille", () => {
  assert.equal(thread(answer({ recalled_fact_ids: ["f1"] }, "streaming")).includes("gb-memory-used"), false);
});

test("un souvenir cité deux fois ne compte qu’une fois, l’ordre du rappel est gardé", () => {
  assert.deepEqual(recalledIds(answer({ recalled_fact_ids: ["f2", "f1", "f2"] })), ["f2", "f1"]);
  assert.deepEqual(pills(thread(answer({ recalled_fact_ids: ["f2", "f1", "f2"] }))), ["2 souvenirs utilisés"]);
});

test("la pastille se place à côté de « Sources », sans la déranger", () => {
  const source = { n: 1, type: "courriel", titre: "Loyer d’octobre", identifiant: "msg-1" };
  const both = thread(answer({ sources: [source], recalled_fact_ids: ["f1"] }));
  const row = both.slice(both.indexOf('class="gb-provenance"'));
  assert.ok(row.indexOf('class="gb-sources"') !== -1 && row.indexOf('class="gb-sources"') < row.indexOf("gb-memory-used"));
  assert.match(both, /Sources <span class="gb-sources-count">1<\/span>/);
  const only = thread(answer({ sources: [source] }));
  assert.match(only, /<details class="gb-sources">/);
  assert.equal(only.includes("gb-memory-used"), false);
  assert.match(thread(answer({})), /<div class="gb-provenance"><\/div>/);
});

// Rend le composant avec un état de hooks minimal pour qu'un clic s'exécute et que le résultat puisse être relu.
const internals = React.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
function mountPill(props) {
  const state = [];
  const render = () => {
    let slot = 0;
    const previous = internals.H;
    internals.H = {
      useState(initial) { const at = slot++; if (!(at in state)) state[at] = initial; return [state[at], (value) => { state[at] = value; }]; },
      useId: () => "id-1",
    };
    try { return MemoryUsedPill(props); } finally { internals.H = previous; }
  };
  const nodes = (node, found = []) => {
    if (Array.isArray(node)) node.forEach((child) => nodes(child, found));
    else if (node && typeof node === "object" && node.props) { found.push(node); nodes(node.props.children, found); }
    return found;
  };
  return { button: () => nodes(render()).find((node) => node.type === "button"), status: () => nodes(render()).find((node) => node.props.role === "status") };
}

test("le clic demande l’écran Mémoire pour les identifiants de la réponse", async () => {
  const calls = [];
  const message = answer({ recalled_fact_ids: ["f2", "f1"] });
  const pill = mountPill({ message, onOpenMemory: (ids, from) => { calls.push([ids, from]); return { ok: true }; } });
  await pill.button().props.onClick();
  assert.deepEqual(calls, [[["f2", "f1"], message]]);
  assert.equal(pill.status(), undefined, "rien à dire quand l’écran s’ouvre");
});

test("quand l’écran Mémoire ne peut pas s’ouvrir, la pastille le dit en français", async () => {
  const refused = mountPill({ message: answer({ recalled_fact_ids: ["f1"] }), onOpenMemory: () => ({ ok: false, message: "Une action est en cours : réessayez dans un instant." }) });
  await refused.button().props.onClick();
  assert.equal(refused.status().props.children, "Une action est en cours : réessayez dans un instant.");
  assert.equal(await openMemory(["f1"], undefined, answer()), MEMORY_UNAVAILABLE);
  assert.equal(await openMemory(["f1"], () => { throw new Error("boom"); }, answer()), "L’écran Mémoire n’a pas pu s’ouvrir.");
  assert.equal(await openMemory(["f1"], () => ({ ok: false }), answer()), "L’écran Mémoire n’a pas pu s’ouvrir.");
});

test("la pastille s’habille de jetons Galet, sans couleur codée en dur ni texte sous 11,5 px", () => {
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  const rules = css.split("\n").filter((line) => line.startsWith(".gb-memory-used") || line.startsWith(".gb-provenance"));
  assert.ok(rules.length >= 4);
  for (const rule of rules) {
    assert.doesNotMatch(rule, /#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i, rule);
    for (const size of rule.matchAll(/font-size:\s*([\d.]+)px/g)) assert.ok(Number(size[1]) >= 11.5, rule);
    assert.doesNotMatch(rule, /--accent/, "le citron est réservé à Envoyer, à l’élément actif et aux yeux de la mascotte");
  }
  assert.match(css, /\.gb-provenance:empty\s*\{\s*display:\s*none/);
});
