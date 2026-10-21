import { useState } from "react";
import { HumanInputCard } from "./form.jsx";
import { skillDiff, skillDiffStats } from "../skill-proposal.js";
import { savedDemonstrationText } from "../demonstration-presentation.js";

// Rendered only by the existing approval card; not a separate permission system.
export function SkillProposal({ message, input, disabled, expired }) {
  const { request, pending, status, act } = input;
  const [loaded, setLoaded] = useState(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState("");
  const summary = request.arguments || {};
  const full = loaded?.entryId === request.entry_id ? loaded.arguments : null;
  const display = full ? { ...full, ...summary, proposal: full.proposal, before: full.before } : summary;
  const content = display.proposal;
  const before = display.before;
  const hasContent = typeof content?.body === "string" && (!before || typeof before.content?.body === "string");
  const diff = hasContent && before ? skillDiff(before.content, content) : [];
  async function loadPreview() {
    if (hasContent || loading) return;
    setLoading(true); setLoadError("");
    try {
      const response = await window.GrokbotPanels.api("/api/forms/skill-content", {
        entry_id: request.entry_id, conversation_id: request.route.target_conversation_id,
      });
      if (typeof response.request?.arguments?.proposal?.body !== "string") throw new Error("Missing content");
      setLoaded({ entryId: request.entry_id, arguments: response.request.arguments });
    } catch { setLoadError("Le contenu n’a pas pu être chargé. Réessayez après reconnexion."); }
    finally { setLoading(false); }
  }
  const counts = skillDiffStats(diff);
  const largeDiff = diff.length > 2000;
  const diffPrefix = (kind) => kind === "removed" ? "− " : kind === "added" ? "+ " : "  ";
  const saved = display.saved;
  const settled = Boolean(saved || display.save_error || ["denied", "expired", "cancelled"].includes(status));
  const badgeText = saved ? `Enregistrée · v${saved.version}` : display.save_error ? "Non enregistrée"
    : status === "denied" ? "Ignorée" : status === "expired" ? "Expirée" : status === "cancelled" ? "Annulée"
    : status === "resolved" ? "En attente d’enregistrement" : null;
  const badge = badgeText && <span className={`gb-skill-badge ${saved ? "gb-skill-badge-saved" : display.save_error ? "gb-skill-badge-error" : ""}`} role="status">{badgeText}</span>;
  const statusText = saved ? "Enregistrement confirmé" : display.save_error ? "La proposition n’a pas été enregistrée"
    : status === "denied" ? "Rien n’a été enregistré" : status === "resolved" ? "Votre accord a été reçu"
    : status === "pending" ? "En attente de vous" : "Rien n’a été enregistré";
  const title = saved ? (before ? "Skill mise à jour" : "Skill enregistrée") : display.save_error ? "Skill non enregistrée"
    : settled ? "Proposition de skill" : before ? "Nouvelle version de skill proposée" : "Nouvelle skill proposée";
  const preview = hasContent && <div className="space-y-3">
    <p className="text-xs text-muted-foreground">{saved ? `Version ${saved.version}` : before ? `Version ${before.version} → ${before.version + 1} · remplacement complet` : "Version 1"}
      {pending && " · Texte rédigé par le bot, à vérifier."} Aucun droit d’exécution n’est accordé.</p>
    <div><h4 className="mb-1 text-xs font-semibold">{saved ? "Contenu enregistré" : settled ? "Contenu de la proposition" : "Contenu proposé"}</h4><pre className="gb-skill-text">{content.body}</pre></div>
    {content.files?.length > 0 && <section aria-label="Fichiers texte annexes" className="space-y-2"><h4 className="text-xs font-semibold">Fichiers annexes · {content.files.length}</h4>{content.files.map((file) => <details key={file.path}><summary className="text-sm">{file.path}</summary><pre className="gb-skill-text mt-2">{file.content}</pre></details>)}</section>}
    {before && <details open className="text-sm"><summary>Différences avec la version {before.version}</summary>{diff.length ? <>
      <p className="gb-skill-diff-counts"><span>+{counts.added} {counts.added === 1 ? "ligne ajoutée" : "lignes ajoutées"}</span> / <span>−{counts.removed} {counts.removed === 1 ? "ligne supprimée" : "lignes supprimées"}</span></p>
      <pre className="gb-skill-text mt-2" aria-label="Différences de la skill">{largeDiff ? diff.map((line) => `${diffPrefix(line.kind)}${line.text}\n`).join("") : diff.map((line, index) => <span key={index} className={`gb-skill-diff-line gb-skill-diff-${line.kind}`}>{diffPrefix(line.kind)}{line.text}</span>)}</pre>
    </> : <p className="mt-2 text-xs text-muted-foreground">Contenu identique à la version actuelle.</p>}</details>}
  </div>;
  return <HumanInputCard message={message} title={title} input={input} statusText={statusText} badge={badge} showDelivery={false}>
    {!content ? <p role="alert">Contenu indisponible. Ignorez cette proposition et demandez-en une nouvelle.</p> : <div className="gb-skill-proposal space-y-3">
      <div><h3 className="text-sm font-semibold">{content.display_name || content.name}</h3><p className="mt-1 text-sm text-muted-foreground">{content.description || "Sans description"}</p></div>
      {settled ? <details className="gb-skill-saved-preview" key="settled" onToggle={(event) => { if (event.currentTarget.open) void loadPreview(); }}><summary>{saved ? "Voir le contenu enregistré" : "Voir la proposition"}</summary><div className="mt-3">{hasContent ? preview : loadError
        ? <div><p className="gb-skill-error" role="alert">{loadError}</p><button type="button" className="gb-retry" disabled={loading} onClick={() => void loadPreview()}>Réessayer</button></div>
        : <p className="text-xs text-muted-foreground" role="status">Chargement du contenu…</p>}</div></details> : preview || <p className="text-xs text-muted-foreground" role="status">Votre accord est reçu. L’enregistrement est en cours…</p>}
      {display.demonstration && <div className="text-xs text-muted-foreground space-y-2"><p>{saved ? savedDemonstrationText(display.demonstration) : settled ? "L’enregistrement de la démonstration a été supprimé." : <>Apprise depuis votre démonstration. {display.demonstration.keep_images ? "Les captures seront conservées localement si vous enregistrez." : "Seul le lien vers la synthèse sera conservé, sans les captures."} Ignorer supprime l’enregistrement.</>}</p>{(!settled || saved) && <button type="button" className="gb-retry" onClick={() => window.EzerbotDemonstration?.showSummary(display.demonstration.id)}>Voir la synthèse de démonstration</button>}</div>}
      {pending && <><p className="text-xs text-muted-foreground">Rien n’est enregistré sans votre clic. Chaque version demande votre accord.</p><fieldset className="flex flex-wrap gap-2" disabled={disabled}>
        <button type="button" className="gb-retry gb-retry-primary" onClick={() => act("forms.decide", { decision: "allow_once" })}>Enregistrer</button>
        <button type="button" className="gb-retry" onClick={() => act("forms.decide", { decision: "deny" })}>Ignorer</button>
      </fieldset></>}
      {input.busy && <p className="text-xs text-muted-foreground" role="status">Décision en cours…</p>}
      {pending && expired && <p className="text-sm">Cette proposition a expiré. Demandez au bot de la proposer à nouveau.</p>}
      {display.save_error && <p role="alert" className="gb-skill-error text-sm">{display.save_error} Rien n’a été modifié par cette proposition.</p>}
    </div>}
  </HumanInputCard>;
}
