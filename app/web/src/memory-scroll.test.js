import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Execute the shell's actual pre-commit policy/callback, including coalesced
// React renders. A frame callback before commit only sees the old scroll height.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const start = shell.indexOf("  function render(followLatest");
assert.ok(start >= 0);
const policy = shell.slice(start, shell.indexOf("    const hasMessages", start)) + "return onLayout; }";
const pollStart = shell.indexOf("    const current = currentConversation();", shell.indexOf("  async function loadState()"));
const pollPolicy = shell.slice(pollStart, shell.indexOf("    schedulePoll();", pollStart));
function fixture(kind = "chat", memory = true) {
  let conversation = { id: "thread", kind, messages: memory ? [{ kind: "memory_auto" }] : [] };
  const scroll = { scrollHeight: 2000, clientHeight: 600, top: 1400,
    get scrollTop() { return this.top; },
    set scrollTop(value) { this.top = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight)); },
  };
  const approval = { pending: false };
  const context = { currentConversation: () => conversation, ui: { scroll }, scrollToPendingApproval: () => approval.pending,
    followAfterLayout: null, scrollGeneration: 0 };
  const render = runInNewContext(`(${policy})`, context);
  return { scroll, render, approval,
    replaceConversation: (value) => { conversation = value; },
    switchConversation: () => { conversation = { id: "other" }; },
    cancel: () => { context.scrollGeneration += 1; context.followAfterLayout = null; },
    poll(previous, current, pinned) {
      conversation = current;
      let layout;
      runInNewContext(pollPolicy, { currentConversation: () => current, previous, previousTail: previous.messages.at(-1)?.id,
        wasLive: previous.messages.some(m => m.state === "streaming"), pinned,
        streamingMessage: () => current.messages.find(m => m.state === "streaming"), render: (follow) => { layout = render(follow); } });
      return layout;
    },
  };
}

test("a pinned solo thread follows a background receipt only after its React layout", () => {
  const f = fixture(), onLayout = f.render();
  f.scroll.scrollHeight += 125;
  assert.equal(f.scroll.scrollTop, 1400);
  onLayout();
  assert.equal(f.scroll.scrollTop, f.scroll.scrollHeight - f.scroll.clientHeight);
});

test("background receipts do not pull a solo reader back down", () => {
  const f = fixture(); f.scroll.scrollTop = 100;
  f.scroll.scrollHeight += 125; f.render()();
  assert.equal(f.scroll.scrollTop, 100);
});

test("group follow remains intact and delayed layout cannot scroll a different thread", () => {
  const f = fixture("group", false), onLayout = f.render();
  f.scroll.scrollHeight += 150; onLayout(); assert.equal(f.scroll.scrollTop, 1550);
  const stale = f.render(); f.switchConversation(); f.scroll.scrollHeight += 100;
  stale(); assert.equal(f.scroll.scrollTop, 1550);
  const plain = fixture("chat", false); plain.scroll.scrollHeight += 100; plain.render()();
  assert.equal(plain.scroll.scrollTop, 1400);
  const up = fixture("group", false); up.scroll.scrollTop = 100; up.render()();
  assert.equal(up.scroll.scrollTop, 100);
});

test("a pending approval keeps priority over following a new memory receipt or terminal reply", () => {
  for (const memory of [false, true]) {
    const f = fixture("chat", memory); f.approval.pending = true;
    const onLayout = f.render(true); f.scroll.scrollHeight += 125; onLayout();
    assert.equal(f.scroll.scrollTop, 1400);
  }
});

test("three direct sends retain their follow request across a coalesced busy render", () => {
  const f = fixture("chat", false);
  for (let send = 0; send < 3; send++) {
    f.render(true); // React has not committed this intermediate render.
    const committed = f.render(); // finally/busy render replaces it.
    const oldTop = f.scroll.scrollTop;
    f.scroll.scrollHeight += 190;
    assert.equal(f.scroll.scrollTop, oldTop);
    committed();
    assert.equal(f.scroll.scrollTop, f.scroll.scrollHeight - f.scroll.clientHeight);
  }
});

test("pinned polls follow terminal errors and one-chunk successes after layout", () => {
  for (const terminal of ["failed", "echoed"]) for (const existed of [false, true]) {
    const f = fixture("chat", false);
    const previous = { id: "thread", messages: [{ id: "user", role: "user" }, ...(existed ? [{ id: "reply", state: "streaming" }] : [])] };
    const current = { id: "thread", messages: [{ id: "user", role: "user" }, { id: "reply", state: terminal }] };
    const layout = f.poll(previous, current, true);
    f.scroll.scrollHeight += 190;
    layout();
    assert.equal(f.scroll.scrollTop, f.scroll.scrollHeight - f.scroll.clientHeight);
  }
});

test("terminal polls never pull an unpinned reader down", () => {
  const f = fixture("chat", false); f.scroll.scrollTop = 100;
  const previous = { id: "thread", messages: [{ id: "reply", state: "streaming" }] };
  const layout = f.poll(previous, { id: "thread", messages: [{ id: "reply", state: "failed" }] }, false);
  f.scroll.scrollHeight += 190; layout();
  assert.equal(f.scroll.scrollTop, 100);
});

test("resizing a pinned viewport follows after layout, but never moves a reader who scrolled up", () => {
  const start = shell.indexOf("  function observeViewportResize()");
  const install = shell.slice(start, shell.indexOf("  function newConversation()", start));
  let resized, follows = 0;
  const listeners = {};
  const scroll = { scrollHeight: 2000, clientWidth: 860, clientHeight: 600, scrollTop: 1400,
    addEventListener: (name, callback) => { listeners[name] = callback; } };
  class ResizeObserver { constructor(callback) { resized = callback; } observe(target) { assert.equal(target, scroll); } }
  runInNewContext(`${install}; observeViewportResize();`, { ResizeObserver, ui: { scroll }, scrollToLatest: () => { follows++; } });
  resized(); assert.equal(follows, 0);
  scroll.clientHeight = 300; scroll.clientWidth = 560;
  listeners.scroll(); // A resize's own scroll event must not lose the old pin.
  resized(); assert.equal(follows, 1);
  scroll.scrollTop = 100; listeners.scroll();
  scroll.clientHeight = 600; scroll.clientWidth = 860;
  resized(); assert.equal(follows, 1);
});

test("reading gestures between render and layout cancel both explicit and automatic follow", () => {
  for (const memory of [false, true]) {
    const f = fixture("chat", memory), layout = f.render(true);
    f.cancel(); f.scroll.scrollTop = 100; f.scroll.scrollHeight += 190; layout();
    assert.equal(f.scroll.scrollTop, 100);
    f.render()(); assert.equal(f.scroll.scrollTop, 100);
  }
});
