import { HumanInputCard, inputClass, useHumanInput } from "./form.jsx";
import { useState } from "react";
import { SkillProposal } from "./skill-proposal.jsx";

export const kind = "approval";

function ActionArguments({ value }) {
  return <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-background p-3 text-xs" aria-label="Arguments de l’action">{JSON.stringify(value, null, 2)}</pre>;
}

export function Card({ message, busy, onAction }) {
  const input = useHumanInput(message, onAction);
  const { request, pending, act } = input;
  const [scope, setScope] = useState(message.payload?.connector ? "global" : "subject");
  const action = request.kind || {};
  const lifecycle = request.connector && typeof action.tool === "string" && (action.tool.startsWith("mcp_start_") || action.tool.startsWith("mcp_remove_"));
  const preview = request.arguments;
  const previewAvailable = preview != null && (!lifecycle || (typeof preview.commande === "string" && Array.isArray(preview.arguments) && Array.isArray(preview.variables_privées)));
  const expired = Number.isFinite(action.expires_at) && action.expires_at <= Date.now();
  const disabled = busy || input.busy || !pending || expired;
  // PS01/PS02: the action is read-only. The preview may be redacted; entry_id,
  // not an echo of that preview, binds the server's canonical action digest.
  const decide = (decision) => act("forms.decide", { decision });
  if (action.tool === "propose_skill") return <SkillProposal message={message} input={input} disabled={disabled} expired={expired} />;
  const review = request.review;
  const automatic = !review?.pending && ((review?.answer?.decision === "allow" && input.status === "resolved")
    || (review?.answer?.decision === "deny" && input.status === "denied"));
  if (automatic) return (
    <article className={`gb-message rounded-xl border border-border bg-card px-4 py-3 text-foreground${input.status === "denied" ? " gb-approval-denied" : ""}`} data-message-id={message.id} data-card-kind="approval">
      <p className="text-sm" role="status"><strong className={input.status === "denied" ? "gb-status-alert" : undefined}>{input.status === "resolved" ? "Approuvé automatiquement : " : "Refusé automatiquement : "}</strong>{review.answer.reason}</p>
      <p className="mt-1 text-xs text-muted-foreground">{request.connector ? <>Connecteur : <strong>{request.connector.server}</strong><br />Outil : <code>{request.connector.tool}</code></> : <>Outil : <code>{action.tool}</code></>}</p>
      {request.arguments == null ? <p className="mt-2 text-xs text-muted-foreground">Aperçu de l’action indisponible.</p> : <div className="mt-2 space-y-2 text-xs text-muted-foreground">
        <code className="block truncate" title={JSON.stringify(request.arguments)}>{JSON.stringify(request.arguments)}</code>
        <details><summary>Voir l’action complète</summary><div className="mt-2"><ActionArguments value={request.arguments} /></div></details>
      </div>}
      <button type="button" className="gb-retry mt-2" onClick={() => act("forms.rules")}>Modifier les règles pour les prochains appels</button>
    </article>
  );
  const reviewing = review?.pending && review.started_at + 12000 > Date.now();
  return (
    <HumanInputCard message={message} title="Approbation demandée" input={input}>
      <div className="space-y-3">
        {pending && review && <p className="text-xs text-muted-foreground" role="status">
          {reviewing && <span className="gb-reviewer-progress" aria-hidden="true"><i /><i /><i /></span>}
          <strong>Relecteur : </strong>{review.pending
            ? (reviewing ? "Relecture des règles en cours… Vous pouvez aussi décider maintenant." : "La revue a été interrompue ou son délai est écoulé. À vous de décider.")
            : review.answer.reason}
        </p>}
        {request.connector ? <p className="text-sm">Connecteur : <strong>{request.connector.server}</strong><br />{lifecycle ? <>Action : <span>{request.connector.tool}</span></> : <>Outil : <code>{request.connector.tool}</code></>}</p> : <p className="text-sm">Outil : <code>{action.tool}</code></p>}
        {!previewAvailable
          ? <p className="text-sm" role={pending ? "alert" : undefined}>Aperçu de l’action indisponible.{pending ? " Refusez la demande si vous ne pouvez pas la vérifier." : ""}</p>
          : lifecycle ? <>
            <dl className="grid gap-3 text-sm" aria-label="Configuration du connecteur">
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Commande</dt><dd className="mt-1 ml-0 whitespace-pre-wrap break-words">{preview.commande}</dd></div>
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Arguments</dt><dd className="mt-1 ml-0">{preview.arguments.length ? <ol className="space-y-1">{preview.arguments.map((arg, i) => <li className="whitespace-pre-wrap break-words" key={i}>{arg}</li>)}</ol> : "Aucun argument."}</dd></div>
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Variables privées</dt><dd className="mt-1 ml-0 break-words">{preview.variables_privées.length ? `${preview.variables_privées.join(", ")} · valeurs masquées` : "Aucune variable privée."}</dd></div>
            </dl>
            {action.tool.startsWith("mcp_remove_") && <p className="text-sm">Le processus sera arrêté. La configuration, les règles permanentes et les variables privées de ce connecteur seront supprimées.</p>}
          </> : <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-background p-3 text-xs" aria-label="Arguments de l’action">{JSON.stringify(preview, null, 2)}</pre>}
        <details className="text-xs text-muted-foreground"><summary>Empreinte et expiration</summary><p className="break-all">{action.args_digest}</p><p>{Number.isFinite(action.expires_at) ? new Date(action.expires_at).toLocaleString("fr-CA") : "Échéance indisponible"}</p></details>
        {pending && <fieldset className="space-y-3" disabled={disabled}>
          <div className="flex flex-wrap gap-2">
            <button type="button" className="gb-retry gb-retry-primary" disabled={!previewAvailable} onClick={() => decide("allow_once")}>Autoriser une fois</button>
            <button type="button" className="gb-retry" onClick={() => decide("deny")}>Refuser</button>
          </div>
          {!lifecycle && <details>
            <summary className="text-sm">Mémoriser une règle…</summary>
            <label className="mt-2 block space-y-1 text-sm"><span>Portée pour ce bot et cet outil</span><select className={inputClass} id={`${message.id}-approval-scope`} value={scope} onChange={(event) => setScope(event.target.value)}><option value="subject">Seulement ces arguments identiques</option><option value="global">Toutes les actions de cet outil</option></select></label>
            <p className="my-2 text-xs text-muted-foreground">Les prochaines demandes correspondantes seront traitées automatiquement. Vous pouvez retirer la règle dans les réglages.</p>
            <div className="flex flex-wrap gap-2"><button type="button" className="gb-retry" disabled={!previewAvailable} onClick={() => decide({ always_allow: { scope } })}>Toujours autoriser</button><button type="button" className="gb-retry" onClick={() => decide({ never: { scope } })}>Toujours refuser</button></div>
          </details>}
        </fieldset>}
        {pending && expired && <p className="text-sm">Le délai de décision est écoulé. Actualisation de l’état…</p>}
      </div>
    </HumanInputCard>
  );
}
