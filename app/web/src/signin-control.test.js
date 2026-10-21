import assert from "node:assert/strict";
import { test, after } from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
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
        .replace('import { createMermaidPlugin } from "@streamdown/mermaid";\n', "function createMermaidPlugin() { return {}; }\n")
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

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  configFile: false,
  plugins: [threadSignInOnly(), react()],
  server: { middlewareMode: true },
  logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Thread } = await vite.ssrLoadModule("/src/Thread.jsx");

const forbidden = /via Codex|Sign in with ChatGPT/;

function signInControls(html) {
  return [...html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)]
    .map((match) => ({ attrs: match[1], text: match[2].replace(/<[^>]+>/g, "").trim() }))
    .filter((button) => button.attrs.includes("chatgpt-button") || forbidden.test(button.text));
}

function thread(message, providerConnected) {
  return renderToStaticMarkup(React.createElement(Thread, {
    messages: [message],
    engineConnected: true,
    providerConnected,
    usageUrl: "https://chatgpt.com/codex/settings/usage",
  }));
}

test("the thread shows exactly one ChatGPT sign-in control and no second connection option", () => {
  const missing = thread({ id: "missing", role: "assistant", state: "failed", error_kind: "signin", error: "Connectez votre compte ChatGPT" }, false);
  const missingControls = signInControls(missing);
  assert.equal(missingControls.length, 1);
  assert.equal(missingControls[0].text, "Se connecter");
  assert.doesNotMatch(missing, forbidden);

  const expired = thread({ id: "expired", role: "assistant", state: "failed", error_kind: "signin_expired", error: "Session expirée" }, false);
  const expiredControls = signInControls(expired);
  assert.equal(expiredControls.length, 1);
  assert.equal(expiredControls[0].text, "Se reconnecter");
  assert.doesNotMatch(expired, forbidden);

  const connected = thread({ id: "ready", role: "assistant", state: "failed", error_kind: "signin", error: "Connectez votre compte ChatGPT" }, true);
  assert.equal(signInControls(connected).length, 0);
  assert.doesNotMatch(connected, forbidden);
});
