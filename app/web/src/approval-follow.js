// Follow only newly arriving pending approval cards for a submission made while
// pinned. Programmatic scrolls do not cancel this intent; explicit user navigation
// does. This survives asynchronous React commits and multi-poll model responses.
// The shell's post-commit follow and the local resize anchor must choose the
// same target. Otherwise a later React layout can hide the local decisions again.
export function revealPendingApproval(card, viewport) {
  if (!card) return;
  const actions = card.querySelector?.("[data-follow-actions] fieldset");
  if (actions && card.getBoundingClientRect().height > viewport?.clientHeight) actions.scrollIntoView({ block: "end" });
  else card.scrollIntoView({ block: "start" });
}

export function createApprovalFollow() {
  let conversation = null, active = false, seen = new Set(), target = null;
  const pending = (message) => message.kind === "approval" && message.payload?.status?.status === "pending";
  return {
    arm(id, messages, pinned) {
      conversation = id; active = pinned; target = null;
      seen = new Set(messages.filter(pending).map((message) => message.id));
    },
    bind(id) { if (conversation === null) conversation = id; },
    cancel() { active = false; target = null; },
    observe(id, messages) {
      if (!active || id !== conversation) return null;
      const cards = messages.filter(pending);
      for (const card of cards) {
        if (!seen.has(card.id)) target = card.id;
        seen.add(card.id);
      }
      if (!cards.some((card) => card.id === target)) target = null;
      return target;
    },
  };
}
