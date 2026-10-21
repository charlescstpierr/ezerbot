// View-only overlay. Never change the durable run or automatically replay a
// request whose outcome is unknown. An authoritative snapshot removes it.
export function connectionMessages(messages, connected) {
  if (connected) return messages;
  return messages.map((message) => message.role === "assistant" && (message.state === "streaming" || ["queued", "running"].includes(message.payload?.turn_status))
    ? { ...message, state: "failed", error_kind: "local_connection", error: "La connexion au moteur local est interrompue. La réponse peut avoir continué : réessaie pour retrouver son état, sans renvoyer ta question.", payload: { ...message.payload, turn_status: "disconnected", wait_reason: null } }
    : message);
}

// Shared presentation policies are executable in the shell and thread tests.
// Keep the authoritative run untouched; offline controls are never Stop buttons.
export function composerConnectionState({ offline = false, live = false, busy = false, group = false }) {
  const active = live && !offline;
  const title = offline ? "Moteur local indisponible" : active ? group ? "Arrêter les tours" : "Arrêter la réponse" : "Envoyer le message";
  return { offline, live: active, busy: busy && !offline, mode: active ? "stop" : "send", icon: active ? "#i-stop" : "#i-send", title,
    label: offline ? "Envoyer le message — moteur local indisponible" : active ? title : busy ? "Envoi en cours" : title };
}
