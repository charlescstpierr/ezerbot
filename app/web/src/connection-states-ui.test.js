import assert from "node:assert/strict";
import { test, after } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
import react from "@vitejs/plugin-react";

const listeners = { addEventListener() {}, removeEventListener() {} };
globalThis.window = { ...listeners, matchMedia: () => ({ matches: false, ...listeners }) };
globalThis.document = {
  documentElement: { dataset: { theme: "dark", accent: "citron" } },
  createElement() { return { getContext: () => ({ fillStyle: "#111111" }) }; },
  ...listeners,
};
globalThis.window.document = globalThis.document;
globalThis.getComputedStyle = () => ({ getPropertyValue: () => "#111111" });
globalThis.MutationObserver = class { observe() {} disconnect() {} };
globalThis.ResizeObserver = class { observe() {} disconnect() {} };

// The sign-in card lives in Thread.jsx. The rest of that module pulls Mermaid,
// Shiki and ProseMirror, none of which paint this card.
function threadSignInOnly() {
  return {
    name: "thread-sign-in-only",
    enforce: "pre",
    transform(code, id) {
      if (!id.endsWith("/src/Thread.jsx")) return null;
      return code
        .replace('import { Streamdown } from "streamdown";\n', "function Streamdown() { return null; }\n")
        .replace('import { createMathPlugin } from "@streamdown/math";\n', "function createMathPlugin() { return {}; }\n")
        .replace('import { code } from "./code.js";\n', "const code = {};\n")
        .replace('import { splitMentionText } from "./mentions.js";\n', "function splitMentionText() { return []; }\n")
        .replace(
          /const cards = new Map\(Object\.values\(import\.meta\.glob\("\.\/cards\/\*\.jsx", \{ eager: true \}\)\)\s*\.filter\(\(module\) => module\.kind && module\.Card\)\s*\.map\(\(module\) => \[module\.kind, module\.Card\]\)\);/,
          "const cards = new Map();",
        );
    },
  };
}

const vite = await createTestServer({ plugins: [threadSignInOnly(), react()] });
after(() => vite.close());
const { Thread } = await vite.ssrLoadModule("/src/Thread.jsx");


function thread(message, props = {}) {
  return renderToStaticMarkup(React.createElement(Thread, {
    messages: [message],
    engineConnected: true,
    usageUrl: "https://chatgpt.com/codex/settings/usage",
    ...props,
  }));
}

const labels = (html) => [...html.matchAll(/<button\b[^>]*>([\s\S]*?)<\/button>/g)].map((match) => match[1].replace(/<[^>]+>/g, "").trim());
const failed = (error, extra = {}) => ({ id: "m", role: "assistant", state: "failed", error_kind: "other", error, ...extra });

test("a failed group turn never says Réessayez and offers no Réessayer button", () => {
  const html = thread(failed("La réponse a dépassé le délai maximal. Réessayez.", { payload: { turn_id: "t" } }), { isGroup: true });
  assert.match(html, /La réponse a dépassé le délai maximal\. Envoyez un nouveau message\./);
  assert.doesNotMatch(html, /Réessayez/);
  assert.ok(!labels(html).includes("Réessayer"));
});

test("a failed direct turn keeps its Réessayer button next to Réessayez", () => {
  const html = thread(failed("La réponse a dépassé le délai maximal. Réessayez."));
  assert.match(html, /Réessayez\./);
  assert.ok(labels(html).includes("Réessayer"));
});

test("a connected account card is a status without any alert icon", () => {
  const html = thread({ id: "ready", role: "assistant", state: "failed", error_kind: "signin", error: "Connectez votre compte ChatGPT" }, { providerConnected: true });
  assert.match(html, /Compte ChatGPT connecté/);
  assert.match(html, /role="status"/);
  assert.doesNotMatch(html, /role="alert"/);
  assert.doesNotMatch(html, /gb-problem(?![-\w])/);
  assert.doesNotMatch(html, /lucide-triangle-alert/);
  assert.match(html, /lucide-check/);
});

test("a ping without an event reads as a slow connection, not a session renewal", () => {
  const raw = "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez.";
  for (const props of [{}, { isGroup: true }]) {
    const html = thread(failed(raw), props);
    assert.match(html, /Connexion lente… on attend la réponse\./);
    assert.doesNotMatch(html, /renouvellement|aucun événement/);
  }
});

test("a reply that stays silent for 60 s asks whether to retry only when a retry button exists", () => {
  const raw = "La réponse d’OpenAI est bloquée : aucun nouvel événement reçu. Réessayez.";
  const direct = thread(failed(raw));
  assert.match(direct, /La réponse n’arrive pas\. Réessayer \?/);
  assert.ok(labels(direct).includes("Réessayer"));
  const group = thread(failed(raw), { isGroup: true });
  assert.match(group, /La réponse n’arrive pas\. Envoyez un nouveau message\./);
  assert.ok(!labels(group).includes("Réessayer"));
});

test("an expired ChatGPT session card shows the validated text, not the generic sign-in copy", () => {
  const expired = { id: "expired", role: "assistant", state: "failed", error_kind: "signin_expired", error: "Votre session ChatGPT a expiré. Reconnectez-vous à votre compte ChatGPT." };
  for (const props of [{}, { isGroup: true }, { isTask: true }]) {
    const html = thread(expired, props);
    assert.match(html, /<p>Ta session ChatGPT a expiré\. Reconnecte-toi pour continuer\.<\/p>/);
    assert.doesNotMatch(html, /<p>[^<]*Connectez votre compte/);
    assert.ok(labels(html).includes("Se reconnecter"));
  }
  const missing = thread({ ...expired, error_kind: "signin" });
  assert.doesNotMatch(missing, /Ta session ChatGPT a expiré/);
});
