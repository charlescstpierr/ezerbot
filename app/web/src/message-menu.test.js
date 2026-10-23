import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import { createElement as h, Fragment } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  MESSAGE_MENU_GAP,
  MESSAGE_MENU_MARGIN,
  REACTIONS,
  bindMessageMenuDismiss,
  layoutMessageMenu,
  messageMenuActionBand,
  messageMenuOpenId,
  placeMessageMenu,
  reactionRowBox,
  readMessageMenuBounds,
} from "./message-actions.js";
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

test("« Répondre en fil » s’offre sur un message livré et sur une réponse terminée, pas sur un envoi perdu", () => {
  const withThread = (message) => renderToStaticMarkup(h(MessageMenu, { message, open: true, onCopy: () => {}, onReact: () => {}, onThread: () => {} }));
  assert.match(withThread({ id: "u", role: "user", state: "echoed", body: "Une phrase", payload: {} }), /Répondre en fil/);
  assert.match(withThread({ id: "a", role: "assistant", state: "echoed", body: "Une réponse", payload: {} }), /Répondre en fil/);
  for (const message of [
    { id: "q", role: "user", state: "queued", body: "Pas parti", payload: {} },
    { id: "r", role: "user", state: "refused", body: "Refusé", payload: {} },
    { id: "s", role: "assistant", state: "streaming", body: "En cours", payload: {} },
    { id: "f", role: "assistant", state: "failed", body: "Raté", payload: {} },
    { id: "e", role: "assistant", state: "echoed", body: "  ", payload: {} },
    { id: "c", role: "assistant", state: "echoed", kind: "capability", body: "Carte", payload: {} },
  ]) assert.doesNotMatch(withThread(message), /Répondre en fil/, message.id);
});

test("sans action de fil (groupe, tâche), le menu ne propose pas « Répondre en fil »", () => {
  const html = renderToStaticMarkup(h(MessageMenu, { message: { id: "u", role: "user", state: "echoed", body: "Une phrase", payload: {} }, open: true, onCopy: () => {}, onReact: () => {} }));
  assert.match(html, /Copier/);
  assert.doesNotMatch(html, /Répondre en fil/);
});

const VIEW = { top: 0, left: 0, right: 1140, bottom: 780 };
// Emoji + Copier, Renvoyer et Supprimer. Assez haut pour passer sous la saisie.
const PANEL = { width: 196, height: 140 };
const menuSource = readFileSync(new URL("./message-menu.jsx", import.meta.url), "utf8");
const menuCss = readFileSync(new URL("./chat.css", import.meta.url), "utf8");

function visibleReactions(left, rightEdge) {
  return REACTIONS.filter((_, index) => {
    const box = reactionRowBox({ left, top: 0 }, index);
    return box.left >= VIEW.left && box.right <= rightEdge;
  }).length;
}

