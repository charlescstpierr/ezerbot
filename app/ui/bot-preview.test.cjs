// Run with: node --test app/ui/bot-preview.test.cjs
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");

function mediaBody(css, query) {
  const start = css.indexOf(query);
  assert.ok(start >= 0, query);
  const open = css.indexOf("{", start);
  let depth = 0;
  for (let index = open; index < css.length; index += 1) {
    if (css[index] === "{") depth += 1;
    else if (css[index] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(open + 1, index);
    }
  }
  throw new Error(`unclosed ${query}`);
}

test("a short window lets the bot preview name shrink beside its mascot", () => {
  const css = readFileSync(`${__dirname}/styles.css`, "utf8");
  const body = mediaBody(css, "@media (max-height: 720px)");
  const name = body.match(/\.bot-preview-name\s*\{([^}]*)\}/);
  assert.ok(name, "the short-window preview names its own rule");
  assert.match(name[1], /min-width:\s*0/);
  assert.match(name[1], /flex:\s*1\b/);
});
