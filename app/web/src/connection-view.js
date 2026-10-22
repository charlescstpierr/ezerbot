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

function signInCopy({ providerConnected, expired, group, isTask }) {
  if (providerConnected) return group ? "Envoyez un nouveau message pour obtenir une réponse." : isTask ? "Poursuivez dans la conversation principale." : "Votre question est conservée. Réessayez pour obtenir la réponse.";
  if (expired) return group ? "Votre session ChatGPT a expiré. Reconnectez votre compte ChatGPT, puis envoyez un nouveau message."
    : isTask ? "Votre session ChatGPT a expiré. Reconnectez votre compte ChatGPT, puis poursuivez dans la conversation principale."
      : "Votre session ChatGPT a expiré. Votre question est conservée. Reconnectez votre compte ChatGPT, puis réessayez.";
  return group ? "Connectez votre compte ChatGPT, puis envoyez un nouveau message."
    : isTask ? "Connectez votre compte ChatGPT, puis poursuivez dans la conversation principale."
      : "Votre question est conservée. Connectez votre compte ChatGPT, puis réessayez.";
}

export function signInCardState({ message, isGroup = false, isTask = false, providerConnected = false, engineConnected = false }) {
  const expired = message.error_kind === "signin_expired";
  const group = isGroup || Boolean(message.payload?.turn_id);
  return {
    title: providerConnected ? "Compte ChatGPT connecté" : expired ? "Votre session ChatGPT a expiré" : "Connectez votre compte ChatGPT",
    copy: signInCopy({ providerConnected, expired, group, isTask }),
    signInLabel: expired ? "Se reconnecter" : "Se connecter",
    showRetry: !group && !isTask,
    retryDisabled: !providerConnected || !engineConnected,
    retryTitle: !engineConnected ? "Moteur local indisponible" : !providerConnected ? "Connectez votre compte ChatGPT avant de réessayer." : "Réessayer cette réponse",
    signInDisabled: !engineConnected,
    showSignIn: !providerConnected,
    alert: !providerConnected,
  };
}

const NO_EVENT = "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez.";
const STALLED = "La réponse d’OpenAI est bloquée : aucun nouvel événement reçu. Réessayez.";

export function presentedFailureText(error, { retry = true, replacement = "Envoyez un nouveau message." } = {}) {
  let text = error || "";
  // Keepalive pings are not a session renewal: the connection is alive, the answer is slow.
  text = text.replaceAll(NO_EVENT, "Connexion lente… on attend la réponse.");
  text = text.replaceAll(STALLED, retry ? "La réponse n’arrive pas. Réessayer ?" : `La réponse n’arrive pas. ${replacement}`);
  if (!retry) {
    const phrase = replacement.replace(/\.\s*$/, "");
    text = text.replace(/Réessayez\.(?=\s*$)/, `${phrase}.`);
    text = text.replace(/Réessayez(?=\s)/, phrase);
  }
  return text;
}

export function replyProblemState(message, { isGroup = false, isTask = false, engineConnected = true } = {}) {
  const signin = ["signin", "signin_expired"].includes(message.error_kind);
  const local = message.error_kind === "local_connection";
  const group = (isGroup || Boolean(message.payload?.turn_id)) && !signin && !local;
  const retry = local || (!group && !isTask && message.error_kind !== "context_budget");
  const error = presentedFailureText(message.error, { retry, replacement: isTask ? "Poursuivez dans la conversation principale." : "Envoyez un nouveau message." });
  const shown = `${message.error || ""} ${error}`.toLocaleLowerCase("fr");
  const advice = local || signin || shown.includes("envoyez un nouveau message") || shown.includes("poursuivez dans la conversation principale") ? null
    : group ? "Envoyez un nouveau message pour demander une nouvelle réponse."
      : isTask ? "Poursuivez dans la conversation principale." : null;
  return { signin, advice, error, retry, retryDisabled: !engineConnected,
    retryTitle: !engineConnected ? "Moteur local indisponible" : "Réessayer cette réponse" };
}

export function floatingFailureState(reply, { providerConnected } = {}) {
  if (reply?.state !== "failed") return null;
  const expired = reply.error_kind === "signin_expired";
  const signin = expired || reply.error_kind === "signin";
  const reconnected = signin && providerConnected;
  // Without a session the answer cannot have failed on its own: say so neutrally.
  const disconnected = !signin && providerConnected === false;
  const alert = !signin && !disconnected;
  return {
    title: reconnected ? "Compte ChatGPT connecté" : expired ? "Votre session ChatGPT a expiré" : signin ? "Connexion ChatGPT requise" : disconnected ? "Non connecté" : "La réponse a échoué.",
    alert,
    detailClass: alert ? "floating-error" : "",
    detail: reconnected ? "Votre question est conservée. Ouvrez le fil pour reprendre la réponse."
      : presentedFailureText(reply.error, { retry: false, replacement: "Ouvrez le fil." }) || (signin ? "Ouvrez le fil pour vous connecter." : "Ouvrez le fil pour reprendre la réponse."),
  };
}

export function visibleNotice(message) {
  return message.state !== "streaming" && message.notice?.startsWith("Préparation d’un outil") ? null : message.notice;
}

export function preparingTool(message) {
  return message.state === "streaming" && Boolean(visibleNotice(message)?.startsWith("Préparation d’un outil"));
}
