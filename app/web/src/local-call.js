export const localToolNames = { run_shell: "Terminal sur votre Mac", read_file: "Lecture de fichier", list_directory: "Contenu du dossier", write_file: "Écriture de fichier" };
export const localToolLabel = (name) => localToolNames[name] || name;
if (typeof window !== "undefined") window.GrokbotLocalUi = { toolLabel: localToolLabel };

export function localStatusLabel(status) {
  return { "terminé": "Terminé", "échec": "Code de sortie non nul", "annulé": "Annulée", "délai dépassé": "Délai dépassé" }[status] || "Statut indisponible";
}
export function localLimits(args = {}) {
  const number = new Intl.NumberFormat("fr-CA", { maximumFractionDigits: 3 });
  return `Délai : ${number.format((args.timeout_ms ?? 30000) / 1000)} s · Sortie cumulée : ${number.format((args.output_limit ?? 131072) / 1024)} Kio maximum`;
}
export function localFailureView(reason, approvalStatus, runStatus) {
  if (typeof reason !== "string") return null;
  if (reason.startsWith("Résultat inconnu")) return { label: "Résultat inconnu", tone: "warning", message: reason };
  if (reason.startsWith("not executed:") || reason.startsWith("Action non exécutée")) return { label: runStatus === "cancelled" ? "Annulée" : "Non exécutée", tone: "warning", message: reason.startsWith("Action non exécutée") ? reason : "Action non exécutée : la réponse a été arrêtée ou interrompue." };
  if (reason.startsWith("Cet outil est désactivé")) return { label: "Désactivé", tone: "warning", message: reason };
  if (approvalStatus === "expired") return { label: "Approbation expirée", tone: "warning", message: "Le délai d’approbation est écoulé. L’action n’a pas été exécutée." };
  if (approvalStatus === "cancelled" || runStatus === "cancelled") return { label: "Annulée", tone: "warning", message: "L’action a été annulée avant son exécution." };
  const normalized = reason.replace(/'/g, "’");
  const humanDenied = normalized.startsWith("La demande a été refusée") || normalized.includes("L’approbation a été refusée")
    || /^L’action a été refusée\.?$/.test(normalized) || reason.startsWith("Approval was denied");
  const ruleDenied = reason.includes("bloquée par une autorisation permanente") || reason.startsWith("Une autorisation permanente refuse cette action");
  if (approvalStatus === "denied" || humanDenied || ruleDenied) {
    const message = ruleDenied ? "L’action a été refusée par une règle « Toujours refuser » (Paramètres › Autorisations permanentes). Elle n’a pas été exécutée."
      : `L’action a été refusée et n’a pas été exécutée.${humanDenied ? "" : ` ${reason}`}`;
    return { label: "Refusée", tone: "warning", message };
  }
  return { label: "Erreur", tone: "danger", message: reason };
}

// Presentation only, never an authorization decision. Invalid model JSON must
// remain printable without passing objects as React text children.
export function localCallView(call) {
  const args = call.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments) ? call.arguments : {};
  const target = call.name === "run_shell" ? args.command : args.path;
  const preview = typeof target === "string" ? target : JSON.stringify(call.arguments, null, 2) ?? "Arguments indisponibles.";
  const shellArguments = typeof args.command === "string" && typeof args.cwd === "string"
    && (args.timeout_ms == null || typeof args.timeout_ms === "number")
    && (args.output_limit == null || typeof args.output_limit === "number");
  return { args, preview, shellArguments };
}
