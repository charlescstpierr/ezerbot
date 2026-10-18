import { useState, useSyncExternalStore } from "react";

export const kind = "tool";

const engine = () => typeof window === "undefined" ? undefined : window.GrokbotEngine;
const subscribe = (changed) => {
  window.addEventListener("grokbot:engine-change", changed);
  return () => window.removeEventListener("grokbot:engine-change", changed);
};
export function useEngine() {
  useSyncExternalStore(subscribe, () => engine()?.version() ?? 0, () => 0);
  return engine();
}

// Shared presentation only. Never dump whole engine events: they can contain
// opaque provider reasoning. All external text stays text, never HTML/Markdown.
export function Data({ value }) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return <pre className="gb-engine-data">{text ?? "Non disponible"}</pre>;
}

export function Disclosure({ message, title, label, children }) {
  const [open, setOpen] = useState(false);
  return (
    <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={message.kind}>
      <details onToggle={(event) => setOpen(event.currentTarget.open)}>
        <summary><strong>{title}</strong>{label && <span className="gb-engine-label">{label}</span>}</summary>
        {open && <div className="gb-engine-details">{children}</div>}
      </details>
    </article>
  );
}

function suspension(message, view) {
  const { run_id: runId, outcome } = message.payload;
  const run = view?.run(runId);
  const human = outcome.cause?.kind === "human_input";
  const request = human ? view?.request(outcome.cause.id) : undefined;
  const current = !view?.latestSuspension(runId) || view.latestSuspension(runId) === message.id;
  if (request?.run_id === runId) {
    const labels = { denied: "Demande refusée", expired: "Demande expirée", cancelled: "Demande annulée" };
    if (Object.hasOwn(labels, request.status)) return { label: labels[request.status] };
    if (request.wake === "delivered") return { label: "Réponse reçue · appel repris" };
    if (request.wake === "failed") return { label: "Reprise non confirmée" };
    if (request.status === "resolved") return { label: "Réponse enregistrée · reprise en attente" };
  }
  if (["cancelled", "failed"].includes(run?.status)) return { label: "Appel interrompu", note: "Le run est terminé. Aucune réponse n’est demandée pour cet appel." };
  const waiting = current && (human
    ? ["awaiting_human_input", "awaiting_approval"].includes(run?.status)
    : outcome.cause?.kind === "children" && run?.status === "awaiting_children");
  if (waiting) return human
    ? { label: "En attente de vous", note: "Répondez dans la carte de demande de la conversation du bot principal." }
    : { label: "En attente de sous-tâches", note: "Cet appel attend ses sous-tâches. Leur fin ne termine pas la tâche principale." };
  // A missing active run proves neither delivery nor child success. Without a
  // receipt or a precise lifecycle snapshot, say only that the wait has ended.
  if (run?.status === "ended") return { label: "Attente terminée", note: "Le run n’est plus actif. Aucune nouvelle réponse n’est demandée par cet appel." };
  if (!current || ["running", "interrupted", "finished"].includes(run?.status)) return { label: human ? "Appel repris" : "Attente de sous-tâches terminée" };
  return { label: "État de l’appel indisponible" };
}

export function Card({ message }) {
  const view = useEngine();
  const { run_id: runId, call, outcome, connector } = message.payload || {};
  // These internal tools are represented by the single task-status card.
  if (["begin_task", "complete_task"].includes(call?.name)) return null;
  if (!call?.call_id || typeof call.name !== "string") {
    return <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind}>Appel d’outil indisponible.</article>;
  }
  const run = view?.run(runId);
  const ended = ["ended", "finished", "cancelled", "failed"].includes(run?.status);
  let label = outcome?.status === "done" ? (outcome.output?.isError ? "Erreur de l’outil" : "Appel terminé") : outcome?.status === "failed" ? "Échec de l’appel" : "Résultat en attente";
  let note;
  if (outcome?.status === "suspend") ({ label, note } = suspension(message, view));
  if (!outcome) {
    label = ended ? "Non exécuté" : run?.status === "awaiting_approval" ? "En attente d’approbation" : "Résultat en attente";
    note = ended ? "Aucun résultat enregistré ; le run n’est plus actif."
      : run?.status === "awaiting_approval" ? "L’approbation se décide dans la conversation du bot principal."
        : "Aucun résultat enregistré pour cet appel.";
  }
  return (
    <Disclosure message={message} title={connector ? `Connecteur · ${connector.server} · ${connector.tool}` : `Outil · ${call.name}`} label={label}>
      <p className="gb-engine-note">Identifiant d’appel : <code>{call.call_id}</code></p>
      <h4>Arguments</h4>
      <Data value={call.arguments} />
      {outcome?.status === "done" && <><h4>Résultat</h4><Data value={outcome.output} /></>}
      {outcome?.status === "failed" && <><h4>Raison de l’échec</h4><Data value={outcome.reason} /></>}
      {note && <p className="gb-engine-note">{note}</p>}
    </Disclosure>
  );
}
