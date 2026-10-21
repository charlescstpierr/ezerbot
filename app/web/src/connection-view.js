// View-only overlay. Never change the durable run or automatically replay a
// request whose outcome is unknown. An authoritative snapshot removes it.
export function connectionMessages(messages, connected) {
  if (connected) return messages;
  return messages.map((message) => message.role === "assistant" && (message.state === "streaming" || ["queued", "running"].includes(message.payload?.turn_status))
    ? { ...message, state: "failed", error_kind: "local_connection", error: "La connexion au moteur local est interrompue. La réponse peut avoir continué : réessaie pour retrouver son état, sans renvoyer ta question.", payload: { ...message.payload, turn_status: "disconnected", wait_reason: null } }
    : message);
}
