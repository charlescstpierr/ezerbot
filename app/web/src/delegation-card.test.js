import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
import react from "@vitejs/plugin-react";

const vite = await createTestServer({ plugins: [react()] });
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/delegation.jsx");

const BEFORE = { id: "screen:t:1", label: "avant", caption: "Page d’accueil avant", url: "/api/screen/shot?conversation_id=c1&id=screen%3At%3A1" };
const AFTER = { id: "attachment_b", label: "après", caption: "Page d’accueil après", url: "/api/attachments/attachment_b" };
let edge;
function setEdge(captures, status = "done") {
  edge = { id: "d1", child_bot: "b", conversation_id: "c1", objective: "Refaire la page", status: { status, result: "Page modifiée.", ...(captures ? { captures } : {}) } };
}
globalThis.window = { GrokbotDelegation: { version: () => 1, edge: () => edge, botName: () => "Bot Test", hasConversation: () => true } };
const element = () => React.createElement(Card, { message: { id: "m1", payload: { conversation_id: "c1" } }, onAction() {} });
const html = () => renderToStaticMarkup(element());

// Minimal in-memory renderer: expands function components with path-keyed hook
// state so a click handler can run and the tree can be read again.
const internals = React.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
function mount() {
  const state = new Map();
  const effects = [];
  function expand(node, path) {
    if (Array.isArray(node)) return node.map((child, at) => expand(child, `${path}.${at}`));
    if (!node || typeof node !== "object" || !node.type) return node;
    if (typeof node.type === "function") {
      let slot = 0;
      const key = () => `${path}#${slot++}`;
      const previous = internals.H;
      internals.H = {
        useState(initial) {
          const id = key();
          if (!state.has(id)) state.set(id, typeof initial === "function" ? initial() : initial);
          return [state.get(id), (value) => state.set(id, typeof value === "function" ? value(state.get(id)) : value)];
        },
        useRef(initial) {
          const id = key();
          if (!state.has(id)) state.set(id, { current: initial ?? null });
          return state.get(id);
        },
        useEffect(run, deps) { effects.push({ id: key(), run, deps }); },
        useSyncExternalStore: (_subscribe, snapshot) => snapshot(),
      };
      try { return expand(node.type(node.props), `${path}/`); } finally { internals.H = previous; }
    }
    return { ...node, props: { ...node.props, children: expand(node.props.children, path) } };
  }
  const collect = (node, nodes = []) => {
    if (Array.isArray(node)) node.forEach((child) => collect(child, nodes));
    else if (node && typeof node === "object" && node.props) { nodes.push(node); collect(node.props.children, nodes); }
    return nodes;
  };
  // Like React's commit: every stale cleanup first, then the effects whose deps changed.
  function commit() {
    const due = effects.splice(0).filter(({ id, deps }) => {
      const last = state.get(`effect:${id}`);
      return !last || !deps || deps.length !== last.deps.length || deps.some((dep, at) => !Object.is(dep, last.deps[at]));
    });
    for (const { id } of due) state.get(`effect:${id}`)?.cleanup?.();
    for (const { id, run, deps } of due) state.set(`effect:${id}`, { deps, cleanup: run() });
  }
  const view = () => { const nodes = collect(expand(element(), "0")); commit(); return nodes; };
  const text = (node) => Array.isArray(node) ? node.map(text).join("") : typeof node === "object" && node ? text(node.props?.children) : node == null || typeof node === "boolean" ? "" : String(node);
  return {
    find: (type, label) => view().find((node) => node.type === type && node.props["aria-label"] === label),
    all: (type) => view().filter((node) => node.type === type),
    // What the open viewer really displays: its own <img>, not the thumbnails.
    shown: () => collect(view().find((node) => node.type === "dialog")).find((node) => node.type === "img")?.props.src,
    text: (node) => text(node),
  };
}

test("two captures show Avant then Après with their legend, after the visible result", () => {
  setEdge([BEFORE, AFTER]);
  const out = html();
  const row = out.slice(out.indexOf('class="gb-captures"'), out.lastIndexOf("</figure>"));
  const result = out.indexOf('class="gb-delegation-result"');
  assert.ok(result >= 0 && result < out.indexOf('class="gb-captures"'), "the row sits after the visible result");
  assert.doesNotMatch(out, /<details|<summary|<pre/, "the result is prose without a technical disclosure");
  assert.ok(row.indexOf("<strong>Avant</strong>") !== -1 && row.indexOf("<strong>Avant</strong>") < row.indexOf("<strong>Après</strong>"));
  assert.ok(row.indexOf("Page d’accueil avant") < row.indexOf("Page d’accueil après"));
  assert.match(row, /<strong>Avant<\/strong><span>Page d’accueil avant<\/span>/);
  assert.match(row, /<strong>Après<\/strong><span>Page d’accueil après<\/span>/);
  assert.match(row, /<button[^>]*aria-label="Avant : Page d’accueil avant"/);
  assert.match(row, /<button[^>]*aria-label="Après : Page d’accueil après"/);
  assert.match(row, /<img src="\/api\/screen\/shot\?conversation_id=c1&amp;id=screen%3At%3A1" alt="Avant : Page d’accueil avant"/);
  assert.equal((row.match(/<button/g) || []).length, 2, "the thumbnails are real buttons");
});

