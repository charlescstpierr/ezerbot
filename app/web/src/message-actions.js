// Menu d’un message : une réaction est une bascule, et Renvoyer / Supprimer
// n’existent que tant que l’envoi n’est pas livré.
export const REACTIONS = ["👍", "❤️", "😂", "🎉", "👀", "🙏"];

const UNDELIVERED = new Set(["queued", "dispatching", "unknown", "refused", "prepared"]);
const IN_FLIGHT = new Set(["queued", "dispatching"]);

export function toggleReaction(current, emoji) {
  return current === emoji ? null : emoji;
}

// Un seul menu à la fois : l’ouvrir remplace l’autre, le refermer ne touche que le sien.
export function messageMenuOpenId(openId, messageId, open) {
  if (open) return messageId;
  return openId === messageId ? null : openId;
}

export function messageReaction(message) {
  const value = message?.payload?.reaction ?? message?.reaction ?? null;
  return typeof value === "string" && value ? value : null;
}

export function messageActions(message, { busy = false } = {}) {
  const userSend = message?.role === "user" && [undefined, null, "message", "submission"].includes(message.kind);
  const failed = userSend && UNDELIVERED.has(message.state) && !(busy && IN_FLIGHT.has(message.state));
  return {
    status: failed ? (message.state === "refused" ? "Échec de l’envoi" : "Pas encore livré") : null,
    copy: true,
    resend: failed,
    delete: failed,
    edit: false,
    confirm: false,
    reaction: messageReaction(message),
  };
}

// Le panneau est en position fixe : ces mesures reprennent chat.css
// (bouton 28, écart 2, marge interne 6, espace avec le déclencheur 4).
export const MESSAGE_MENU_GAP = 4;
export const MESSAGE_MENU_MARGIN = 8;
export const MESSAGE_MENU_PAD = 6;
export const REACTION_BUTTON_SIZE = 28;
export const REACTION_BUTTON_GAP = 2;

export function reactionRowBox(origin, index) {
  const left = origin.left + MESSAGE_MENU_PAD + index * (REACTION_BUTTON_SIZE + REACTION_BUTTON_GAP);
  const top = origin.top + MESSAGE_MENU_PAD;
  return { left, top, right: left + REACTION_BUTTON_SIZE, bottom: top + REACTION_BUTTON_SIZE };
}

// Bande des entrées sous les emojis : Copier, et Renvoyer / Supprimer s’il y a lieu.
export function messageMenuActionBand(place) {
  const top = place.top + MESSAGE_MENU_PAD + REACTION_BUTTON_SIZE + MESSAGE_MENU_GAP;
  return { top, bottom: place.bottom - MESSAGE_MENU_PAD };
}

export function messageMenuBounds(viewport = {}, obstacle) {
  const top = viewport.top ?? 0;
  const left = viewport.left ?? 0;
  const right = viewport.right ?? left + (viewport.width ?? 0);
  const viewBottom = viewport.bottom ?? top + (viewport.height ?? 0);
  const limit = Number.isFinite(obstacle?.top) ? Math.min(viewBottom, obstacle.top) : viewBottom;
  return { top, left, right, bottom: limit };
}

// La saisie (`.composer-dock`) borne le bas : le dernier message est juste au-dessus.
export function readMessageMenuBounds(view = globalThis) {
  const width = view?.innerWidth ?? 0;
  const height = view?.innerHeight ?? 0;
  const viewport = { top: 0, left: 0, right: width, bottom: height, width, height };
  const rect = view?.document?.querySelector?.(".composer-dock")?.getBoundingClientRect?.();
  const obstacle = rect && rect.height > 0 ? { top: rect.top, bottom: rect.bottom, height: rect.height } : null;
  return { viewport, obstacle };
}

// Ouvre vers le bas, sauf si le panneau passerait sous la saisie ou hors de la fenêtre :
// il se retourne alors vers le haut. Le bord droit est ramené dans la fenêtre.
export function placeMessageMenu({ anchor, panel, viewport, obstacle, gap = MESSAGE_MENU_GAP, margin = MESSAGE_MENU_MARGIN } = {}) {
  const bounds = messageMenuBounds(viewport, obstacle);
  const width = panel?.width ?? 0;
  const height = panel?.height ?? 0;
  const belowTop = anchor.bottom + gap;
  const aboveTop = anchor.top - gap - height;
  const roomBelow = bounds.bottom - margin - belowTop;
  const roomAbove = anchor.top - gap - (bounds.top + margin);
  let placement = "down";
  let top = belowTop;
  if (height > roomBelow && (height <= roomAbove || roomAbove > roomBelow)) {
    placement = "up";
    top = aboveTop;
  }
  const minTop = bounds.top + margin;
  const maxTop = Math.max(minTop, bounds.bottom - margin - height);
  top = Math.min(Math.max(top, minTop), maxTop);
  const minLeft = bounds.left + margin;
  const maxLeft = Math.max(minLeft, bounds.right - margin - width);
  const left = Math.min(Math.max(anchor.left, minLeft), maxLeft);
  return { top, left, width, height, placement, right: left + width, bottom: top + height };
}

export function layoutMessageMenu(panel, anchor, bounds = {}) {
  const width = panel.offsetWidth || panel.width || 0;
  const height = panel.offsetHeight || panel.height || 0;
  const place = placeMessageMenu({
    anchor,
    panel: { width, height },
    viewport: bounds.viewport ?? bounds,
    obstacle: bounds.obstacle,
  });
  panel.style.position = "fixed";
  panel.style.top = `${place.top}px`;
  panel.style.left = `${place.left}px`;
  panel.dataset.placement = place.placement;
  panel.dataset.placed = "true";
  return place;
}

// Échap ferme. Un défilement ferme aussi : le menu ne suit pas le fil.
export function bindMessageMenuDismiss(target, close) {
  if (!target?.addEventListener || typeof close !== "function") return () => {};
  const onKey = (event) => {
    if (event.key !== "Escape" || event.isComposing) return;
    event.preventDefault?.();
    close();
  };
  const onScroll = (event) => {
    const node = event.target;
    if (typeof node?.closest === "function" && node.closest(".gb-message-menu-panel")) return;
    close();
  };
  target.addEventListener("keydown", onKey);
  target.addEventListener("scroll", onScroll, true);
  return () => {
    target.removeEventListener("keydown", onKey);
    target.removeEventListener("scroll", onScroll, true);
  };
}
