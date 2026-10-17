import { resolve } from "node:path";
import { copyFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const outDir = resolve(import.meta.dirname, "../../.lavish/chat");

// Builds the review page shown in Lavish before a design goes into the app. Lavish serves
// artifacts from a sandboxed frame, where module scripts are blocked, hence one classic script.
export default defineConfig({
  plugins: [react(), tailwindcss(), { name: "preview-page", closeBundle: () => copyFileSync(resolve(import.meta.dirname, "preview/index.html"), resolve(outDir, "index.html")) }],
  define: { "process.env.NODE_ENV": '"production"' },
  build: {
    outDir,
    emptyOutDir: true,
    cssCodeSplit: false,
    lib: { entry: resolve(import.meta.dirname, "preview/preview.jsx"), formats: ["iife"], name: "GrokbotChatPreview", fileName: () => "preview.js", cssFileName: "preview" },
  },
});
