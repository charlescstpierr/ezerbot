import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The Rust wrapper embeds a fixed list of files and serves them under a strict CSP,
// so the chat ships as one classic script and one stylesheet next to the shell.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: { "process.env.NODE_ENV": '"production"' },
  build: {
    outDir: resolve(import.meta.dirname, "../ui"),
    emptyOutDir: false,
    cssCodeSplit: false,
    lib: { entry: resolve(import.meta.dirname, "src/main.jsx"), formats: ["iife"], name: "GrokbotChatBundle", fileName: () => "chat.js", cssFileName: "chat" },
  },
});
