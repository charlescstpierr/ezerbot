// CM19: a fixed Teach instruction remains model-facing but is represented by a
// compact translated marker. Keep exact equality so ordinary user prose is never
// silently reclassified. Historical submissions use this same immutable text.
export const TEACH_PROMPT = "Apprends ceci comme skill : à partir de cette conversation, rédige une procédure réutilisable et fidèle à ce que nous avons établi. N’invente pas d’étapes et n’inclus aucun secret. Consulte tes skills pour éviter les doublons, puis utilise propose_skill pour proposer une nouvelle skill ou une nouvelle version. Présente le nom, la description, le contenu et les éventuels fichiers texte annexes dans la carte Enregistrer / Ignorer. Rien ne doit être enregistré sans mon clic.";

export function isTeachMessage(message) {
  return message?.role === "user" && message.body === TEACH_PROMPT && !message.payload?.source;
}

export function exportMessage(message) {
  return isTeachMessage(message) ? { ...message, body: "Apprendre ceci comme skill", display: "teach" } : message;
}

export function searchableMessageText(message) {
  return isTeachMessage(message) ? "" : message.body || "";
}
