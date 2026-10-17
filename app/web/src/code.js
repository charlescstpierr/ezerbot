import { createHighlighterCoreSync } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import bash from "@shikijs/langs/bash";
import c from "@shikijs/langs/c";
import cpp from "@shikijs/langs/cpp";
import css from "@shikijs/langs/css";
import diff from "@shikijs/langs/diff";
import go from "@shikijs/langs/go";
import html from "@shikijs/langs/html";
import java from "@shikijs/langs/java";
import javascript from "@shikijs/langs/javascript";
import json from "@shikijs/langs/json";
import jsx from "@shikijs/langs/jsx";
import markdown from "@shikijs/langs/markdown";
import python from "@shikijs/langs/python";
import rust from "@shikijs/langs/rust";
import sql from "@shikijs/langs/sql";
import swift from "@shikijs/langs/swift";
import toml from "@shikijs/langs/toml";
import tsx from "@shikijs/langs/tsx";
import typescript from "@shikijs/langs/typescript";
import yaml from "@shikijs/langs/yaml";

// The stock @streamdown/code plugin loads every Shiki grammar on demand. The app is one
// embedded script with no network, so this plugin ships a fixed set and highlights synchronously.
const themes = ["github-light", "github-dark"];
const highlighter = createHighlighterCoreSync({
  themes: [githubLight, githubDark],
  langs: [bash, c, cpp, css, diff, go, html, java, javascript, json, jsx, markdown, python, rust, sql, swift, toml, tsx, typescript, yaml],
  engine: createJavaScriptRegexEngine({ forgiving: true }),
});
const languages = new Set(highlighter.getLoadedLanguages());

function resolve(language) {
  const name = String(language || "").trim().toLowerCase();
  return languages.has(name) ? name : "text";
}

export const code = {
  name: "shiki",
  type: "code-highlighter",
  supportsLanguage: (language) => resolve(language) !== "text",
  getSupportedLanguages: () => [...languages],
  getThemes: () => themes,
  highlight: ({ code: source, language }) => highlighter.codeToTokens(source, { lang: resolve(language), themes: { light: themes[0], dark: themes[1] } }),
};
