import { useState } from "react";

export const kind = "topic_memory_offer";

// « Retenir pour ce sujet ? » : le bot propose, Charles décide. Rien n'est écrit
// dans la mémoire du sujet avant « Retenir ».
export function Card({ message, onAction }) {
  const offer = message.payload || {};
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function act(action) {
    setBusy(true); setError("");
    try { await onAction(action, { offer_id: message.id, topic_id: offer.topic_id }); }
    catch (failure) { setError(failure.message || "Cette action n’a pas pu être terminée."); }
    finally { setBusy(false); }
  }
  const topicName = offer.topic_name || "ce sujet";
  const retained = offer.status === "accepted" && !offer.forgotten;
  const heading = offer.status === "pending" ? "Retenir pour ce sujet ?" : retained ? "Retenu pour ce sujet" : "";
  return <article className="gb-message gb-engine-card gb-memory-share gb-topic-memory" data-card-kind={kind} data-message-id={message.id} data-status={offer.status}>
    {heading && <h3>{heading}</h3>}
    {offer.status === "pending" && <>
      <p className="memory-note">Tous les fils de « {topicName} » pourront le lire, quel que soit le bot. Rien n’est retenu sans votre accord.</p>
      <blockquote className="topic-memory-quote">{offer.text}</blockquote>
      <div className="memory-actions">
        <button type="button" className="gb-retry gb-retry-primary" disabled={busy} onClick={() => act("accept")}>Retenir</button>
        <button type="button" className="gb-text-button" disabled={busy} onClick={() => act("dismiss")}>Pas maintenant</button>
      </div>
    </>}
    {retained && <>
      <blockquote className="topic-memory-quote">{offer.text}</blockquote>
      <div className="memory-actions">
        <button type="button" className="gb-text-button topic-memory-link" disabled={busy} onClick={() => act("open-memory")}>Mémoire du sujet</button>
      </div>
    </>}
    {offer.status === "accepted" && offer.forgotten && <p className="memory-note">Ce souvenir a été oublié.</p>}
    {offer.status === "dismissed" && <p className="memory-note">Pas retenu pour ce sujet.</p>}
    {error && <p className="gb-memory-error" role="alert">{error}</p>}
  </article>;
}
