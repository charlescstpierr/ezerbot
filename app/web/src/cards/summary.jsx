import { Data, Disclosure } from "./tool.jsx";

export const kind = "summary";

export function Card({ message }) {
  const summary = message.payload || {};
  // RC06 (REA server-context-residual.md) exposes BOTH inclusive bounds and
  // the previous-summary chain. Compaction never deletes the visible transcript.
  return (
    <Disclosure message={message} title="Contexte résumé" label="Historique conservé">
      <p className="gb-engine-note">Ce résumé allège le contexte envoyé au modèle. Les messages originaux restent dans la conversation.</p>
      <Data value={typeof summary.text === "string" ? summary.text : message.body || "Résumé indisponible."} />
      <h4>Détails techniques</h4>
      <dl className="gb-engine-bounds">
        <dt>Résumé jusqu’à l’entrée (incluse)</dt><dd>{summary.truncation_last_entry_inclusive || "Non disponible"}</dd>
        <dt>Reprise à l’entrée (incluse)</dt><dd>{summary.resume_from_entry_inclusive || "Non disponible"}</dd>
        {summary.previous_summary_id && <><dt>Résumé précédent</dt><dd>{summary.previous_summary_id}</dd></>}
        <dt>Résultats d’outils</dt><dd>{summary.includes_tool_results === true ? "Inclus" : summary.includes_tool_results === false ? "Non inclus" : "Non précisé"}</dd>
        {summary.strategy && <><dt>Stratégie</dt><dd>{summary.strategy}</dd></>}
      </dl>
    </Disclosure>
  );
}
