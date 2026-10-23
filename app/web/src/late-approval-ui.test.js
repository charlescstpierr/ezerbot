import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/approval.jsx");

// A card left waiting by a turn that went on without it (#215), then decided or expired.
function render(status, { detached = true, decision = "allow_once", wake } = {}) {
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: {
      id: "late", kind: "approval", payload: {
        entry_id: "late", status: { status }, route: { requesting_bot_id: "grokbot" }, detached, decision,
        kind: { kind: "approval", tool: "test_protected_write", expires_at: Date.now() + (status === "expired" ? -60000 : 60000) },
        arguments: { path: "x" }, receipt: wake ? { resolution_id: "r", wake } : null,
      },
    } }));
  } finally { delete globalThis.window; }
}
const failedWake = { status: "failed", reason: "run already ended" };

test("a card approved after its turn ended says « Autorisé une fois » and that the bot resumes in the thread", () => {
  const html = render("resolved", { wake: failedWake });
  assert.match(html, /Autorisé une fois/);
  assert.match(html, /Le bot reprend dans la conversation\./);
  // The ended turn is not what resumes: no failure line for a wake that was never meant to land.
  assert.doesNotMatch(html, /La reprise a échoué/);
  assert.doesNotMatch(html, /Cette tâche du bot est déjà terminée/);
  assert.doesNotMatch(html, /<button/);
});

test("a refusal after the turn ended tells that the bot is told, without a failed resume", () => {
  const html = render("denied", { decision: "deny", wake: failedWake });
  assert.match(html, /Refusée/);
  assert.match(html, /Refus transmis au bot\./);
  assert.doesNotMatch(html, /La reprise a échoué/);
});

test("an expired card is grey, has no button and says that no action was made", () => {
  for (const detached of [true, false]) {
    const html = render("expired", { detached, wake: failedWake });
    assert.match(html, /gb-approval-expired/);
    assert.match(html, /Cette demande a expiré\. Aucune action n’a été faite\./);
    assert.match(html, /Expirée/);
    assert.doesNotMatch(html, /<button/);
    assert.doesNotMatch(html, /Autoriser une fois/);
    if (detached) assert.doesNotMatch(html, /La reprise a échoué/);
  }
});

test("a pending or resolved card is not greyed and keeps its ordinary resume line when it blocked its turn", () => {
  assert.doesNotMatch(render("pending", { detached: false }), /gb-approval-expired/);
  const html = render("resolved", { detached: false, wake: { status: "pending" } });
  assert.doesNotMatch(html, /gb-approval-expired/);
  assert.match(html, /Le bot reprend…/);
  assert.doesNotMatch(html, /Le bot reprend dans la conversation/);
});
