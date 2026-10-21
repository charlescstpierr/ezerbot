import { useEffect, useState } from "react";
import { reconcileShareSelection } from "../memory-share-selection.js";

export const kind = "memory_share";
export function Card({ message, onAction }) {
  const facts = message.payload?.facts || [];
  const signature = JSON.stringify(facts.map((f) => [f.fact_id, f.shared, f.sensitive]));
  const [selection, setSelection] = useState(() => reconcileShareSelection(null, message));
  const [busy, setBusy] = useState(false);
  const [receipt, setReceipt] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    setSelection((previous) => reconcileShareSelection(previous, message));
  }, [message.id, signature]);
  const selected = selection.selected;
  async function share() {
    setBusy(true); setError("");
    try {
      let result;
      await onAction("share", { fact_ids: [...selected], onConfirmed: (report) => { result = report; } });
      if (!result || typeof result.shared_count !== "number") throw new Error("Partage non confirmé. Actualisez avant de réessayer.");
      setReceipt(`${result.shared_count} souvenir${result.shared_count > 1 ? "s" : ""} partagé${result.shared_count > 1 ? "s" : ""}. Les autres restent privés.`);
      setSelection((previous) => ({ ...previous, selected: new Set() }));
    } catch (failure) { setError(failure.message || "Partage non confirmé."); }
    finally { setBusy(false); }
  }
  return <article className="gb-message gb-engine-card gb-memory-share" data-card-kind={kind} data-message-id={message.id}>
    <h3>Partager à l’équipe ?</h3>
    <p className="memory-note">Équipe locale : tous vos bots. Rien n’est partagé sans votre accord. Les faits personnels et à vérifier ne sont pas cochés par défaut.</p>
    {facts.length === 0 && <p className="memory-note">Ces souvenirs ont été oubliés ou remplacés.</p>}
    <fieldset disabled={busy} className="memory-controls">
      {facts.map((fact) => <label className="memory-share-choice" key={fact.fact_id}>
        <input type="checkbox" checked={fact.shared || selected.has(fact.fact_id)} disabled={fact.sensitive || fact.shared} onChange={(e) => setSelection((old) => { const next = new Set(old.selected); if (e.target.checked) next.add(fact.fact_id); else next.delete(fact.fact_id); return { ...old, selected: next }; })} />
        <span>{fact.text}<small>{fact.shared ? "Déjà en équipe" : fact.sensitive ? "Sensible · reste privé" : fact.class === "work" ? "Travail" : fact.class === "personal" ? "Personnel · privé par défaut" : "À vérifier · privé par défaut"}</small></span>
      </label>)}
      <button type="button" className="gb-retry gb-retry-primary" disabled={busy || !selected.size} onClick={share}>{busy ? "Partage…" : `Partager la sélection (${selected.size})`}</button>
    </fieldset>
    {receipt && <p className="memory-result" role="status">{receipt}</p>}
    {error && <p className="gb-memory-error" role="alert">{error}</p>}
  </article>;
}
