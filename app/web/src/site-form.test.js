import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

test("la carte de site affiche l’origine confirmée et la promesse exacte", async () => {
  const vite = await createServer({ configFile: false, cacheDir: "app/web/node_modules/.vite", server: { middlewareMode: true }, esbuild: { jsx: "automatic" } });
  try {
    const { Card } = await vite.ssrLoadModule("/app/web/src/cards/form.jsx");
    globalThis.window = {};
    const html = renderToStaticMarkup(React.createElement(Card, { message: { kind: "form", payload: {
      entry_id: "site", route: {}, status: { status: "pending" }, kind: { kind: "site_form", origin: "https://example.org", fields: [] },
    } } }));
    assert.match(html, /Jamais montré à votre bot\. Enregistré de façon sécurisée dans le coffre local\./);
    assert.match(html, /Site confirmé : https:\/\/example.org/);
    assert.match(html, /<form/);
  } finally { await vite.close(); }
});

test("un refus du serveur garde les valeurs et affiche son message, un succès les efface", async () => {
  const error = "Formulaire trop volumineux : limite de 4096 octets, origine et noms des champs compris.";
  let fail = true;
  const panels = { state: { forms: {} }, register(value) { this.panel = value; },
    async api() {
      if (fail) throw new Error(error);
      return { accepted: true, receipt: { wake: { status: "pending" } } };
    }, async refresh() {}, notify() {} };
  const window = { GrokbotPanels: panels, dispatchEvent() {}, setTimeout, clearTimeout };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), {
    document: { createElement: () => ({}), getElementById: () => null }, window, Event: class {},
  });
  const message = { kind: "form", payload: { entry_id: "site", route: { target_conversation_id: "conv" },
    kind: { kind: "site_form", origin: "https://example.org" }, status: { status: "pending" } } };
  const source = readFileSync(new URL("./cards/form.jsx", import.meta.url), "utf8");
  const handler = source.slice(source.indexOf("  async function submit(event) {"), source.indexOf("  return (", source.indexOf("  async function submit(event) {")));
  const makeSubmit = new Function("fields", "site", "disabled", "FormData", "act", `${handler}; return submit;`);
  const form = { value: "mot de passe privé", reportValidity: () => true, reset() { this.value = ""; } };
  const submit = makeSubmit([{ name: "password", required: true, field_type: { kind: "text" } }], true, false,
    class { get() { return form.value; } },
    (action, detail) => panels.panel.onCardAction(message, action, detail, { conversation: { id: "conv" }, state: panels.state }));
  await submit({ preventDefault() {}, currentTarget: form });
  assert.equal(form.value, "mot de passe privé");
  assert.equal(window.GrokbotForms.view("site").error, error);
  const vite = await createServer({ configFile: false, cacheDir: "app/web/node_modules/.vite", server: { middlewareMode: true }, esbuild: { jsx: "automatic" } });
  try {
    const { Card } = await vite.ssrLoadModule("/app/web/src/cards/form.jsx");
    globalThis.window = window;
    const html = renderToStaticMarkup(React.createElement(Card, { message }));
    assert.ok(html.includes(error));
    assert.match(html, /role="alert"/);
  } finally { await vite.close(); }
  fail = false;
  await submit({ preventDefault() {}, currentTarget: form });
  assert.equal(form.value, "");
});
