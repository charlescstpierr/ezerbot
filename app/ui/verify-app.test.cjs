// Run with: node --test app/ui/verify-app.test.cjs
// The 820 frame used to restyle .composer-area only. A wide window then
// failed Ezerbot and Rédacteur even though the composer width was forced.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");

const source = readFileSync(`${__dirname}/../../tools/verify-app.py`, "utf8");

test("verify-app resizes the window to 820×560 before measuring that frame", () => {
  assert.match(source, /for width, height in \(\(1140, 780\), \(820, 560\)\):/);
  assert.match(source, /chrome\('resize', str\(width\), str\(height\)\)/);
  assert.match(source, /window\.__qaRunFrame/);
  assert.match(
    source,
    /innerWidth !== width \|\| innerHeight !== height/,
    "a frame that did not resize the viewport must fail instead of checking a wide window",
  );
});
