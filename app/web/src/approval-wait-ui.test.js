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
const { approvalWaitLine } = await vite.ssrLoadModule("/src/approval-wait.js");

const MINUTE = 60_000, HOUR = 60 * MINUTE;
const NOW = Date.UTC(2026, 9, 23, 12, 0, 0);

// The card is really rendered, with a frozen clock: the line is read from its markup.
function render({ status = "pending", age = 2 * HOUR, left = 22 * HOUR, created = NOW - age, receipt = null, tool = "NE_JAMAIS_AFFICHER_CET_OUTIL" } = {}) {
  const realNow = Date.now;
  Date.now = () => NOW;
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: {
      id: "wait", kind: "approval", created_at: created, payload: {
        entry_id: "wait", status: { status }, route: { requesting_bot_id: "grokbot" }, detached: true, receipt,
        kind: { kind: "approval", tool, expires_at: NOW + left }, arguments: { secret: "NE_JAMAIS_AFFICHER_CET_ARGUMENT" },
      },
    } }));
  } finally { delete globalThis.window; Date.now = realNow; }
}

test("a waiting card shows its age and expiry in a grey line above it, instead of the bare waiting mention", () => {
  const html = render();
  assert.match(html, /En attente de vous depuis 2 h · expire dans 22 h/);
  assert.ok(html.indexOf("En attente de vous depuis") < html.indexOf("<article"), "la ligne précède la carte");
  assert.match(html, /<p class="gb-approval-wait[^"]*text-muted-foreground"/);
  // The card header no longer repeats it.
  assert.doesNotMatch(html.slice(html.indexOf("<article")), /En attente de vous/);
  assert.match(html, /Demande d’Ezerbot/);
  assert.match(html, /Autoriser une fois/);
});

test("the line follows the clock: minutes, hours and days, never negative", () => {
  assert.equal(approvalWaitLine(NOW - 5 * MINUTE, NOW + 55 * MINUTE, NOW), "En attente de vous depuis 5 min · expire dans 55 min");
  assert.equal(approvalWaitLine(NOW - 30_000, NOW + HOUR, NOW), "En attente de vous depuis moins d’une minute · expire dans 1 h");
  assert.equal(approvalWaitLine(NOW - 90 * MINUTE, NOW + 22 * HOUR + 30 * MINUTE, NOW), "En attente de vous depuis 1 h 30 · expire dans 22 h 30");
  assert.equal(approvalWaitLine(NOW - 50 * HOUR, NOW + 118 * HOUR, NOW), "En attente de vous depuis 2 j 2 h · expire dans 4 j 22 h");
  assert.equal(approvalWaitLine(NOW + HOUR, NOW + 2 * HOUR, NOW), "En attente de vous depuis moins d’une minute · expire dans 2 h");
});

test("the line says nothing about the tool, the arguments or any secret", () => {
  const html = render();
  const line = html.match(/<p class="gb-approval-wait[^>]*>([^<]*)<\/p>/)[1];
  assert.equal(line, "En attente de vous depuis 2 h · expire dans 22 h");
});

test("a card that cannot tell its age or that is decided, expired or past its deadline keeps its usual header", () => {
  for (const html of [render({ created: 0 }), render({ left: -MINUTE }), render({ receipt: { resolution_id: "r", wake: { status: "pending" } } })]) {
    assert.doesNotMatch(html, /gb-approval-wait/);
    assert.doesNotMatch(html, /expire dans/);
  }
  assert.match(render({ created: 0 }), /Demande d’Ezerbot · En attente de vous/);
  for (const status of ["resolved", "denied", "expired", "cancelled"]) {
    const html = render({ status });
    assert.doesNotMatch(html, /gb-approval-wait/, status);
    assert.doesNotMatch(html, /depuis 2 h/, status);
  }
});
