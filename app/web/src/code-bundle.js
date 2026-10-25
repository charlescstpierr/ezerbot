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

// Built alone into chat-code.js. The stock @streamdown/code plugin loads each grammar on
// demand from a CDN; this file ships a fixed set and highlights synchronously once loaded.
const highlighter = createHighlighterCoreSync({
  themes: [githubLight, githubDark],
  langs: [bash, c, cpp, css, diff, go, html, java, javascript, json, jsx, markdown, python, rust, sql, swift, toml, tsx, typescript, yaml],
  engine: createJavaScriptRegexEngine({ forgiving: true }),
});

export const languages = highlighter.getLoadedLanguages();

export const highlight = (source, language) => highlighter.codeToTokens(source, { lang: language, themes: { light: "github-light", dark: "github-dark" } });
