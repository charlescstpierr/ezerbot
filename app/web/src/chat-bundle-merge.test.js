import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("the committed chat bundle keeps voice, partial meetings and the compact panel", () => {
  const script = readFileSync(new URL("../../ui/chat.js", import.meta.url), "utf8");
  const css = readFileSync(new URL("../../ui/chat.css", import.meta.url), "utf8");
  for (const marker of ["createVoiceReader", "voiceNeedsAction", "floating-root", "ezerbotFloating"]) {
    assert.ok(script.includes(marker), marker);
  }
  assert.match(css, /\.gb-meeting-partial\{/);
  assert.match(css, /\.floating-panel\{/);
  assert.match(css, /-webkit-line-clamp:2/);
});
