import assert from "node:assert/strict";
import { test } from "node:test";
import { installScriptDocument } from "./fake-script-document.js";

let fresh = 0;
const loader = () => import(`./bundles.js?instance=${fresh++}`);

test("deux demandes simultanées d’un morceau n’ajoutent qu’un script, et la suivante le réutilise", async () => {
  const page = installScriptDocument();
  const { loadBundle } = await loader();
  const first = loadBundle("code");
  const second = loadBundle("code");
  assert.equal(page.appended.length, 1);
  assert.equal(page.appended[0].tagName, "script");
  assert.equal(page.appended[0].src, "/chat-code.js");
  const bundle = { highlight() {} };
  page.arrive(page.appended[0], "GrokbotCode", bundle);
  assert.equal(await first, bundle);
  assert.equal(await second, bundle);
  assert.equal(await loadBundle("code"), bundle);
  assert.equal(page.appended.length, 1);
  delete globalThis.GrokbotCode;
});

test("un échec de chargement retire le script et le message suivant réessaie", async () => {
  const page = installScriptDocument();
  const { loadBundle } = await loader();
  const failed = loadBundle("code");
  page.fail(page.appended[0]);
  await assert.rejects(failed, /chat-code\.js/);
  assert.deepEqual(page.live("/chat-code.js"), []);
  const retried = loadBundle("code");
  assert.equal(page.appended.length, 2);
  const bundle = { highlight() {} };
  page.arrive(page.appended[1], "GrokbotCode", bundle);
  assert.equal(await retried, bundle);
  delete globalThis.GrokbotCode;
});

test("un script chargé qui ne définit pas son global compte comme un échec", async () => {
  const page = installScriptDocument();
  const { loadBundle } = await loader();
  const empty = loadBundle("code");
  page.arrive(page.appended[0], "GrokbotCode", undefined);
  await assert.rejects(empty);
  loadBundle("code");
  assert.equal(page.live("/chat-code.js").length, 1);
});
