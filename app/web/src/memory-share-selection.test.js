import assert from "node:assert/strict";
import { test } from "node:test";
import { reconcileShareSelection } from "./memory-share-selection.js";

const work = (fact_id, extra = {}) => ({ fact_id, class: "work", sensitive: false, shared: false, ...extra });
const message = (id, facts) => ({ id, payload: { facts } });
const ids = (state) => [...state.selected].sort();

test("unchecking a work fact survives a refresh that marks another fact shared", () => {
  let state = reconcileShareSelection(null, message("card-1", [work("a"), work("b"), work("c")]));
  assert.deepEqual(ids(state), ["a", "b", "c"]);
  state = { ...state, selected: new Set(["a", "b"]) };
  state = reconcileShareSelection(state, message("card-1", [work("a", { shared: true }), work("b"), work("c")]));
  assert.deepEqual(ids(state), ["b"]);
});

test("a cleared selection stays clear when the same card refreshes after a partial share", () => {
  const state = reconcileShareSelection(
    { messageId: "card-1", selected: new Set() },
    message("card-1", [work("a", { shared: true }), work("b", { shared: true }), work("c")]),
  );
  assert.deepEqual(ids(state), []);
});

test("a new message preselects only its eligible work facts", () => {
  const first = reconcileShareSelection(null, message("card-1", [work("a")]));
  const next = reconcileShareSelection(first, message("card-2", [
    work("b"),
    work("c", { sensitive: true }),
    work("d", { shared: true }),
    { fact_id: "p", class: "personal", sensitive: false, shared: false },
  ]));
  assert.deepEqual(ids(next), ["b"]);
});

test("a personal fact checked by hand survives a refresh of the same message", () => {
  let state = reconcileShareSelection(null, message("card-1", [
    work("a"),
    { fact_id: "p", class: "personal", sensitive: false, shared: false },
  ]));
  state = { ...state, selected: new Set([...state.selected, "p"]) };
  state = reconcileShareSelection(state, message("card-1", [
    work("a", { shared: true }),
    { fact_id: "p", class: "personal", sensitive: false, shared: false },
  ]));
  assert.deepEqual(ids(state), ["p"]);
});

test("shared, sensitive and missing facts leave the selection", () => {
  let state = reconcileShareSelection(null, message("card-1", [work("a"), work("b"), work("c"), work("d")]));
  state = reconcileShareSelection(state, message("card-1", [
    work("a", { shared: true }),
    work("b", { sensitive: true }),
    work("d"),
  ]));
  assert.deepEqual(ids(state), ["d"]);
});
