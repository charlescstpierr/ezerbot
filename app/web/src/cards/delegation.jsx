import { useSyncExternalStore } from "react";

export const kind = "delegation";

const view = () => typeof window === "undefined" ? undefined : window.GrokbotDelegation;
const subscribe = (changed) => {
  window.addEventListener("grokbot:delegation-change", changed);
  return () => window.removeEventListener("grokbot:delegation-change", changed);
};
const labels = {
  queued: "En file d’attente", running: "En cours", done: "Terminée",
  failed: "En échec", refused: "Refusée", cancelled: "Annulée",
};

export function Card({ message, busy = false, onAction }) {
  useSyncExternalStore(subscribe, () => view()?.version() ?? 0, () => 0);
  const edge = view()?.edge(message.payload?.conversation_id) || message.payload || {};
  const status = edge.status?.status;
  const known = Object.hasOwn(labels, status);
  const child = view()?.botName(edge.child_bot) || edge.child_bot || "Bot inconnu";
  const hasThread = typeof edge.conversation_id === "string" && Boolean(edge.conversation_id);
  const available = hasThread && (view()?.hasConversation(edge.conversation_id) ?? true);
  const result = status === "done" ? edge.status.result : ["failed", "refused"].includes(status) ? edge.status.reason : null;
  return (
    <article className="gb-message rounded-xl border border-border bg-card p-4 text-foreground" data-message-id={message.id} data-card-kind={kind} data-delegation-status={known ? status : "unknown"}>
      <header className="gb-meta"><strong>Sous-tâche · {child}</strong></header>
      {typeof edge.objective === "string" && edge.objective && <p className="mt-2 text-sm whitespace-pre-wrap break-words">{edge.objective}</p>}
      <p className="my-2 text-sm" role="status" aria-live="polite">{known ? labels[status] : "État indisponible"}</p>
      {status === "queued" && <p className="mb-2 text-xs text-muted-foreground">La sous-tâche attend une place disponible.</p>}
      {typeof result === "string" && result && <details className="mb-3 text-sm">
        <summary>{status === "done" ? "Résultat de la sous-tâche" : "Motif"}</summary>
        {/* Server-redacted external data stays plain text, never HTML/Markdown. */}
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words font-sans text-sm">{result}</pre>
      </details>}
      {hasThread ? <>
        <button type="button" className="gb-retry" disabled={busy || !available || !onAction} onClick={() => onAction?.("delegation.open")}>Ouvrir le fil de tâche</button>
        {!available && <p className="mt-2 text-xs text-muted-foreground">Ce fil n’est plus disponible.</p>}
      </> : <p className="text-xs text-muted-foreground">Aucun fil de tâche disponible.</p>}
      {status === "running" && <p className="mt-2 text-xs text-muted-foreground">Les demandes d’approbation sont adressées à la conversation du bot principal.</p>}
    </article>
  );
}
