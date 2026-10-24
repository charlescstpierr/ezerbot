import assert from "node:assert/strict";
import { test } from "node:test";
import { installScriptDocument } from "./fake-script-document.js";
import { code } from "./code.js";
import { createMermaidPlugin } from "./mermaid.js";
import * as codeBundle from "./code-bundle.js";

const page = installScriptDocument();
const rust = "fn main() {}";
const text = (result) => result.tokens.map((line) => line.map((token) => token.content).join("")).join("\n");

test("un bloc de code reste en clair jusqu’à l’arrivée de chat-code.js, puis se colore, même après un premier échec", async () => {
  const results = [];
  assert.equal(code.highlight({ code: rust, language: "rust" }, (result) => results.push(result)), null);
  assert.equal(page.live("/chat-code.js").length, 1);
  page.fail(page.live("/chat-code.js")[0]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(results, []);

  assert.equal(code.highlight({ code: rust, language: " Rust " }, (result) => results.push(result)), null);
  const [script] = page.live("/chat-code.js");
  page.arrive(script, "GrokbotCode", codeBundle);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(results.length, 1);
  assert.equal(text(results[0]), rust);
  assert.equal(results[0].tokens[0][0].htmlStyle.color, "#D73A49");

  const now = code.highlight({ code: "let a = 1;", language: "js" }, () => assert.fail("already loaded, no callback"));
  assert.equal(text(now), "let a = 1;");
  assert.equal(page.appended.length, 2);
});

test("un bloc sans langue connue ne télécharge pas la coloration", () => {
  const before = page.appended.length;
  delete globalThis.GrokbotCode;
  for (const language of ["", "text", "brainfuck", undefined]) {
    assert.equal(code.highlight({ code: "a b c", language }, () => assert.fail("never highlighted")), null);
  }
  assert.equal(page.appended.length, before);
});

test("le plugin annonce exactement les langues que chat-code.js colore", () => {
  assert.deepEqual(new Set(code.getSupportedLanguages()), new Set(codeBundle.languages));
  assert.equal(code.supportsLanguage("TSX"), true);
  assert.equal(code.supportsLanguage("text"), false);
});

test("un diagramme attend chat-mermaid.js, puis Mermaid le dessine avec la configuration du fil, même après un premier échec", async () => {
  const plugin = createMermaidPlugin({ config: { theme: "base" } });
  const first = plugin.getMermaid({ fontFamily: "serif" }).render("d1", "graph TD; A-->B;");
  const second = plugin.getMermaid().render("d2", "graph LR; C-->D;");
  assert.equal(page.live("/chat-mermaid.js").length, 1);
  page.fail(page.live("/chat-mermaid.js")[0]);
  await assert.rejects(first, /chat-mermaid\.js/);
  await assert.rejects(second, /chat-mermaid\.js/);

  const retried = plugin.getMermaid({ fontFamily: "serif" }).render("d3", "graph TD; A-->B;");
  const drawn = [];
  const bundle = {
    createMermaidPlugin: (options) => ({
      getMermaid: (config) => ({ render: async (id, source) => { drawn.push({ options, config, id, source }); return { svg: `<svg id="${id}"></svg>` }; } }),
    }),
  };
  page.arrive(page.live("/chat-mermaid.js")[0], "GrokbotMermaid", bundle);
  assert.deepEqual(await retried, { svg: '<svg id="d3"></svg>' });
  assert.deepEqual(drawn, [{ options: { config: { theme: "base" } }, config: { fontFamily: "serif" }, id: "d3", source: "graph TD; A-->B;" }]);
  await plugin.getMermaid().render("d4", "graph TD; X-->Y;");
  assert.equal(page.appended.filter((script) => script.src === "/chat-mermaid.js").length, 2);
});