test("sur le dernier message, le menu s’ouvre vers le haut pour laisser Copier, Renvoyer et Supprimer au-dessus de la saisie", () => {
  const composer = { top: 680, height: 100 };
  const anchor = { top: 640, bottom: 666, left: 420, right: 446 };
  const openedDown = anchor.bottom + MESSAGE_MENU_GAP + PANEL.height;
  assert.ok(openedDown > composer.top, "sans retournement, Supprimer passe sous la saisie");
  const place = placeMessageMenu({ anchor, panel: PANEL, viewport: VIEW, obstacle: composer });
  assert.equal(place.placement, "up");
  assert.ok(place.bottom <= composer.top - MESSAGE_MENU_MARGIN);
  assert.ok(place.top >= VIEW.top + MESSAGE_MENU_MARGIN);
  const band = messageMenuActionBand(place);
  assert.ok(band.top < band.bottom);
  assert.ok(band.bottom <= composer.top - MESSAGE_MENU_MARGIN);
  const panel = { offsetWidth: PANEL.width, offsetHeight: PANEL.height, style: {}, dataset: {} };
  const laid = layoutMessageMenu(panel, anchor, readMessageMenuBounds({
    innerWidth: VIEW.right,
    innerHeight: VIEW.bottom,
    document: { querySelector: (selector) => (selector === ".composer-dock" ? { getBoundingClientRect: () => composer } : null) },
  }));
  assert.equal(panel.style.position, "fixed");
  assert.equal(panel.dataset.placement, "up");
  assert.equal(panel.dataset.placed, "true");
  assert.equal(laid.top, place.top);
  assert.match(menuSource, /layoutMessageMenu\(/);
  assert.match(menuSource, /readMessageMenuBounds\(/);
  const rule = menuCss.slice(menuCss.indexOf(".gb-message-menu-panel {"), menuCss.indexOf(".gb-reaction-row"));
  assert.match(rule, /position:\s*fixed/);
  assert.doesNotMatch(rule, /top:\s*calc\(100%/);
  const roomy = placeMessageMenu({
    anchor: { top: 120, bottom: 146, left: 420, right: 446 },
    panel: PANEL,
    viewport: VIEW,
    obstacle: composer,
  });
  assert.equal(roomy.placement, "down");
  assert.equal(roomy.top, 150);
});

test("les six réactions restent dans la fenêtre quand le menu toucherait le bord droit", () => {
  const anchor = { top: 180, bottom: 206, left: 1040, right: 1066 };
  assert.equal(visibleReactions(anchor.left, VIEW.right), 3);
  const place = placeMessageMenu({ anchor, panel: PANEL, viewport: VIEW, obstacle: { top: 680 } });
  assert.equal(visibleReactions(place.left, VIEW.right - MESSAGE_MENU_MARGIN), REACTIONS.length);
  assert.ok(place.left >= VIEW.left + MESSAGE_MENU_MARGIN);
  assert.ok(place.right <= VIEW.right - MESSAGE_MENU_MARGIN);
  for (const [index] of REACTIONS.entries()) {
    const box = reactionRowBox(place, index);
    assert.ok(box.left >= VIEW.left + MESSAGE_MENU_MARGIN, index);
    assert.ok(box.right <= VIEW.right - MESSAGE_MENU_MARGIN, index);
  }
  const panel = { offsetWidth: PANEL.width, offsetHeight: PANEL.height, style: {}, dataset: {} };
  layoutMessageMenu(panel, anchor, { viewport: VIEW, obstacle: { top: 680 } });
  assert.equal(panel.style.position, "fixed");
  assert.equal(Number(panel.style.left.replace("px", "")), place.left);
  const squeezed = placeMessageMenu({
    anchor: { top: 180, bottom: 206, left: 0, right: 26 },
    panel: PANEL,
    viewport: VIEW,
  });
  assert.ok(squeezed.left >= MESSAGE_MENU_MARGIN);
  assert.ok(squeezed.right <= VIEW.right - MESSAGE_MENU_MARGIN);
  const rule = menuCss.slice(menuCss.indexOf(".gb-message-menu-panel {"), menuCss.indexOf(".gb-reaction-row"));
  assert.match(rule, /min-width:\s*196px/);
  assert.match(rule, /padding:\s*6px/);
  assert.match(menuCss, /\.gb-reaction-row \{[^}]*gap:\s*2px/);
  assert.match(menuCss, /\.gb-reaction-row button \{[^}]*width:\s*28px/);
});

test("Échap ferme le menu, et le défilement le ferme au lieu de le laisser suivre", () => {
  const listeners = [];
  const target = {
    addEventListener(type, fn, capture) { listeners.push({ type, fn, capture }); },
    removeEventListener(type, fn, capture) {
      const index = listeners.findIndex((item) => item.type === type && item.fn === fn && item.capture === capture);
      if (index >= 0) listeners.splice(index, 1);
    },
    emit(type, event) { for (const item of [...listeners]) if (item.type === type) item.fn(event); },
  };
  const anchor = { top: 200, bottom: 226, left: 480, right: 506 };
  const panel = { offsetWidth: PANEL.width, offsetHeight: PANEL.height, style: {}, dataset: {} };
  const place = layoutMessageMenu(panel, anchor, { viewport: VIEW, obstacle: { top: 680 } });
  assert.equal(panel.style.position, "fixed");
  assert.equal(place.placement, "down");
  let closed = 0;
  let prevented = 0;
  const unbind = bindMessageMenuDismiss(target, () => { closed += 1; });
  const scroll = listeners.find((item) => item.type === "scroll");
  assert.equal(scroll.capture, true);
  target.emit("keydown", { key: "Escape", preventDefault() { prevented += 1; } });
  assert.equal(closed, 1);
  assert.equal(prevented, 1);
  target.emit("keydown", { key: "Enter", preventDefault() { prevented += 1; } });
  target.emit("keydown", { key: "Escape", isComposing: true, preventDefault() { prevented += 1; } });
  assert.equal(closed, 1);
  target.emit("scroll", { target: { className: "chat-scroll" } });
  assert.equal(closed, 2);
  const menuNode = { closest: (selector) => (selector === ".gb-message-menu-panel" ? menuNode : null) };
  target.emit("scroll", { target: menuNode });
  assert.equal(closed, 2);
  unbind();
  target.emit("keydown", { key: "Escape", preventDefault() {} });
  target.emit("scroll", { target: { className: "chat-scroll" } });
  assert.equal(closed, 2);
  assert.equal(listeners.length, 0);
  assert.match(menuSource, /bindMessageMenuDismiss\(/);
  assert.match(menuSource, /createPortal\(/);
});
