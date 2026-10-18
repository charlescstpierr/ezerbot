import { useState } from "react";

export const kind = "memory_auto";

export function Card({ message, onAction }) {
  const fact = message.payload || {};
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  async function act(action, detail) {
    setSaving(true);
    try { if (await onAction(action, detail)) setEditing(false); }
    finally { setSaving(false); }
  }
  return (
    <article className="gb-message gb-engine-card gb-memory-card memory-fact" data-message-id={message.id} data-card-kind={kind}>
      <p className="memory-text">{fact.active ? <><strong>Mémorisé :</strong> {fact.text}</> : fact.cancelled ? "Mémorisation annulée." : fact.updated ? "Ce fait a été mis à jour." : "Ce fait a été oublié."}</p>
      {fact.active && !editing && <div className="memory-actions">
        {fact.can_undo !== false && <button type="button" className="gb-text-button" disabled={saving} onClick={() => act("undo")}>Annuler</button>}
        <button type="button" className="gb-text-button" disabled={saving} onClick={() => { setText(fact.text); setEditing(true); }}>Modifier</button>
      </div>}
      {fact.active && editing && <form onSubmit={(event) => { event.preventDefault(); act("correct", { text: text.trim() }); }}>
        <label>Modifier le fait mémorisé<textarea name="memory-text" aria-label="Modifier le fait mémorisé" maxLength={2000} value={text} onChange={(event) => setText(event.target.value)} autoFocus disabled={saving} /></label>
        <div className="memory-actions">
          <button type="submit" className="gb-text-button" disabled={saving || !text.trim()}>Enregistrer</button>
          <button type="button" className="gb-text-button" disabled={saving} onClick={() => setEditing(false)}>Fermer</button>
        </div>
      </form>}
    </article>
  );
}
