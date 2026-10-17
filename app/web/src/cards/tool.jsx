import { useState } from "react";

export const kind = "tool";

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

export function Card({ message }) {
  const { call, outcome } = message.payload || {};
  if (!call?.call_id || typeof call.name !== "string") {
    return <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind}>Appel d’outil indisponible.</article>;
  }
  const human = outcome?.status === "suspend" && outcome.cause?.kind === "human_input";
  const children = outcome?.status === "suspend" && outcome.cause?.kind === "children";
  const labels = { done: "Appel terminé", failed: "Échec de l’appel", suspend: "Appel suspendu" };
  const label = Object.hasOwn(labels, outcome?.status) ? labels[outcome.status] : "Résultat en attente";
  return (
    <Disclosure message={message} title={`Outil · ${call.name}`} label={label}>
      <p className="gb-engine-note">Identifiant d’appel : <code>{call.call_id}</code></p>
      <h4>Arguments</h4>
      <Data value={call.arguments} />
      {outcome?.status === "done" && <><h4>Résultat</h4><Data value={outcome.output} /></>}
      {outcome?.status === "failed" && <><h4>Raison de l’échec</h4><Data value={outcome.reason} /></>}
      {human && <p className="gb-engine-note">Cet appel a demandé une réponse humaine. Répondez dans la carte de demande de la conversation du bot principal.</p>}
      {children && <p className="gb-engine-note">Cet appel attend ses sous-tâches. Leur fin ne signifie pas que la tâche principale est terminée.</p>}
      {!outcome && <p className="gb-engine-note">Aucun résultat enregistré pour cet appel.</p>}
    </Disclosure>
  );
}
