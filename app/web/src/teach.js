import { sourceNotes } from "./sources.js";
// CM19: a fixed Teach instruction remains model-facing but is represented by a
// compact translated marker. Keep exact equality so ordinary user prose is never
// silently reclassified. Historical submissions use this same immutable text.
export const TEACH_PROMPT = "Apprends ceci comme skill : à partir de cette conversation, rédige une procédure réutilisable et fidèle à ce que nous avons établi. N’invente pas d’étapes et n’inclus aucun secret. Consulte tes skills pour éviter les doublons, puis utilise propose_skill pour proposer une nouvelle skill ou une nouvelle version. Présente le nom, la description, le contenu et les éventuels fichiers texte annexes dans la carte Enregistrer / Ignorer. Rien ne doit être enregistré sans mon clic.";

export function isTeachMessage(message) {
  return message?.role === "user" && message.body === TEACH_PROMPT && !message.payload?.source;
}

export function isDemonstrationMessage(message) {
  return message?.role === "user" && typeof message.payload?.demonstration_id === "string";
}

export function exportMessage(message) {
  if (isDemonstrationMessage(message)) return { ...message, body: "Démonstration terminée — apprendre la tâche", display: "demonstration" };
  return isTeachMessage(message) ? { ...message, body: "Apprendre ceci comme skill", display: "teach" } : message.payload?.sources?.length ? { ...message, body: sourceNotes(message.body, message.payload.sources) } : message;
}

export function searchableMessageText(message) {
  return isTeachMessage(message) || isDemonstrationMessage(message) ? "" : message.body || "";
}
