import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  configFile: false,
  plugins: [react()],
  server: { middlewareMode: true },
  logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { MessageMenu } = await vite.ssrLoadModule("/src/message-menu.jsx");

function menu(message) {
  return renderToStaticMarkup(h(MessageMenu, {
    message,
    open: true,
    onCopy: () => {},
    onReact: () => {},
    onResend: () => {},
    onDelete: () => {},
  }));
}

test("le menu d’un message livré copie et réagit, sans Renvoyer, Supprimer ni édition", () => {
  const html = menu({ role: "user", state: "echoed", body: "Déjà livré", payload: { reaction: "👍" } });
  assert.match(html, /Réaction/);
  assert.match(html, /👍/);
  assert.match(html, /Copier/);
  assert.doesNotMatch(html, /Renvoyer/);
  assert.doesNotMatch(html, /Supprimer/);
  assert.doesNotMatch(html, /Modifier|Éditer/);
  assert.match(html, /aria-pressed="true"/);
});

test("le menu d’un envoi non livré propose Renvoyer et Supprimer sans confirmation", () => {
  for (const [state, label] of [["unknown", "Pas encore livré"], ["queued", "Pas encore livré"], ["refused", "Échec de l’envoi"]]) {
    const html = menu({ role: "user", state, body: "Phrase", payload: {} });
    assert.match(html, new RegExp(label));
    assert.match(html, /Renvoyer/);
    assert.match(html, /Supprimer/);
    assert.match(html, /Copier/);
    assert.doesNotMatch(html, /Confirmer|Voulez-vous/);
    assert.doesNotMatch(html, /Modifier|Éditer/);
  }
});
