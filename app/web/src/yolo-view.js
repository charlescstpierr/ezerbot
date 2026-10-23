// Présentation seule, jamais une décision d'autorisation : le serveur a déjà
// admis l'action et l'a inscrite au journal d'activité.
export const AUTOMATIC_LABEL = "Fait automatiquement";

export const isAutomatic = (authority) => authority?.kind === "yolo";

const text = (value) => (typeof value === "string" ? value : "");
const flat = (value, max = 160) => {
  const line = text(value).split(/\s+/).filter(Boolean).join(" ");
  return line.length > max ? `${line.slice(0, max).trimEnd()}…` : line;
};
const bytes = (value) => (typeof value === "number" ? `${new Intl.NumberFormat("fr-CA").format(value)} octet${value > 1 ? "s" : ""}` : "");

// Un verbe par état : en cours, fait, ou non fait. Le sujet vient des arguments
// de l'appel, tels que le bot les a écrits.
export function automaticLine({ call, outcome, connector } = {}) {
  const args = call?.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments) ? call.arguments : {};
  const state = !outcome || outcome.status === "suspend" ? 0 : outcome.status === "failed" ? 2 : 1;
  const output = outcome?.output && typeof outcome.output === "object" ? outcome.output : {};
  const say = (verbs, subject, extra = "") => `${verbs[state]} : ${subject}${extra}`;
  switch (call?.name) {
    case "run_shell": {
      const background = state === 1 && args.background === true && output.status === "en cours";
      const verbs = ["Commande en cours", background ? "Commande lancée en tâche de fond" : "Commande exécutée", "Commande non exécutée"];
      const code = state === 1 && !background && output.exit_code != null ? ` · code de sortie ${output.exit_code}` : "";
      return say(verbs, flat(args.command), code);
    }
    case "read_file": return say(["Lecture en cours", "Fichier lu", "Lecture refusée ou impossible"], text(args.path));
    case "list_directory": return say(["Lecture du dossier en cours", "Dossier listé", "Dossier non listé"], text(args.path));
    case "write_file": {
      const verb = args.expected_sha256 ? "remplacé" : "créé";
      const size = state === 1 && typeof output.bytes === "number" ? ` · ${bytes(output.bytes)}` : "";
      return say(["Écriture en cours", `Fichier ${verb}`, "Fichier non écrit"], text(args.path), size);
    }
    case "delegate": return say(["Délégation en cours", "Tâche déléguée", "Tâche non déléguée"], flat(args.objective));
    default:
      if (connector?.server) return say(["Appel en cours", "Appel effectué", "Appel non effectué"], `${connector.server} · ${connector.tool || call?.name || ""}`);
      if (typeof call?.name === "string" && call.name.startsWith("computer_")) return say(["Geste en cours", "Geste effectué", "Geste non effectué"], call.name.replace("computer_", "écran du Mac · "));
      return say(["Appel en cours", "Appel effectué", "Appel non effectué"], text(call?.name) || "outil");
  }
}

export function sourceLabel(authority) {
  return authority?.source === "bot" ? "Réglage de ce bot" : "Réglage global";
}