test("a click on Après opens the existing viewer on that image and the arrow leads to Avant", () => {
  setEdge([BEFORE, AFTER]);
  const page = mount();
  assert.equal(page.all("dialog").length, 0);
  page.find("button", "Après : Page d’accueil après").props.onClick({ currentTarget: { focus() {} } });
  const viewer = page.all("dialog")[0];
  assert.equal(viewer.props["aria-label"], "Visionneuse");
  assert.match(page.text(viewer), /2 sur 2/);
  const shown = page.all("img").filter((img) => img.props.alt === "Après : Page d’accueil après" && img.props.src === AFTER.url);
  assert.equal(shown.length, 2, "the thumbnail and the viewer both show Après");
  assert.equal(page.all("figcaption").map((node) => page.text(node)).at(-1), "Après : Page d’accueil après");
  assert.equal(page.shown(), AFTER.url);
  page.find("button", "Image précédente").props.onClick();
  assert.match(page.text(page.all("dialog")[0]), /1 sur 2/);
  assert.equal(page.shown(), BEFORE.url, "the arrow really swaps the displayed image");
  assert.equal(page.all("figcaption").map((node) => page.text(node)).at(-1), "Avant : Page d’accueil avant");
  page.find("button", "Image suivante").props.onClick();
  assert.match(page.text(page.all("dialog")[0]), /2 sur 2/);
  assert.equal(page.shown(), AFTER.url);
  page.all("dialog")[0].props.onClose();
  assert.equal(page.all("dialog").length, 0);
});

test("a click on Avant opens the viewer on the first image of the list", () => {
  setEdge([BEFORE, AFTER]);
  const page = mount();
  page.find("button", "Avant : Page d’accueil avant").props.onClick({ currentTarget: { focus() {} } });
  assert.match(page.text(page.all("dialog")[0]), /1 sur 2/);
  assert.equal(page.shown(), BEFORE.url);
});

test("closing the viewer returns the focus to the thumbnail that opened it, not to the other one", () => {
  setEdge([BEFORE, AFTER]);
  const page = mount();
  const focused = [];
  const opener = (name) => ({ focus: () => focused.push(name) });
  page.find("button", "Après : Page d’accueil après").props.onClick({ currentTarget: opener("après") });
  assert.deepEqual(focused, [], "nothing is focused while the viewer is open");
  page.all("dialog")[0].props.onClose();
  assert.equal(page.all("dialog").length, 0);
  assert.deepEqual(focused, ["après"]);
  page.find("button", "Avant : Page d’accueil avant").props.onClick({ currentTarget: opener("avant") });
  page.find("button", "Image suivante").props.onClick();
  page.all("dialog")[0].props.onClose();
  assert.equal(page.all("dialog").length, 0);
  assert.deepEqual(focused, ["après", "avant"], "the opener is the clicked thumbnail even after the arrows moved on");
  page.all("figure");
  assert.deepEqual(focused, ["après", "avant"], "re-rendering alone does not steal the focus");
});

test("missing, empty and invalid captures keep the same prose and navigation", () => {
  const expected = '<article class="gb-message gb-delegation-followup" data-message-id="m1" data-card-kind="delegation" data-delegation-status="done"><header class="gb-meta"><strong>Sous-tâche · Bot Test</strong></header><p class="mt-2 text-sm whitespace-pre-wrap break-words">Refaire la page</p><p class="my-2 text-sm" role="status" aria-live="polite">Terminée</p><p class="gb-delegation-result" data-find-body="true">Page modifiée.</p><button type="button" class="gb-retry">Ouvrir le fil de tâche</button></article>';
  setEdge(undefined);
  assert.equal(html(), expected);
  setEdge([]);
  assert.equal(html(), expected);
  setEdge([{ id: "x", label: "autre", caption: "?", url: "/x" }, { label: "avant" }, null]);
  assert.equal(html(), expected, "entries the engine would never send are ignored");
  setEdge([BEFORE, AFTER], "running");
  assert.doesNotMatch(html(), /gb-captures/, "only a finished task shows captures");
});

test("the legend is plain text, never HTML", () => {
  setEdge([{ ...BEFORE, caption: '<img src=x onerror="alert(1)"><b>gras</b>' }, AFTER]);
  const out = html();
  assert.doesNotMatch(out, /<b>gras/);
  assert.doesNotMatch(out, /<img src="x"/);
  assert.match(out, /&lt;img src=x onerror=&quot;alert\(1\)&quot;&gt;&lt;b&gt;gras&lt;\/b&gt;/);
});

test("a thumbnail whose image fails to load shows a placeholder and leaves the card intact", () => {
  setEdge([BEFORE, AFTER]);
  const page = mount();
  page.all("img").find((img) => img.props.alt === "Avant : Page d’accueil avant").props.onError();
  assert.equal(page.all("img").filter((img) => img.props.alt === "Avant : Page d’accueil avant").length, 0);
  assert.ok(page.all("span").some((node) => page.text(node) === "Image non disponible"));
  assert.ok(page.find("button", "Avant : Page d’accueil avant"), "the button stays and still opens the viewer");
  assert.ok(page.find("button", "Après : Page d’accueil après"));
  assert.ok(page.all("p").some((node) => node.props.className === "gb-delegation-result" && page.text(node) === "Page modifiée."));
  assert.equal(page.all("details").length, 0);
  assert.ok(page.all("button").some((node) => page.text(node) === "Ouvrir le fil de tâche"));
});

test("the row uses Galet tokens only, no hard-coded colour", () => {
  const css = readFileSync(new URL("./chat.css", import.meta.url), "utf8");
  const rules = css.split("\n").filter((line) => /^\.gb-capture/.test(line));
  assert.ok(rules.length >= 5);
  for (const rule of rules) assert.doesNotMatch(rule, /#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i, rule);
  assert.match(rules.join("\n"), /var\(--subtle\)/);
});
