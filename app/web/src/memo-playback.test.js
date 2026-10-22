import assert from "node:assert/strict";
import { test } from "node:test";
import { createMemoPlayer, formatMemoSpeed, parseMemoSpeed, startingMemoSpeed, UNTIMED_WORD_NOTICE } from "./memo-playback.js";

function audioStub() {
  const audio = { playbackRate: 1, currentTime: 0, plays: 0, play() { audio.plays += 1; return Promise.resolve(); } };
  return audio;
}

test("a speed from 0,5 to 2 is accepted with a comma or a point", () => {
  assert.equal(parseMemoSpeed("0,5"), 0.5);
  assert.equal(parseMemoSpeed("2"), 2);
  assert.equal(parseMemoSpeed(" 1.25 "), 1.25);
  assert.equal(formatMemoSpeed(1.5), "1,5");
  assert.equal(formatMemoSpeed(2), "2");
});

test("a speed outside the range or not a number is refused", () => {
  for (const text of ["3", "2,01", "0,49", "0", "-1", "", "vite", "1,5x", "NaN", "Infinity", "1e1"]) assert.equal(parseMemoSpeed(text), null, text);
});

test("the memo starts at the speed of its bot, or at 1 when that speed is unusable", () => {
  assert.equal(startingMemoSpeed(1.5), 1.5);
  for (const unusable of [undefined, null, 0, 3, Number.NaN, "2"]) assert.equal(startingMemoSpeed(unusable), 1);
});

test("the player follows the chosen speed", () => {
  const audio = audioStub();
  const player = createMemoPlayer(audio);
  assert.equal(player.setSpeed("2").ok, true);
  assert.equal(audio.playbackRate, 2);
  assert.equal(player.setSpeed("0,5").ok, true);
  assert.equal(audio.playbackRate, 0.5);
  assert.equal(player.speed, 0.5);
});

test("a speed beyond 2 is refused and the previous speed stays", () => {
  const audio = audioStub();
  const player = createMemoPlayer(audio, 1.5);
  player.apply();
  const refused = player.setSpeed("3");
  assert.equal(refused.ok, false);
  assert.equal(refused.speed, 1.5);
  assert.match(refused.message, /de 0,5 à 2/);
  assert.match(refused.message, /reste 1,5/);
  assert.equal(audio.playbackRate, 1.5);
});

test("a click on a word places the playback at the start of that word and plays", () => {
  const audio = audioStub();
  const player = createMemoPlayer(audio);
  player.setSpeed("2");
  const jump = player.jump({ text: "milieu", start_ms: 9500 });
  assert.equal(jump.ok, true);
  assert.equal(audio.currentTime, 9.5);
  assert.equal(audio.playbackRate, 2, "the jump keeps the chosen speed");
  assert.equal(audio.plays, 1);
});

test("a word without a start does not move the playback", () => {
  const audio = audioStub();
  audio.currentTime = 4;
  const player = createMemoPlayer(audio);
  for (const word of [{ text: "sans", start_ms: null }, { text: "absent" }, { text: "négatif", start_ms: -5 }, null]) {
    const jump = player.jump(word);
    assert.equal(jump.ok, false);
    assert.equal(jump.message, UNTIMED_WORD_NOTICE);
  }
  assert.equal(audio.currentTime, 4);
  assert.equal(audio.plays, 0);
});

test("a word at the very start of the memo is a target", () => {
  const audio = audioStub();
  audio.currentTime = 7;
  assert.equal(createMemoPlayer(audio).jump({ text: "Bonjour", start_ms: 0 }).ok, true);
  assert.equal(audio.currentTime, 0);
});

test("a refused play promise does not escape", async () => {
  const audio = audioStub();
  audio.play = () => Promise.reject(new Error("autoplay"));
  assert.equal(createMemoPlayer(audio).jump({ text: "mot", start_ms: 100 }).ok, true);
  await new Promise((resolve) => setImmediate(resolve));
});
