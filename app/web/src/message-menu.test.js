import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import { createElement as h, Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { messageMenuOpenId } from "./message-actions.js";
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

test("un seul menu de message est ouvert à la fois", () => {
  const undelivered = (id) => ({ id, role: "user", state: "unknown", body: id, payload: {} });
  let openId = messageMenuOpenId(null, "un", true);
  openId = messageMenuOpenId(openId, "deux", true);
  const html = renderToStaticMarkup(h(Fragment, null,
    h(MessageMenu, { message: undelivered("un"), expanded: openId === "un", onOpenChange: () => {} }),
    h(MessageMenu, { message: undelivered("deux"), expanded: openId === "deux", onOpenChange: () => {} }),
  ));
  assert.equal((html.match(/role="menu"/g) || []).length, 1);
  assert.match(html, /data-message-id="deux"[^>]*data-open="true"/);
  assert.match(html, /data-message-id="un"[^>]*data-open="false"/);
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
