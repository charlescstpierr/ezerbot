import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
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

// KaTeX's @font-face lists woff2, woff and ttf, which lib mode would inline as base64 into
// chat.css. Each face keeps only its woff2, served from /katex/, fetched when a glyph needs it.
// The fonts are copied there after the build, so /katex/ is external to Vite.
const katexFonts = {
  postcssPlugin: "katex-fonts",
  AtRule: {
    "font-face": (rule) => rule.walkDecls("src", (decl) => {
      const font = decl.value.match(/KaTeX_[\w-]+(?=\.woff2)/)?.[0];
      if (font) decl.value = `url(/katex/${font}.woff2) format("woff2")`;
    }),
  },
};

function iife({ entry, name, file, plugins = [], alias = [], postcss = [], copyPublicDir = false }) {
  return build({
    root,
    configFile: false,
    plugins,
    resolve: { alias },
    css: { postcss: { plugins: postcss } },
    define: { "process.env.NODE_ENV": '"production"' },
    build: {
      outDir,
      emptyOutDir: false,
      copyPublicDir,
      cssCodeSplit: false,
      rolldownOptions: { external: [/^\/katex\//] },
      lib: { entry: resolve(root, entry), formats: ["iife"], name, fileName: () => file, cssFileName: "chat" },
    },
  });
}

await iife({ entry: "src/main.jsx", name: "GrokbotChatBundle", file: "chat.js", plugins: [react(), tailwindcss()], postcss: [katexFonts], copyPublicDir: true });
const fonts = resolve(root, "node_modules/katex/dist/fonts");
mkdirSync(resolve(outDir, "katex"), { recursive: true });
for (const font of readdirSync(fonts).filter((name) => name.endsWith(".woff2"))) copyFileSync(resolve(fonts, font), resolve(outDir, "katex", font));
const pieces = {
  code: { entry: "src/code-bundle.js" },
  mermaid: { entry: "src/mermaid-bundle.js", alias: [{ find: /^katex$/, replacement: resolve(root, "src/mermaid-katex.js") }] },
  math: { entry: "src/math-bundle.js" },
};
for (const [piece, options] of Object.entries(pieces)) {
  await iife({ ...options, name: bundles[piece].global, file: bundles[piece].file });
}
