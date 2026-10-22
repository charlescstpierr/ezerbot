import { useState } from "react";
import { HumanInputCard, inputClass } from "./form.jsx";

export const MAX_DRAFT_CHARACTERS = 65536;

// Rendered only by the existing approval card: the draft is the card of a closed list of
// publications, never a second permission system. It has no standing-rule button.
export function DraftCard({ message, input, disabled, expired }) {
  const { request, pending, status, act } = input;
  const draft = request.arguments?.brouillon || {};
  const [edited, setEdited] = useState(null);
  const text = edited ?? draft.texte ?? "";
  const changed = edited !== null && edited !== draft.texte;
  const blank = !text.trim();
  const tooLong = text.length > MAX_DRAFT_CHARACTERS;
  const result = request.publication_result;
  const denied = status === "denied";
  const failed = result?.status === "failed" && !denied;
  const published = result?.status === "done";
  const statusText = denied ? "Annulé · rien n’a été publié" : published ? "Publié" : failed ? "Non publié"
    : status === "resolved" ? "Envoi validé" : status === "pending" ? "En attente de vous" : "Rien n’a été publié";
  const send = () => act("forms.decide", { decision: "allow_once", ...(changed ? { text } : {}) });
  return <HumanInputCard message={message} title={`Brouillon · ${draft.action || "Publication"}`} input={input} statusText={statusText} showDelivery={false}>
    <div className="gb-draft space-y-3" data-draft-service={draft.service}>
      <dl className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3 text-sm" aria-label="Cible du brouillon">
        <div className="min-w-0"><dt className="text-xs text-muted-foreground">Dépôt</dt><dd className="mt-1 ml-0 break-words">{draft.depot}</dd></div>
        <div className="min-w-0"><dt className="text-xs text-muted-foreground">Issue</dt><dd className="mt-1 ml-0">#{draft.numero}</dd></div>
        <div className="min-w-0"><dt className="text-xs text-muted-foreground">Publié sous le compte</dt><dd className="mt-1 ml-0 break-words">{draft.login}</dd></div>
      </dl>
      {pending
        ? <label className="block space-y-1 text-sm"><span>Texte exact du commentaire (modifiable)</span>
          <textarea className={`${inputClass} min-h-24`} value={text} disabled={disabled} aria-invalid={blank || tooLong} onChange={(event) => setEdited(event.target.value)} /></label>
        : <div><h4 className="mb-1 text-xs font-semibold">Texte du commentaire</h4><pre className="gb-skill-text">{draft.texte}</pre></div>}
      {pending && <>
        <p className="text-xs text-muted-foreground">Rien n’est publié avant votre clic. Envoyer publie ce texte une seule fois, sous le compte affiché.</p>
        {tooLong && <p className="text-sm" role="alert">Le texte dépasse {MAX_DRAFT_CHARACTERS} caractères.</p>}
        <fieldset className="flex flex-wrap gap-2 border-0 p-0 m-0" disabled={disabled}>
          <button type="button" className="gb-retry gb-retry-primary" disabled={blank || tooLong} onClick={send}>Envoyer</button>
          <button type="button" className="gb-retry" onClick={() => act("forms.decide", { decision: "deny" })}>Annuler</button>
        </fieldset></>}
      {input.busy && <p className="text-xs text-muted-foreground" role="status">Décision en cours…</p>}
      {pending && expired && <p className="text-sm">Ce brouillon a expiré. Demandez au bot de le préparer à nouveau.</p>}
      {published && <p className="gb-status-ok text-sm" role="status">Commentaire publié sous {result.output?.compte} : <a href={result.output?.url} target="_blank" rel="noreferrer">{result.output?.url}</a></p>}
      {failed && <p className="gb-status-alert text-sm" role="alert">{result.reason}</p>}
      <details className="text-xs text-muted-foreground"><summary>Empreinte</summary><p className="break-all">{draft.empreinte}</p></details>
    </div>
  </HumanInputCard>;
}
