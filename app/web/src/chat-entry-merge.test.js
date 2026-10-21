import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("the chat entry exports the voice reader and mounts the floating panel", () => {
  const source = readFileSync(new URL("./main.jsx", import.meta.url), "utf8");
  assert.match(source, /import \{ createVoiceReader, voiceNeedsAction \} from "\.\/voice-conversation\.js";/);
  assert.match(source, /import \{ mountFloating \} from "\.\/floating\.jsx";/);
  assert.match(source, /if \(document\.getElementById\("floating-root"\)\) mountFloating\(document\.getElementById\("floating-root"\)\);/);
  const api = source.slice(source.indexOf("window.GrokbotChat = {"));
  assert.match(api, /createVoiceReader, voiceNeedsAction,/);
});
