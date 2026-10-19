import { localFailureView } from "./local-call.js";

export const documentActionLabel = (action) => ({ open: "Ouvrir le document", reveal: "Afficher dans le Finder", save: "Enregistrer une copie" })[action] || "Action sur le document";
export const documentApprovalEffect = (action) => ({
  save: "Un nouveau fichier sera écrit sur le Mac. Aucun remplacement.",
  reveal: "Le Finder affichera le fichier, sans l’ouvrir.",
  open: "Le document sera ouvert par l’application par défaut de macOS, en dehors d’Ezerbot.",
})[action] || "Vérifiez les détails de l’action avant de décider.";

export function documentToolView(payload, run) {
  const { call, outcome, approval_status, run_status } = payload;
  const status = run?.status && run.status !== "ended" ? run.status : run_status || run?.status;
  const failure = outcome?.status === "failed" ? localFailureView(outcome.reason, approval_status, status) : null;
  const creating = call.name === "create_document";
  const title = creating ? (failure ? "Échec de la création du document" : "Création du document") : documentActionLabel(call.arguments?.action);
  const label = failure?.label || (outcome?.status === "done" ? "Terminé" : status === "awaiting_approval" ? "Accord requis" : status === "running" ? "En cours…" : "En attente");
  // Keep model-oriented syntax out of the main error banner, but available for diagnosis.
  let summary = failure?.message;
  if (creating && failure) {
    if (/Pour CSV ou Excel|Le tableau doit/.test(summary)) summary = "Le tableau fourni par le modèle est mal formé.";
    if (/Mermaid|Pour SVG|Diagramme invalide/.test(summary)) summary = "Le diagramme fourni par le modèle n’a pas pu être rendu.";
  }
  return { title, label, failure, summary, tone: failure?.tone || (outcome?.status === "done" ? "success" : "neutral"), state: failure ? "failed" : !outcome && status === "running" ? "running" : "" };
}
