import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Run the shell's actual pre-commit policy and callback. A synchronous scroll
// before React commits would see the old height and leave the receipt hidden.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const start = shell.indexOf("  function render(followGroup");
const policy = shell.slice(start, shell.indexOf("    const hasMessages", start)) + "return onLayout; }";
function fixture(kind = "chat", memory = true) {
  let conversation = { id: "thread", kind, messages: memory ? [{ kind: "memory_auto" }] : [] };
  const scroll = { scrollHeight: 2000, clientHeight: 600, top: 1400,
    get scrollTop() { return this.top; },
    set scrollTop(value) { this.top = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight)); },
  };
  const approval = { pending: false };
  const render = runInNewContext(`(${policy})`, { currentConversation: () => conversation, ui: { scroll }, scrollToPendingApproval: () => approval.pending });
  return { scroll, render, approval, switchConversation: () => { conversation = { id: "other" }; } };
}

test("a pinned solo thread follows a background receipt only after its React layout", () => {
  const f = fixture(), onLayout = f.render();
  assert.equal(typeof onLayout, "function");
  f.scroll.scrollHeight += 125; // Receipt is committed, after streaming ended.
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

test("a pending approval keeps priority over following a new memory receipt", () => {
  const f = fixture(); f.approval.pending = true;
  const onLayout = f.render(); f.scroll.scrollHeight += 125; onLayout();
  assert.equal(f.scroll.scrollTop, 1400);
});
