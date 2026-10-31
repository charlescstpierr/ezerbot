const chatKinds = new Set([undefined, null, "message", "submission", "reply"]);
const usefulCards = new Set(["form", "secret", "memory_auto", "memory_share", "memory_overflow", "topic_memory_offer", "meeting", "voice_call", "group_event", "group_notice", "delegation"]);
const approvalProblems = {
  denied: "L’action a été refusée. Elle n’a pas été exécutée.",
  expired: "Cette demande a expiré. Aucune action n’a été faite.",
  cancelled: "La demande a été annulée. Aucune action n’a été faite.",
};
const failureLabels = {
  run_shell: "La commande n’a pas abouti.",
  read_file: "Le fichier n’a pas pu être lu.",
  list_directory: "Le dossier n’a pas pu être consulté.",
  write_file: "Le fichier n’a pas pu être enregistré.",
  create_document: "Le document n’a pas pu être créé.",
  document_action: "L’action sur le document n’a pas abouti.",
  generate_image: "L’image n’a pas pu être créée.",
};

export function threadActivity({ runs = [], conversationId, engineConnected = false } = {}) {
  if (!conversationId || !engineConnected) return [];
  return runs.filter((run) => run.conversation_id === conversationId && run.status === "running")
    .map(({ run_id, bot_id }) => ({ run_id, bot_id }));
}

const replyDiagnostics = {
  signin: "Connectez votre compte ChatGPT pour recevoir une réponse.",
  signin_expired: "Votre session ChatGPT a expiré. Reconnectez-vous à votre compte ChatGPT.",
  limit: "La limite d’utilisation de ce compte ChatGPT est atteinte. Consultez votre utilisation avant de réessayer.",
  local_connection: "La connexion au moteur local est interrompue. La réponse peut avoir continué : réessayez pour retrouver son état, sans renvoyer votre question.",
  context_budget: "Le texte et les pièces jointes dépassent ce que le modèle peut lire en un message. Raccourcissez le texte ou joignez moins de documents dans un nouveau message.",
};
const conversationalErrors = new Set([
  "La réponse a dépassé le délai maximal. Réessayez.",
  "Les connexions précédentes se ferment encore. Réessayez dans une minute.",
  "La réponse d’OpenAI dépasse la taille autorisée. Réessayez avec une demande plus courte.",
  "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez.",
  "La réponse d’OpenAI est bloquée : aucun nouvel événement reçu. Réessayez.",
  "Reconnectez-vous et envoyez un nouveau message.",
  "Les modèles de votre compte ChatGPT ne sont pas encore chargés. Relancez la réponse dans un instant.",
  "La génération distante a été interrompue. Son résultat est incertain ; elle n’a pas été relancée automatiquement.",
  "La réponse a été interrompue avant la fin. Vous pouvez la relancer.",
]);

export function conversationReplyError(message) {
  if (Object.hasOwn(replyDiagnostics, message.error_kind)) return replyDiagnostics[message.error_kind];
  if (conversationalErrors.has(message.error)) return message.error;
  return "La réponse n’a pas abouti. Réessayez.";
}

function toolProblem(message) {
  const { call, outcome, approval_status } = message.payload || {};
  if (outcome?.status !== "failed" && !outcome?.output?.isError
    && !["échec", "annulé", "délai dépassé", "arrêtée"].includes(outcome?.output?.status)) return null;
  if (approvalProblems[approval_status]) return approvalProblems[approval_status];
  if (["annulé", "arrêtée"].includes(outcome?.output?.status)) return "La tâche a été arrêtée.";
  if (outcome?.output?.status === "délai dépassé") return "La commande a pris trop de temps et s’est arrêtée.";
  if (call?.name === "generate_image" && outcome?.reason === "La génération de cette image a été refusée pour des raisons de sécurité.") return outcome.reason;
  return failureLabels[call?.name] || "L’action n’a pas abouti.";
}

function problem(message, text, localState) {
  return { ...message, presentation: { kind: "problem", text, localState } };
}

