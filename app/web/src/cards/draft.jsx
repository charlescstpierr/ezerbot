import { useState } from "react";
import { HumanInputCard, inputClass } from "./form.jsx";

export const MAX_DRAFT_CHARACTERS = 65536;
export const MAX_ISSUE_TITLE_CHARACTERS = 256;
export const MAX_EMAIL_SUBJECT_CHARACTERS = 255;
export const MAX_SLACK_CHARACTERS = 40000;

// How a send that did not complete is named on the card: none of them is a mail that left.
const EMAIL_FAILURES = { refused: "Refusé", rate_limited: "Trop de messages", unconfirmed: "Envoi non confirmé · accusé de réception manquant" };
const SLACK_FAILURES = { refused: "Refusé", rate_limited: "Trop de messages", unconfirmed: "Envoi non confirmé" };

// Rendered only by the existing approval card: the draft is the card of a closed list of
// publications, never a second permission system. It has no standing-rule button.
export function DraftCard({ message, input, disabled, expired }) {
  const { request, pending, status, act } = input;
  const draft = request.arguments?.brouillon || {};
  const email = draft.objet !== undefined;
  const issue = draft.titre !== undefined;
  const slack = draft.espace !== undefined;
  const titled = issue || email;
  const [edited, setEdited] = useState(null);
  const [editedTitle, setEditedTitle] = useState(null);
  const text = edited ?? draft.texte ?? "";
  const title = editedTitle ?? draft.titre ?? draft.objet ?? "";
  const changed = edited !== null && edited !== draft.texte;
  const titleChanged = editedTitle !== null && editedTitle !== (draft.titre ?? draft.objet);
  // An issue or an email may have no body; a comment is its text.
  const blank = titled ? !title.trim() : !text.trim();
  const limit = slack ? MAX_SLACK_CHARACTERS : MAX_DRAFT_CHARACTERS;
  const tooLong = text.length > limit;
  const maxTitle = email ? MAX_EMAIL_SUBJECT_CHARACTERS : MAX_ISSUE_TITLE_CHARACTERS;
  const titleTooLong = title.length > maxTitle;
  const result = request.publication_result;
  const denied = status === "denied";
  const failed = result?.status === "failed" && !denied;
  const published = result?.status === "done";
  const nothing = email ? "envoyé" : "publié";
  const failure = slack ? SLACK_FAILURES[draft.etat] : undefined;
  const statusText = denied ? `Annulé · rien n’a été ${nothing}` : published ? (email ? "Courriel envoyé" : issue ? "Issue créée" : "Publié")
    : failed ? (email ? EMAIL_FAILURES[draft.etat] || "Non envoyé" : failure || "Non publié")
    : status === "resolved" ? "Envoi validé" : status === "pending" ? "En attente de vous" : `Rien n’a été ${nothing}`;
  const send = () => act("forms.decide", { decision: "allow_once", ...(changed ? { text } : {}), ...(titleChanged ? { title } : {}) });
  return <HumanInputCard message={message} title={`Brouillon · ${draft.action || "Publication"}`} input={input} statusText={statusText} showDelivery={false}>
    <div className="gb-draft space-y-3" data-draft-service={draft.service}>
      {slack
        ? <dl className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3 text-sm" aria-label="Cible du brouillon">
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Espace</dt><dd className="mt-1 ml-0 break-words">{draft.espace}</dd></div>
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Cible</dt><dd className="mt-1 ml-0 break-words">{draft.cible}</dd></div>
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Fil</dt><dd className="mt-1 ml-0 break-words">{draft.fil || "Aucun · dans le canal"}</dd></div>
        </dl>
        : email
        ? <dl className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3 text-sm" aria-label="Destinataires du brouillon">
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">De</dt><dd className="mt-1 ml-0 break-words">{draft.de}</dd></div>
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">À</dt><dd className="mt-1 ml-0 break-words">{(draft.a || []).join(", ")}</dd></div>
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Cc</dt><dd className="mt-1 ml-0 break-words">{(draft.cc || []).length ? draft.cc.join(", ") : "—"}</dd></div>
        </dl>
        : <dl className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3 text-sm" aria-label="Cible du brouillon">
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Dépôt</dt><dd className="mt-1 ml-0 break-words">{draft.depot}</dd></div>
          {!issue && <div className="min-w-0"><dt className="text-xs text-muted-foreground">Issue</dt><dd className="mt-1 ml-0">#{draft.numero}</dd></div>}
          <div className="min-w-0"><dt className="text-xs text-muted-foreground">Publié sous le compte</dt><dd className="mt-1 ml-0 break-words">{draft.login}</dd></div>
        </dl>}
      {titled && (pending
        ? <label className="block space-y-1 text-sm"><span>{email ? "Objet (modifiable)" : "Titre de l’issue (modifiable)"}</span>
          <input type="text" className={inputClass} value={title} disabled={disabled} aria-invalid={blank || titleTooLong} onChange={(event) => setEditedTitle(event.target.value)} /></label>
        : <div><h4 className="mb-1 text-xs font-semibold">{email ? "Objet" : "Titre de l’issue"}</h4><p className="text-sm break-words">{draft.titre ?? draft.objet}</p></div>)}
      {pending
        ? <label className="block space-y-1 text-sm"><span>{email ? "Corps du courriel (modifiable)" : issue ? "Corps de l’issue (modifiable)" : slack ? "Texte exact du message (modifiable)" : "Texte exact du commentaire (modifiable)"}</span>
          <textarea className={`${inputClass} min-h-24`} value={text} disabled={disabled} aria-invalid={(!titled && blank) || tooLong} onChange={(event) => setEdited(event.target.value)} /></label>
        : <div><h4 className="mb-1 text-xs font-semibold">{email ? "Corps du courriel" : issue ? "Corps de l’issue" : slack ? "Texte du message" : "Texte du commentaire"}</h4><pre className="gb-skill-text">{draft.texte}</pre></div>}
      {pending && <>
        <p className="text-xs text-muted-foreground">Rien n’est {nothing} avant votre clic. Envoyer {email ? "envoie ce courriel" : issue ? "crée cette issue" : "publie ce texte"} une seule fois, sous {slack ? "votre nom, dans l’espace affiché" : "le compte affiché"}.</p>
        {titleTooLong && <p className="text-sm" role="alert">{email ? "L’objet" : "Le titre"} dépasse {maxTitle} caractères.</p>}
        {tooLong && <p className="text-sm" role="alert">Le texte dépasse {limit} caractères.</p>}
        <fieldset className="flex flex-wrap gap-2 border-0 p-0 m-0" disabled={disabled}>
          <button type="button" className="gb-retry gb-retry-primary" disabled={blank || tooLong || titleTooLong} onClick={send}>Envoyer</button>
          <button type="button" className="gb-retry" onClick={() => act("forms.decide", { decision: "deny" })}>Annuler</button>
        </fieldset></>}
      {input.busy && <p className="text-xs text-muted-foreground" role="status">Décision en cours…</p>}
      {pending && expired && <p className="text-sm">Ce brouillon a expiré. Demandez au bot de le préparer à nouveau.</p>}
      {published && email && <p className="gb-status-ok text-sm" role="status">Courriel envoyé sous {result.output?.compte} à {(result.output?.a || []).join(", ")}.</p>}
      {published && issue && <p className="gb-status-ok text-sm" role="status">Issue {result.output?.numero ? `#${result.output.numero} ` : ""}créée sous {result.output?.compte} : <a href={result.output?.url} target="_blank" rel="noreferrer">{result.output?.url}</a></p>}
      {published && slack && <p className="gb-status-ok text-sm" role="status">Message publié dans {draft.cible} de l’espace {result.output?.compte}.</p>}
      {published && !issue && !email && !slack && <p className="gb-status-ok text-sm" role="status">Commentaire publié sous {result.output?.compte} : <a href={result.output?.url} target="_blank" rel="noreferrer">{result.output?.url}</a></p>}
      {failed && <p className="gb-status-alert text-sm" role="alert">{failure && <strong>{failure} · </strong>}{result.reason}</p>}
      <details className="text-xs text-muted-foreground"><summary>Empreinte</summary><p className="break-all">{draft.empreinte}</p></details>
    </div>
  </HumanInputCard>;
}
