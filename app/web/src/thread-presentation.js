import { localFailureView } from "./local-call.js";

const chatKinds = new Set([undefined, null, "message", "submission", "reply"]);
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
const TECHNICAL = /[{}[\]<>`]|\/\w/u;
const COMPACTED = "J’ai résumé le début de notre échange pour garder le fil. Vos messages restent ici.";

export function threadActivity({ runs = [], conversationId, engineConnected = false } = {}) {
  if (!conversationId || !engineConnected) return [];
  return runs.filter((run) => run.conversation_id === conversationId && ["running", "awaiting_approval", "awaiting_human_input"].includes(run.status))
    .map(({ run_id, bot_id, status }) => ({ run_id, bot_id, status }));
}

// A waiting run speaks only when its request lives in the principal conversation.
export function visibleActivity(activity, { requestRuns }) {
  return activity.filter((run) => run.status === "running" || !requestRuns.has(run.run_id));
}

// The engine writes sentences; only a provider's appended detail can be a machine payload.
function humanSentence(text, fallback) {
  const whole = typeof text === "string" ? text.trim() : "";
  if (whole && !TECHNICAL.test(whole)) return whole;
  const head = whole.split(/ (?:Détail )?: /u, 1)[0].trim();
  if (!head || TECHNICAL.test(head)) return fallback;
  return /[.!?…]$/u.test(head) ? head : `${head}.`;
}

export function conversationReplyError(message) {
  return humanSentence(message.error, "La réponse n’a pas abouti. Réessayez.");
}

// Settings and card actions run without a model: their result is the only narrator.
const hostRun = (runId) => typeof runId === "string" && runId.startsWith("mcp-host-");

function toolProblem(payload, settledApprovals) {
  const { call, outcome, code, connector, approval_status: approvalStatus, run_id: runId } = payload;
  if (code === "uncertain") return "L’action a été interrompue et son résultat est inconnu. Vérifiez son effet avant de réessayer.";
  if (approvalProblems[approvalStatus] && settledApprovals.has(`${runId} ${call?.name}`)) return null;
  if (outcome?.status !== "failed" && !outcome?.output?.isError
    && !["échec", "annulé", "délai dépassé", "arrêtée"].includes(outcome?.output?.status)) return null;
  const decided = localFailureView(outcome?.reason, approvalStatus);
  if (decided && decided.label !== "Erreur") return decided.message;
  if (["annulé", "arrêtée"].includes(outcome?.output?.status)) return "La tâche a été arrêtée.";
  if (outcome?.output?.status === "délai dépassé") return "La commande a pris trop de temps et s’est arrêtée.";
  if (call?.name === "generate_image" && outcome?.reason === "La génération de cette image a été refusée pour des raisons de sécurité.") return outcome.reason;
  const label = failureLabels[call?.name] || "L’action n’a pas abouti.";
  if (hostRun(runId)) return humanSentence(outcome?.reason, label);
  return connector?.recoverable === true ? `Le connecteur ${connector.server} ne répond plus. Ouvrez Paramètres › Connecteurs pour le rétablir.` : label;
}

function toolSentence({ call, outcome, run_id: runId }, spoken) {
  if (outcome?.status !== "done") return null;
  if (hostRun(runId)) return outcome.output?.message;
  const question = call?.name === "attendre_reponse" ? call.arguments?.question : null;
  return question && !spoken.get(runId)?.includes(question) ? question : null;
}

function approvalProblem({ status, review }) {
  const refusal = review?.answer?.decision === "deny" && review.answer.reason;
  return status?.status === "denied" && refusal ? `L’action a été refusée automatiquement : ${refusal}` : approvalProblems[status?.status];
}

function problem(message, text, localState) {
  return { ...message, presentation: { kind: "problem", text, localState } };
}

function notice(message, text) {
  return { ...message, presentation: { kind: "notice", text } };
}

export function presentThread(orderedMessages) {
  const messages = [], interventions = [];
  const latestTasks = new Map(), spoken = new Map(), settledApprovals = new Set(), requestRuns = new Set();
  for (const message of orderedMessages) {
    const runId = message.payload?.run_id;
    if (message.kind === "task" && runId
      && (!latestTasks.has(runId) || message.payload.event_kind === "task_changed")) latestTasks.set(runId, message.id);
    if (chatKinds.has(message.kind) && runId) spoken.set(runId, `${spoken.get(runId) || ""}${message.body || ""}`);
    if (["approval", "form", "secret"].includes(message.kind)) requestRuns.add(runId);
    if (message.kind === "approval" && approvalProblems[message.payload?.status?.status]) settledApprovals.add(`${runId} ${message.payload.kind?.tool}`);
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
      else if (approvalProblems[payload.status?.status]) messages.push(problem(message, approvalProblem(payload)));
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
      const failure = toolProblem(payload, settledApprovals);
      const sentence = !failure && toolSentence(payload, spoken);
      if (failure) messages.push(problem(message, failure, outcome?.status === "failed" ? "failed" : outcome?.output?.status));
      else if (sentence) messages.push(notice(message, sentence));
      continue;
    }
    if (message.kind === "task") {
      if (payload.event_kind === "turn_ended" && payload.turn_end === "budget_exhausted" && payload.task_status?.status !== "done") messages.push(problem(message, "Le budget de ce tour est épuisé : la tâche n’est pas terminée."));
      if (latestTasks.get(payload.run_id) !== message.id) continue;
      const completed = payload.task_status?.status === "done" && typeof payload.task_status.summary === "string" && payload.task_status.summary.trim();
      if (completed) {
        messages.push({ ...message, presentation: { kind: "completion", text: payload.task_status.summary } });
      }
      if (payload.status === "failed" && !failedReplies.has(payload.run_id)) messages.push(problem(completed ? { ...message, id: `${message.id}:problem` } : message, "Le dernier échange n’a pas abouti."));
      continue;
    }
    if (message.kind === "reset") messages.push(notice(message, message.body));
    else if (message.kind === "summary") messages.push(notice(message, COMPACTED));
    else messages.push(message);
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
    requestRuns,
  };
}
