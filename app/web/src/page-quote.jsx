// Puce « Page « Titre » — « passage » » d'un message envoyé avec une ancre ; un clic ouvre la Page.
// L'envoi en cours n'a pas encore le titre (le moteur le renvoie avec le fil) : la puce garde alors le passage seul.
export function PageQuote({ anchor, onOpenPage }) {
  const label = `Page${anchor.title ? ` « ${anchor.title} »` : ""} — « ${anchor.passage.replace(/\s+/g, " ")} »`;
  return (
    <button type="button" className="gb-quote-chip" title={label} aria-label={`Ouvrir ${label}`} onClick={() => onOpenPage?.(anchor.page_id)}>
      <span className="gb-quote-text">{label}</span>
    </button>
  );
}
