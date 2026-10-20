import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const source = shell.slice(shell.indexOf("  function notify("), shell.indexOf("  async function copyText("));

function fixture({ dialogs = [], popover = true } = {}) {
  const timers = new Map(), positions = {}, calls = [];
  let nextTimer = 0;
  const toast = { hidden: true, isConnected: true, style: { setProperty(key, value) { positions[key] = value; } } };
  if (popover) Object.assign(toast, { showPopover() { calls.push("show"); }, hidePopover() { calls.push("hide"); } });
  const host = (localName, box, footer) => ({ localName, getBoundingClientRect: () => box,
    querySelector: () => footer && { getBoundingClientRect: () => footer },
    append(child) { assert.equal(child, toast); toast.parent = this; } });
  const composer = host("div", { left: 278, width: 854, top: 595, bottom: 780 });
  const sheets = dialogs.map((footer) => host("dialog", { left: 335, width: 470, top: 20, bottom: 760 }, footer));
  const notify = runInNewContext(`(${source})`, {
    ui: { toast }, toastTimer: null, innerHeight: 780,
    document: { querySelectorAll: () => sheets, querySelector: () => composer },
    clearTimeout: (id) => timers.delete(id),
    setTimeout(callback, delay) { assert.equal(delay, 4000); timers.set(++nextTimer, callback); return nextTimer; },
  });
  return { notify, toast, calls, positions, timers, composer, sheets };
}

test("conversation and compose confirmations keep their position above the composer", () => {
  const f = fixture();
  f.notify("Texte placé dans le brouillon. Rien n’a été envoyé.");
  assert.equal(f.toast.hidden, false);
  assert.equal(f.positions["--toast-left"], "705px");
  assert.equal(f.positions["--toast-bottom"], "193px");
  assert.deepEqual(f.calls, ["hide", "show"]);
  [...f.timers.values()][0]();
  assert.equal(f.toast.hidden, true);
  assert.deepEqual(f.calls, ["hide", "show", "hide"]);
});

test("copy feedback enters the top layer over settings, even when its footer is offscreen", () => {
  const f = fixture({ dialogs: [{ top: 1400 }] });
  f.notify("Lien copié. Il n’accorde aucun droit d’accès.");
  assert.equal(f.positions["--toast-left"], "570px");
  assert.equal(f.positions["--toast-bottom"], "36px");
  assert.equal(f.toast.parent, f.sheets[0], "a status outside the active modal would remain inert");
  assert.deepEqual(f.calls, ["hide", "show"]);
});

test("the foremost bot sheet anchors feedback above its visible action buttons", () => {
  const f = fixture({ dialogs: [{ top: 1400 }, { top: 690 }] });
  f.notify("Lien copié. Il n’accorde aucun droit d’accès.");
  assert.equal(f.positions["--toast-bottom"], "98px");
  assert.equal(f.toast.hidden, false);
  f.notify("Le lien n’a pas pu être copié.");
  assert.equal(f.timers.size, 1, "replace the old expiry rather than hiding the new feedback early");
  assert.equal(f.toast.textContent, "Le lien n’a pas pu être copié.");
  assert.deepEqual(f.calls, ["hide", "show", "hide", "show"]);
});

test("document save feedback anchors above the destination dialog action footer", () => {
  const component = readFileSync(new URL("./generated-document.jsx", import.meta.url), "utf8");
  assert.match(component, /className="gb-file-actions dialog-footer"/);
  const f = fixture({ dialogs: [{ top: 490 }] });
  f.notify("Destination vérifiée.");
  assert.equal(f.positions["--toast-bottom"], "298px");
  assert.equal(f.toast.parent, f.sheets[0]);
});

test("older WebKit without popover support hosts the status inside the active modal", () => {
  const f = fixture({ dialogs: [undefined], popover: false });
  f.notify("Lien copié. Il n’accorde aucun droit d’accès.");
  assert.equal(f.toast.parent, f.sheets[0]);
  assert.equal(f.toast.hidden, false);
  [...f.timers.values()][0]();
  assert.equal(f.toast.hidden, true);
});

test("expiry safely restores the toast after its temporary modal host has been removed", () => {
  const f = fixture({ dialogs: [undefined] });
  f.notify("Bot enregistré.");
  f.toast.isConnected = false;
  [...f.timers.values()][0]();
  assert.equal(f.toast.hidden, true);
  assert.equal(f.toast.parent, f.composer);
  assert.deepEqual(f.calls, ["hide", "show"], "do not call the popover API on a detached editor");
});

test("the status uses a manual popover without stealing focus or clicks", () => {
  const html = readFileSync(new URL("../../ui/index.html", import.meta.url), "utf8");
  const css = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
  assert.match(html, /id="toast"[^>]*role="status"[^>]*popover="manual"[^>]*hidden/);
  assert.match(css, /\.toast \{ pointer-events: none; position: fixed;/);
  assert.doesNotMatch(source, /\.focus\(/);
});
