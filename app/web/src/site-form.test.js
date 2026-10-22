import assert from "node:assert/strict";
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
    assert.match(html, /Jamais montré à votre bot\. Rempli directement sur https:\/\/example.org\./);
    assert.match(html, /Site confirmé : https:\/\/example.org/);
    assert.match(html, /<form/);
  } finally { await vite.close(); }
});
