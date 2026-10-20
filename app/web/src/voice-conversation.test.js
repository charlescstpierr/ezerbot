import assert from "node:assert/strict";
import { test } from "node:test";
import { createVoiceReader, spokenText, voiceNeedsAction } from "./voice-conversation.js";
const reply = (body, state = "streaming", id = "r") => ({id,body,state,role:"assistant",kind:"reply"});
test("streamed sentences are spoken once before completion; tails flush at end", () => {
  const reader = createVoiceReader([reply("Ancienne réponse.","echoed","old")]);
  assert.deepEqual(reader.read([reply("Bonjour. La suite")]),["Bonjour."]);
  assert.deepEqual(reader.read([reply("Bonjour. La suite arrive ! Et")]),["La suite arrive !"]);
  assert.deepEqual(reader.read([reply("Bonjour. La suite arrive ! Et voilà","echoed")]),["Et voilà"]);
  assert.deepEqual(reader.read([reply("Bonjour. La suite arrive ! Et voilà","echoed")]),[]);
});
test("cards and secrets never become audio; pause retains unread sentences, interruption skips them", () => {
  const reader = createVoiceReader();
  const messages = [reply("Première phrase."),{id:"s",role:"assistant",kind:"secret",body:"ne pas lire",payload:{status:{status:"pending"}}}];
  assert.equal(voiceNeedsAction(messages),true);
  assert.deepEqual(reader.read(messages,true),[]);
  assert.deepEqual(reader.read(messages),["Première phrase."]);
  reader.interrupt(); assert.deepEqual(reader.read([reply("Première phrase. Trop tard.")]),[]);
  reader.nextTurn(); assert.deepEqual(reader.read([reply("Première phrase. Trop tard."),reply("Nouveau tour.","echoed","next")]),["Nouveau tour."]);
  assert.deepEqual(reader.read([reply("Erreur technique.","failed","bad")]),[]);
});
test("French text, markup and long Unicode phrases obey the 1500 UTF-16 bound", () => {
  assert.equal(spokenText("## Bonjour **à vous**. [Lien](https://exemple.com)\n```sh\nsecret\n```"),"Bonjour à vous. Lien");
  const chunks = createVoiceReader().read([reply("é😀".repeat(1200),"echoed")]);
  assert.ok(chunks.length > 1);
  assert.ok(chunks.every(t => t.length <= 1500 && !/[\uD800-\uDBFF]$/.test(t)));
});
