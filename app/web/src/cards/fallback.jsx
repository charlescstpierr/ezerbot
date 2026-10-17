// Unknown entries remain visible without dumping payloads (which may contain private metadata).
export function Card({ message }) {
  return (
    <article className="gb-message gb-pending gb-problem" data-message-id={message.id} data-card-kind={message.kind}>
      <div className="gb-pending-copy">
        <strong>Carte non prise en charge : {message.kind || "inconnue"}</strong>
        <p>{message.body || "Cette entrée sera disponible avec une prochaine version de l’application."}</p>
      </div>
    </article>
  );
}
