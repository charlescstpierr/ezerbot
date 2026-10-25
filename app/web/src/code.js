import { loadBundle, loadedBundle } from "./bundles.js";

// The languages chat-code.js highlights, aliases and embedded grammars included, so a block
// in any other language never downloads Shiki. lazy-plugins.test.js keeps it equal to the bundle.
const languages = new Set(["shellscript", "c", "regexp", "glsl", "cpp-macro", "cpp", "css", "diff", "go", "javascript", "html", "java", "json", "jsx", "python", "rust", "sql", "swift", "toml", "tsx", "typescript", "yaml", "markdown", "bash", "sh", "shell", "zsh", "md", "regex", "c++", "js", "cjs", "mjs", "py", "rs", "ts", "cts", "mts", "yml"]);
const themes = ["github-light", "github-dark"];

function resolve(language) {
  const name = String(language || "").trim().toLowerCase();
  return languages.has(name) ? name : null;
}

// Streamdown shows the plain code until highlight returns tokens or calls back with them.
export const code = {
  name: "shiki",
  type: "code-highlighter",
  supportsLanguage: (language) => resolve(language) !== null,
  getSupportedLanguages: () => [...languages],
  getThemes: () => themes,
  highlight({ code: source, language }, callback) {
    const lang = resolve(language);
    if (!lang) return null;
    const bundle = loadedBundle("code");
    if (bundle) return bundle.highlight(source, lang);
    loadBundle("code").then((loaded) => callback?.(loaded.highlight(source, lang)), () => {});
    return null;
  },
};
