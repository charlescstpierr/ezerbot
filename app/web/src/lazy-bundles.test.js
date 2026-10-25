import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";

const shipped = (file) => readFileSync(new URL(`../../ui/${file}`, import.meta.url), "utf8");
const has = (bundle, marker) => bundle.includes(marker);

test("chat.js ne contient pas la coloration du code, chat-code.js la contient", () => {
  const chat = shipped("chat.js");
  const code = shipped("chat-code.js");
  for (const grammar of ["source.rust", "source.swift", "source.python"]) {
    assert.equal(has(chat, grammar), false, `${grammar} in chat.js`);
    assert.equal(has(code, `"scopeName":"${grammar}"`), true, `${grammar} in chat-code.js`);
  }
  assert.equal(has(chat, "codeToTokens"), false, "Shiki in chat.js");
  assert.equal(code.startsWith("var GrokbotCode="), true, "chat-code.js sets window.GrokbotCode");
});

test("chat.js ne contient pas Mermaid, chat-mermaid.js le contient", () => {
  const chat = shipped("chat.js");
  const mermaid = shipped("chat-mermaid.js");
  for (const marker of ["No diagram type detected", "flowchart-v2", "cytoscape", "dagre"]) {
    assert.equal(has(chat, marker), false, `${marker} in chat.js`);
    assert.equal(has(mermaid, marker), true, `${marker} in chat-mermaid.js`);
  }
  assert.equal(mermaid.startsWith("var GrokbotMermaid="), true, "chat-mermaid.js sets window.GrokbotMermaid");
});

test("KaTeX n’existe qu’une fois parmi les fichiers servis, dans chat-math.js", () => {
  const copies = ["chat.js", "chat-code.js", "chat-mermaid.js", "chat-math.js"].filter((file) => has(shipped(file), "Unknown stretchy delimiter"));
  assert.deepEqual(copies, ["chat-math.js"]);
  assert.equal(shipped("chat-math.js").startsWith("var GrokbotMath="), true, "chat-math.js sets window.GrokbotMath");
  assert.match(shipped("chat.css"), /\.katex \.katex-version:after\{content:"0\.16\.47"/);
  assert.equal(has(shipped("chat-math.js"), "0.16.47"), true, "KaTeX JS matches the CSS");
});

test("chat.css n’embarque aucune police : chaque face KaTeX pointe vers son woff2 servi sous /katex/", () => {
  const css = shipped("chat.css");
  assert.equal(has(css, "data:font"), false, "inlined font in chat.css");
  const faces = css.match(/@font-face\{[^}]*\}/g);
  const sources = faces.map((face) => face.match(/src:([^;}]*)/)[1]);
  const fonts = readdirSync(new URL("../../ui/katex/", import.meta.url)).sort();
  assert.equal(fonts.length, 20);
  assert.deepEqual(sources.map((source) => source.match(/^url\(\/katex\/(KaTeX_[\w-]+\.woff2)\)format\("woff2"\)$/)?.[1]).sort(), fonts);
});
