import { resolve } from "node:path";
import { build } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { bundles } from "./src/bundles.js";

// The Rust wrapper embeds a fixed list of files and serves them under a strict CSP, so the
// chat ships as one classic script and one stylesheet next to the shell, plus one classic
// script per piece that loads only when a message needs it (src/bundles.js).
const root = import.meta.dirname;
const outDir = resolve(root, "../ui");

function iife({ entry, name, file, plugins = [], copyPublicDir = false }) {
  return build({
    root,
    configFile: false,
    plugins,
    define: { "process.env.NODE_ENV": '"production"' },
    build: {
      outDir,
      emptyOutDir: false,
      copyPublicDir,
      cssCodeSplit: false,
      lib: { entry: resolve(root, entry), formats: ["iife"], name, fileName: () => file, cssFileName: "chat" },
    },
  });
}

await iife({ entry: "src/main.jsx", name: "GrokbotChatBundle", file: "chat.js", plugins: [react(), tailwindcss()], copyPublicDir: true });
const entries = { code: "src/code-bundle.js", mermaid: "src/mermaid-bundle.js" };
for (const [piece, entry] of Object.entries(entries)) {
  await iife({ entry, name: bundles[piece].global, file: bundles[piece].file });
}
