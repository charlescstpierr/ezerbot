import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";
import React from "react";
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
const { GeneratedImage } = await vite.ssrLoadModule("/src/generated-image.jsx");
const { Card } = await vite.ssrLoadModule("/src/cards/tool.jsx");

const call = { call_id: "dessin", name: "generate_image", arguments: { description: "un phare au crépuscule" } };
const html = (element) => renderToStaticMarkup(element);

test("pending image card shows the indicator and no image", () => {
  const markup = html(React.createElement(GeneratedImage, { message: { id: "m1", payload: { call } } }));
  assert.match(markup, /Génération d’une image…/);
  assert.doesNotMatch(markup, /<img/);
  assert.doesNotMatch(markup, /approbation/);
});

test("finished image card replaces the indicator", () => {
  const markup = html(React.createElement(Card, {
    message: {
      id: "m2",
      kind: "tool",
      payload: { call, outcome: { status: "done", output: { attachment_id: "attachment_abc" } }, run_status: "running" },
      attachments: [{ id: "attachment_abc", mime_type: "image/png", url: "/api/attachments/attachment_abc" }],
    },
  }));
  assert.match(markup, /src="\/api\/attachments\/attachment_abc"/);
  assert.match(markup, /alt="Image générée"/);
  assert.doesNotMatch(markup, /Génération/);
  assert.doesNotMatch(markup, /approbation/);
});

test("safety failure is a french sentence", () => {
  const markup = html(React.createElement(GeneratedImage, {
    message: {
      id: "m3",
      payload: { call, outcome: { status: "failed", reason: "La génération de cette image a été refusée pour des raisons de sécurité." } },
    },
  }));
  assert.match(markup, /sécurité/);
  assert.doesNotMatch(markup, /<img/);
  assert.doesNotMatch(markup, /approbation/);
});

test("missing session shows the ChatGPT sign-in card", () => {
  const markup = html(React.createElement(GeneratedImage, {
    message: {
      id: "m4",
      body: "Connectez votre compte ChatGPT",
      payload: { call, outcome: { status: "failed", reason: "Connectez votre compte ChatGPT" } },
    },
  }));
  assert.match(markup, /<h3>Connectez votre compte ChatGPT<\/h3>/);
  assert.match(markup, />Se connecter à ChatGPT</);
  assert.match(markup, /data-action="provider.signin"/);
  assert.match(markup, /data-provider-signin/);
  assert.doesNotMatch(markup, /Connecte ton compte OpenAI/);
  assert.doesNotMatch(markup, /clé/);
  assert.doesNotMatch(markup, /<img/);
});