export function presentThread(orderedMessages) {
  const messages = [], interventions = [];
  const latestTasks = new Map();
  for (const message of orderedMessages) {
    if (message.kind === "task" && message.payload?.run_id
      && (!latestTasks.has(message.payload.run_id) || message.payload.event_kind === "task_changed")) latestTasks.set(message.payload.run_id, message.id);
  }
  const failedReplies = new Set(orderedMessages.filter((message) => chatKinds.has(message.kind) && message.state === "failed")
    .map((message) => message.payload?.run_id).filter(Boolean));
  for (const message of orderedMessages) {
    const payload = message.payload || {};
    if (chatKinds.has(message.kind)) {
      const notice = /^Préparation d[’']un outil/u.test(message.notice || "") ? undefined : message.notice;
      const visible = message.role === "user" || message.body?.trim() || message.attachments?.length
        || notice || ["failed", "stopped"].includes(message.state) || payload.turn_status === "queued";
      if (visible) messages.push(notice === message.notice ? message : { ...message, notice });
      continue;
    }
    if (message.kind === "approval") {
      const reviewable = payload.arguments?.brouillon || payload.kind?.tool === "propose_skill";
      if (reviewable) messages.push(message);
      else if (payload.status?.status === "pending") interventions.push({ kind: "approval", message });
      else if (approvalProblems[payload.status?.status]) messages.push(problem(message, approvalProblems[payload.status.status]));
      else if (payload.configuration_result?.status === "failed") messages.push(problem(message, "La modification n’a pas abouti. Réessayez depuis Paramètres."));
      else if (payload.receipt?.wake?.status === "failed" && !payload.detached) messages.push(problem(message, "La réponse est enregistrée, mais le bot n’a pas pu reprendre."));
      continue;
    }
    if (message.kind === "tool") {
      const { call, outcome, run_id: runId } = payload;
      if (call?.name === "create_document" && outcome?.status === "done" && outcome.output?.id?.startsWith("generated_")) {
        messages.push(message);
        continue;
      }
      if (call?.name === "generate_image" && (outcome?.status === "done" || outcome?.reason === "Connectez votre compte ChatGPT")) {
        messages.push(message);
        continue;
      }
      const background = call?.name === "run_shell" && outcome?.output?.background === true && outcome.output.status === "en cours";
      if (background && runId && call.call_id) {
        interventions.push({ kind: "local", message });
      }
      const failure = toolProblem(message);
      if (failure) messages.push(problem(message, failure, outcome?.status === "failed" ? "failed" : outcome?.output?.status));
      continue;
    }
    if (message.kind === "task") {
      if (latestTasks.get(payload.run_id) !== message.id) continue;
      const completed = payload.task_status?.status === "done" && typeof payload.task_status.summary === "string" && payload.task_status.summary.trim();
      if (completed) {
        messages.push({ ...message, presentation: { kind: "completion", text: payload.task_status.summary } });
      }
      if (payload.status === "failed" && !failedReplies.has(payload.run_id)) messages.push(problem(completed ? { ...message, id: `${message.id}:problem` } : message, "Le dernier échange n’a pas abouti."));
      continue;
    }
    if (usefulCards.has(message.kind) || message.kind === "turn-heading") messages.push(message);
  }
  const firstByRun = new Map();
  for (const message of messages) {
    const runId = message.payload?.run_id;
    if (runId && message.kind !== "turn-heading" && !firstByRun.has(runId)) firstByRun.set(runId, message);
  }
  const visible = messages.filter((message) => message.kind !== "turn-heading"
    || message.payload?.turn_id && firstByRun.has(message.payload?.run_id) && !chatKinds.has(firstByRun.get(message.payload.run_id).kind));
  const headings = new Set(visible.filter((message) => message.kind === "turn-heading").map((message) => message.payload.run_id));
  return {
    messages: visible.map((message) => chatKinds.has(message.kind) && message.continuation && firstByRun.get(message.payload?.run_id) === message && !headings.has(message.payload?.run_id)
      ? { ...message, continuation: false } : message),
    interventions,
  };
}
