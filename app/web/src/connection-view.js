// View-only overlay. Never change the durable run or automatically replay a
// request whose outcome is unknown. An authoritative snapshot removes it.
export function connectionMessages(messages, connected) {
  if (connected) return messages;
  return messages.map((message) => message.role === "assistant" && (message.state === "streaming" || ["queued", "running"].includes(message.payload?.turn_status))
    ? { ...message, state: "failed", error_kind: "local_connection", error: "La connexion au moteur local est interrompue. La réponse peut avoir continué : réessayez pour retrouver son état, sans renvoyer votre question.", payload: { ...message.payload, turn_status: "disconnected", wait_reason: null } }
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

export function signInCardState({ message, isGroup = false, isTask = false, providerConnected = false, engineConnected = true }) {
  const expired = message.error_kind === "signin_expired";
  const group = isGroup || Boolean(message.payload?.turn_id);
  return {
    title: providerConnected ? "Compte ChatGPT connecté" : expired ? "Votre session ChatGPT a expiré" : "Connectez votre compte ChatGPT",
    copy: providerConnected ? group ? "Envoyez un nouveau message pour obtenir une réponse." : isTask ? "Poursuivez dans la conversation principale." : "Votre question est conservée. Réessayez pour obtenir la réponse." : group ? "Connectez votre compte ChatGPT via Codex, puis envoyez un nouveau message."
      : isTask ? "Connectez votre compte ChatGPT via Codex, puis poursuivez dans la conversation principale."
        : "Votre question est conservée. Connectez-vous via Codex, puis réessayez.",
    signInLabel: expired ? "Se reconnecter via Codex" : "Se connecter via Codex",
    showRetry: !group && !isTask,
    retryDisabled: !providerConnected || !engineConnected,
    retryTitle: !engineConnected ? "Moteur local indisponible" : !providerConnected ? "Connectez-vous via Codex avant de réessayer." : "Réessayer cette réponse",
    signInDisabled: !engineConnected,
    showSignIn: !providerConnected,
  };
}

export function replyProblemState(message, { isGroup = false, isTask = false, engineConnected = true } = {}) {
  const signin = ["signin", "signin_expired"].includes(message.error_kind);
  const local = message.error_kind === "local_connection";
  const group = (isGroup || Boolean(message.payload?.turn_id)) && !signin && !local;
  const advice = local || signin || message.error?.toLocaleLowerCase("fr").includes("envoyez un nouveau message") ? null
    : group ? "Envoyez un nouveau message pour demander une nouvelle réponse."
      : isTask ? "Poursuivez dans la conversation principale." : null;
  return { signin, advice, retry: local || (!group && !isTask && message.error_kind !== "context_budget"), retryDisabled: !engineConnected,
    retryTitle: !engineConnected ? "Moteur local indisponible" : "Réessayer cette réponse" };
}
