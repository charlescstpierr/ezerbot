import { HumanInputCard } from "./form.jsx";
import { skillDiff } from "../skill-proposal.js";

// Rendered only by the existing approval card; not a separate permission system.
export function SkillProposal({ message, input, disabled, expired }) {
  const { request, pending, status, act } = input;
  const display = request.arguments || {};
  const content = display.proposal;
  const before = display.before;
  const diff = content && before ? skillDiff(before.content, content) : [];
  const largeDiff = diff.length > 2000;
  const diffPrefix = (kind) => kind === "removed" ? "− " : kind === "added" ? "+ " : "  ";
  const statusText = display.saved ? `Enregistrée · version ${display.saved.version}`
    : display.save_error ? "Non enregistrée"
    : status === "denied" ? "Ignorée · rien n’a été enregistré"
    : status === "resolved" ? "Enregistrement autorisé"
    : status === "pending" ? "En attente de vous" : "Proposition expirée ou annulée · rien n’a été enregistré";
  return <HumanInputCard message={message} title={before ? "Nouvelle version de skill proposée" : "Nouvelle skill proposée"} input={input} statusText={statusText}>
    {!content ? <p role="alert">Contenu indisponible. Ignorez cette proposition et demandez-en une nouvelle.</p> : <div className="gb-skill-proposal space-y-3">
      <div><h3 className="text-sm font-semibold">{content.display_name || content.name}</h3><p className="mt-1 text-sm text-muted-foreground">{content.description || "Sans description"}</p></div>
      <p className="text-xs text-muted-foreground">{before ? `Version ${before.version} → ${before.version + 1} · remplacement complet` : "Version 1"} · Texte proposé par le bot, à vérifier. Aucun droit d’exécution n’est accordé.</p>
      <div><h4 className="mb-1 text-xs font-semibold">Contenu proposé</h4><pre className="gb-skill-text">{content.body}</pre></div>
      {content.files?.length > 0 && <section aria-label="Fichiers texte annexes" className="space-y-2"><h4 className="text-xs font-semibold">Fichiers annexes · {content.files.length}</h4>{content.files.map((file) => <details key={file.path}><summary className="text-sm">{file.path}</summary><pre className="gb-skill-text mt-2">{file.content}</pre></details>)}</section>}
      {before && <details open className="text-sm"><summary>Différences avec la version {before.version}</summary>{diff.length ? <pre className="gb-skill-text mt-2" aria-label="Différences de la skill">{largeDiff ? diff.map((line) => `${diffPrefix(line.kind)}${line.text}\n`).join("") : diff.map((line, index) => <span key={index} className={`gb-skill-diff-${line.kind}`}>{diffPrefix(line.kind)}{line.text}{"\n"}</span>)}</pre> : <p className="mt-2 text-xs text-muted-foreground">Contenu identique à la version actuelle.</p>}</details>}
      {pending && <><p className="text-xs text-muted-foreground">Rien n’est enregistré sans votre clic. Chaque version demande votre accord.</p><fieldset className="flex flex-wrap gap-2" disabled={disabled}>
        <button type="button" className="gb-retry gb-retry-primary" onClick={() => act("forms.decide", { decision: "allow_once" })}>Enregistrer</button>
        <button type="button" className="gb-retry" onClick={() => act("forms.decide", { decision: "deny" })}>Ignorer</button>
      </fieldset></>}
      {pending && expired && <p className="text-sm">Cette proposition a expiré. Demandez au bot de la proposer à nouveau.</p>}
      {display.save_error && <p role="alert" className="text-sm">{display.save_error} Rien n’a été modifié par cette proposition.</p>}
    </div>}
  </HumanInputCard>;
}
