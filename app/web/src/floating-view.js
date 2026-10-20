export const floatingConversations = (state) => (state?.conversations || []).filter((c) => ["chat", "group"].includes(c.kind));
export const floatingReply = (conversation) => conversation?.messages?.filter((m) => m.role === "assistant" && [undefined, null, "message", "reply"].includes(m.kind)).at(-1);

// A non-primary bot's approval can be routed to the coordinator's thread.
// Match the generating run, not just the card's destination conversation.
export function floatingPendingRequests(state, conversation) {
  const runs = new Set((conversation?.messages || []).map((m) => m.payload?.run_id).filter(Boolean));
  return (state?.forms?.requests || []).filter((request) => request.status?.status === "pending" && runs.has(request.run_id));
}
