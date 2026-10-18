import assert from "node:assert/strict";
import { test } from "node:test";
import { createDeepLinkQueue } from "./deep-links.js";

function fixture(options = {}) {
  const visits = [], errors = [], resolves = [];
  let at = 10;
  const queue = createDeepLinkQueue({
    now: () => at,
    resolve: async (raw) => { resolves.push(raw); return { canonical: raw.toLowerCase(), destination: { kind: "compose", text: raw } }; },
    navigate: (destination) => visits.push(destination), report: (error) => errors.push(error), ...options,
  });
  return { queue, visits, errors, resolves, advance: (ms) => { at += ms; } };
}

test("cold boot queues until restoration, deduplicates pending links, preserves order", async () => {
  const p = fixture(); p.queue.receive("A"); p.queue.receive("A"); p.queue.receive("B");
  assert.equal(p.resolves.length, 0);
  p.queue.setReady(true); await p.queue.idle();
  assert.deepEqual(p.visits.map((v) => v.text), ["A", "B"]);
});

test("canonical duplicates are ignored for 2s, then an intentional reopen works", async () => {
  const p = fixture(); p.queue.setReady(true);
  p.queue.receive("A"); p.queue.receive("a"); await p.queue.idle();
  assert.equal(p.visits.length, 1);
  p.advance(1999); p.queue.receive("a"); await p.queue.idle(); assert.equal(p.visits.length, 1);
  p.advance(1); p.queue.receive("a"); await p.queue.idle(); assert.equal(p.visits.length, 2);
});

test("queue full and malformed input give French errors, not unbounded replay", async () => {
  const p = fixture(); for (let n = 0; n < 17; n++) p.queue.receive(String(n));
  p.queue.receive(null); p.queue.receive("x".repeat(2049));
  assert.equal(p.errors.length, 3); assert.match(p.errors[0], /Trop de liens en attente/);
  p.queue.setReady(true); await p.queue.idle(); assert.equal(p.visits.length, 16);
});

test("refused and unavailable destinations show an error and never navigate", async () => {
  const p = fixture({ resolve: async () => { throw new Error("La cible de ce lien est introuvable ou inaccessible sur ce Mac."); } });
  p.queue.setReady(true); p.queue.receive("denied"); await p.queue.idle();
  assert.equal(p.visits.length, 0); assert.match(p.errors[0], /inaccessible/);
});

test("navigation errors do not strand subsequent links", async () => {
  let count = 0;
  const p = fixture({ navigate: () => { if (!count++) throw new Error("Fermez l’éditeur en cours."); } });
  p.queue.receive("a"); p.queue.receive("b"); p.queue.setReady(true); await p.queue.idle();
  assert.equal(count, 2); assert.equal(p.errors.length, 1);
});

test("readiness lost during resolution requeues without delivering or marking seen", async () => {
  let finish;
  const p = fixture({ resolve: (raw) => new Promise((done) => { finish = () => done({ canonical: raw, destination: raw }); }) });
  p.queue.setReady(true); p.queue.receive("a"); p.queue.setReady(false); finish(); await p.queue.idle();
  assert.equal(p.visits.length, 0);
  p.queue.setReady(true); finish(); await p.queue.idle(); assert.equal(p.visits.length, 1);
});

test("a host-admitted duplicate after a renderer reload never navigates", async () => {
  const p = fixture({ resolve: async () => ({ canonical: "a", destination: {}, duplicate: true }) });
  p.queue.setReady(true); p.queue.receive("a"); await p.queue.idle();
  assert.equal(p.visits.length, 0); assert.equal(p.errors.length, 0);
});

test("a late result cannot undo the user's newer navigation", async () => {
  let generation = 0, finish;
  const p = fixture({ generation: () => generation, resolve: () => new Promise((done) => { finish = done; }) });
  p.queue.setReady(true); p.queue.receive("a"); generation++;
  finish({ canonical: "a", destination: {} }); await p.queue.idle();
  assert.equal(p.visits.length, 0); assert.match(p.errors[0], /Navigation annulée/);
});

test("a new application instance never replays the old compose intent", async () => {
  const first = fixture(); first.queue.receive("text"); first.queue.setReady(true); await first.queue.idle();
  const restarted = fixture(); restarted.queue.setReady(true); await restarted.queue.idle();
  assert.equal(restarted.visits.length, 0);
  restarted.queue.receive("text"); await restarted.queue.idle(); assert.equal(restarted.visits.length, 1);
});
