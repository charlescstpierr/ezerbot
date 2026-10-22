import { runView } from "./run-status.js";

export const screenTools = {
  computer_screenshot: "Capture de l’écran", computer_move: "Déplacer la souris", computer_click: "Cliquer",
  computer_scroll: "Faire défiler", computer_type: "Saisir du texte", computer_key: "Appuyer sur une touche",
};
export const screenWarnings = {
  computer_screenshot: "Ezerbot prendra une capture de l’écran principal, partagée avec le fournisseur d’IA. Rien ne sera cliqué ni saisi.",
  computer_move: "Ezerbot déplacera le pointeur sur l’écran principal, sans cliquer.",
  computer_scroll: "Ezerbot fera défiler la fenêtre sous le pointeur, sans cliquer ni saisir de texte.",
  computer_click: "Vérifiez la cible : ce clic peut acheter, envoyer un message ou supprimer des données. Ces effets peuvent être irréversibles.",
  computer_type: "Vérifiez le texte et la cible. Les champs de mot de passe sont refusés : saisissez-les vous-même. Les sauts de ligne peuvent valider ou envoyer le texte.",
  computer_key: "Cette touche peut valider un achat, envoyer un message ou supprimer des données. Vérifiez la cible avant de confirmer.",
};
export function screenActionState({ outcome, approval_status: approval, run_status: run }) {
  if (approval === "pending" && !outcome) return { hidden: true };
  if (approval === "denied") return { label: "Refusée", reason: "L’action a été refusée.", error: true };
  if (approval === "cancelled") return { label: "Arrêtée", reason: "Session arrêtée — action non exécutée.", error: true };
  if (approval === "expired") return { label: "Délai écoulé", reason: "Le délai d’approbation est écoulé — action non exécutée.", error: true };
  if (outcome?.status === "failed") {
    const reason = outcome.reason || "L’action a échoué.";
    return { label: /permission|Accessibilité|Enregistrement|Surveillance/i.test(reason) ? "Permission manquante" : /arrêt|expir|repris|pause|annul/i.test(reason) ? "Arrêtée" : "Échec", reason, error: true };
  }
  if (outcome) return { label: "Terminée" };
  if (runView(run).stopped) return { label: "Arrêtée", reason: "Action interrompue — aucun résultat enregistré.", error: true };
  // A later call in the same batch may not have reached its approval yet.
  // Absence of a result is never proof of a human decision or execution.
  if (approval !== "resolved") return { hidden: true };
  return { label: "En cours…" };
}
export function screenDecisionLabel(approval) {
  return { resolved: "Autorisée par vous", denied: "Refusée par vous", cancelled: "Approbation annulée", expired: "Approbation expirée" }[approval] || null;
}
