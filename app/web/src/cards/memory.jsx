import { useEffect, useState } from "react";
import { memoryPresentation } from "../memory-presentation.js";

export const kind = "memory_auto";

export function Card({ message, onAction, botName, isGroup }) {
  const fact = message.payload || {};
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const view = memoryPresentation(fact, botName, isGroup);
  useEffect(() => { setEditing(false); setText(""); setError(""); }, [fact.fact_id, fact.active]);
  async function act(action, detail) {
    setSaving(true); setError("");
    try { if (await onAction(action, detail) === true) setEditing(false); }
    catch (failure) { setError(failure.message || "Cette action n’a pas pu être terminée."); }
    finally { setSaving(false); }
  }
  return (
    <article className={`gb-message gb-engine-card gb-memory-card memory-fact${fact.active ? "" : " is-inactive"}`} data-message-id={message.id} data-card-kind={kind}>
      <p className="memory-text">{fact.active ? <><strong>{view.label}</strong> {view.quote}</> : view.receipt}</p>
      {error && fact.active && <p className="gb-memory-error" role="alert">{error}</p>}
      {fact.active && !editing && <div className="memory-actions">
        {fact.can_undo !== false && <button type="button" className="gb-text-button" disabled={saving} onClick={() => act("undo")}>Annuler</button>}
        <button type="button" className="gb-text-button" disabled={saving} onClick={() => { setText(fact.text); setEditing(true); }}>Modifier</button>
      </div>}
      {fact.active && editing && <form onSubmit={(event) => { event.preventDefault(); act("correct", { text: text.trim() }); }}>
        <label>Modifier le fait mémorisé<textarea name="memory-text" aria-label="Modifier le fait mémorisé" maxLength={2000} value={text} onChange={(event) => setText(event.target.value)} autoFocus disabled={saving} /></label>
        <div className="memory-actions">
          <button type="submit" className="gb-retry gb-retry-primary" disabled={saving || !text.trim()}>Enregistrer</button>
          <button type="button" className="gb-text-button" disabled={saving} onClick={() => setEditing(false)}>Fermer</button>
        </div>
      </form>}
    </article>
  );
}
