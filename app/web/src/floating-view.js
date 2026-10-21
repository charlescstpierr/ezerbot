export const floatingConversations = (state) => (state?.conversations || []).filter((c) => ["chat", "group"].includes(c.kind));
export const floatingReply = (conversation, textOnly = false) => conversation?.messages?.filter((m) => m.role === "assistant" && [undefined, null, "message", "reply"].includes(m.kind) && (!textOnly || m.body?.trim())).at(-1);

export function resizeFloatingInput(element) {
  if (!element) return;
  element.style.height = "auto";
  element.style.height = `${Math.max(40, Math.min(element.scrollHeight, 120))}px`;
  element.style.overflowY = element.scrollHeight > 120 ? "auto" : "hidden";
}

export function floatingHasMore(element) {
  return Boolean(element && element.scrollHeight - element.scrollTop - element.clientHeight > 2);
}

// A non-primary bot's approval can be routed to the coordinator's thread.
// Match the generating run, not just the card's destination conversation.
export function floatingPendingRequests(state, conversation) {
  const runs = new Set((conversation?.messages || []).map((m) => m.payload?.run_id).filter(Boolean));
  return (state?.forms?.requests || []).filter((request) => request.status?.status === "pending" && runs.has(request.run_id));
}
