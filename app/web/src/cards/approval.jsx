import { HumanInputCard, inputClass, useHumanInput } from "./form.jsx";
import { useState } from "react";
import { SkillProposal } from "./skill-proposal.jsx";
import { DraftCard } from "./draft.jsx";
import { LocalApprovalPreview, LocalApprovalWarning, SensitiveWarning, localApprovalBlocked } from "../local-approval.js";
import { localLimits, localToolLabel, localToolNames } from "../local-call.js";
import { screenTools, screenWarnings, ScreenArguments } from "../screen.jsx";
import { documentApprovalEffect } from "../generated-presentation.js";

export const kind = "approval";

function ActionArguments({ value }) {
  return <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-background p-3 text-xs" aria-label="Arguments de l’action">{JSON.stringify(value, null, 2)}</pre>;
}

export function Card({ message, busy, onAction }) {
  const input = useHumanInput(message, onAction);
  const { request, pending, act } = input;
  const [scope, setScope] = useState(message.payload?.connector ? "global" : "subject");
  const action = request.kind || {};
  const screen = Boolean(screenTools[action.tool]);
  const lifecycle = request.connector && typeof action.tool === "string" && (action.tool.startsWith("mcp_start_") || action.tool.startsWith("mcp_remove_"));
  const background = action.tool === "routines_background_configure";
  const publication = action.tool === "routines_webhook_publish";
  const hostAction = background || publication;
  const configuration = hostAction || lifecycle ? request.configuration_result : null;
  const preview = request.arguments;
  const previewAvailable = preview != null && (!lifecycle || (typeof preview.commande === "string" && Array.isArray(preview.arguments) && Array.isArray(preview.variables_privées)));
  const argumentPreview = screen ? <ScreenArguments tool={action.tool} arguments={preview || {}} /> : <ActionArguments value={preview} />;
  const expired = Number.isFinite(action.expires_at) && action.expires_at <= Date.now();
  const disabled = busy || input.busy || !pending || expired;
  const blocked = !previewAvailable || localApprovalBlocked(action.tool, preview);
  // entry_id, not an echo of a redacted preview, binds the canonical action.
  const decide = (decision) => act("forms.decide", { decision });
  const recall = request.recall;
  if (recall) {
    const join = recall.action === "join";
    const result = request.configuration_result;
    return (
      <HumanInputCard message={message} title={join ? "Inviter le bot dans la réunion" : "Déconnecter Réunions"} input={input} showDelivery={false}>
        <div className="space-y-3" data-follow-actions="true" data-recall-card={recall.action}>
          {join ? <>
            <dl className="grid gap-3 text-sm" aria-label="Invitation à la réunion">
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Lien de la réunion</dt><dd className="mt-1 ml-0 whitespace-pre-wrap break-words">{recall.link}</dd></div>
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Nom vu par les participants</dt><dd className="mt-1 ml-0 break-words">{recall.bot_name}</dd></div>
            </dl>
            <p className="text-sm">{recall.phrase}</p>
          </> : <p className="text-sm">Déconnecter {recall.name} retire la clé Recall de ce Mac. Une réunion en cours est quittée avant le retrait.</p>}
          {pending && <fieldset className="space-y-3 border-0 p-0 m-0" disabled={disabled}>
            <div className="flex flex-wrap gap-2">
              <button type="button" className="gb-retry gb-retry-primary" onClick={() => decide("allow_once")}>{join ? "Accepter" : "Confirmer"}</button>
              <button type="button" className="gb-retry" onClick={() => decide("deny")}>Annuler</button>
            </div>
          </fieldset>}
          {input.status === "denied" && <p className="text-sm" role="status">{join ? "Annulé : personne n’a été invité." : "Annulé : la clé est conservée."}</p>}
          {!pending && input.status === "resolved" && <p className={`text-sm${result?.status === "failed" ? " gb-status-alert" : ""}`} role={result?.status === "failed" ? "alert" : "status"}>{result?.status === "failed" ? `Échec : ${result.reason}` : result?.status === "done" ? result.output?.message : "En cours…"}</p>}
          {pending && expired && <p className="text-sm">Le délai de décision est écoulé. Actualisation de l’état…</p>}
        </div>
      </HumanInputCard>
    );
  }
  const backgroundTask = request.background_task;
  if (backgroundTask) {
    const decision = request.decision;
    const always = Boolean(decision && typeof decision === "object" && decision.always_allow);
    const once = decision === "allow_once";
    const refused = input.status === "denied" || decision === "deny" || Boolean(decision?.never);
    const state = once ? "Autorisé une fois" : always ? "Toujours autorisé" : refused ? "Refusé" : "";
    const bot = backgroundTask.bot_name || "le bot";
    return (
      <HumanInputCard message={message} title={`Autoriser ${bot} à exécuter une tâche en arrière-plan ?`} input={input}>
        <div className="space-y-3">
          <div className="gb-background-box">
            <p className="gb-background-kind">{backgroundTask.kind}</p>
            <p className="gb-background-summary">{backgroundTask.summary}</p>
          </div>
          {state && <p className={refused ? "gb-status-alert" : "gb-status-ok"} role="status">{state}</p>}
          {pending && <div className="flex flex-wrap gap-2">
            <button type="button" className="gb-retry gb-retry-primary" disabled={disabled} onClick={() => decide("allow_once")}>Autoriser une fois</button>
            {request.policy?.remember && <button type="button" className="gb-retry" disabled={disabled} data-always-scope="global" onClick={() => decide({ always_allow: { scope: "global" } })}>Toujours autoriser</button>}
            <button type="button" className="gb-retry" onClick={() => decide("deny")}>Refuser</button>
          </div>}
        </div>
      </HumanInputCard>
    );
  }
  if (preview?.brouillon) return <DraftCard message={message} input={input} disabled={disabled} expired={expired} />;
  if (action.tool === "propose_skill") return <SkillProposal message={message} input={input} disabled={disabled} expired={expired} />;
  const review = request.review;
  const automatic = !screen && !localToolNames[action.tool] && !review?.pending && ((review?.answer?.decision === "allow" && input.status === "resolved")
    || (review?.answer?.decision === "deny" && input.status === "denied"));
  // The corresponding ScreenCard carries the durable decision in its footer.
  if (screen && !pending) return null;
  if (automatic) return (
    <article className={`gb-message rounded-xl border border-border bg-card px-4 py-3 text-foreground${input.status === "denied" ? " gb-approval-denied" : ""}`} data-message-id={message.id} data-card-kind="approval">
      {input.status === "resolved" && <p className="gb-status-ok" role="status">Autorisé une fois</p>}
      <p className="text-sm" role="status"><strong className={input.status === "denied" ? "gb-status-alert" : undefined}>{input.status === "resolved" ? "Approuvé automatiquement : " : "Refusé automatiquement : "}</strong>{review.answer.reason}</p>
      <p className="mt-1 text-xs text-muted-foreground">{request.connector ? <>Connecteur : <strong>{request.connector.server}</strong><br />Outil : <code>{request.connector.tool}</code></> : <>Outil : <code>{action.tool}</code></>}</p>
      {preview == null ? <p className="mt-2 text-xs text-muted-foreground">Aperçu de l’action indisponible.</p> : <div className="mt-2 space-y-2 text-xs text-muted-foreground">
        <code className="block truncate" title={JSON.stringify(preview)}>{JSON.stringify(preview)}</code>
        <details><summary>Voir l’action complète</summary><div className="mt-2">{argumentPreview}</div></details>
      </div>}
      <button type="button" className="gb-retry mt-2" onClick={() => act("forms.rules")}>Modifier les règles pour les prochains appels</button>
    </article>
  );
  const reviewing = review?.pending && review.started_at + 12000 > Date.now();
  const policy = request.policy;
  const outbound = policy?.protection === "outbound";
  const destructive = policy?.protection === "destructive";
  // Disconnecting an account is a deletion: the card names the account and offers Confirmer / Annuler only.
  const account = lifecycle && action.tool.startsWith("mcp_remove_") && typeof preview?.catalogue?.compte === "string" ? preview.catalogue : null;
  const title = account ? `Déconnecter le compte ${account.service} ${account.compte} ?` : outbound ? "Envoi sous votre nom" : destructive ? "Confirmation d’une suppression" : "Approbation demandée";
  const showRemember = policy
    ? Boolean(policy.remember) && !screen && !lifecycle
    : !screen && !lifecycle && !["read_file", "list_directory"].includes(action.tool);
  return (
    <HumanInputCard message={message} title={title} input={input} showDelivery={!hostAction && !lifecycle}>
      <div className="space-y-3" data-follow-actions={hostAction || screen || localToolNames[action.tool] || request.connector?.origin === "settings" ? "true" : undefined}>
        {pending && review && <p className="text-xs text-muted-foreground" role="status">
          {reviewing && <span className="gb-reviewer-progress" aria-hidden="true"><i /><i /><i /></span>}
          <strong>Relecteur : </strong>{review.pending
            ? (reviewing ? "Relecture des règles en cours… Vous pouvez aussi décider maintenant." : "La revue a été interrompue ou son délai est écoulé. À vous de décider.")
            : review.answer.reason}
        </p>}
        {request.connector ? <p className="text-sm">Connecteur : <strong>{request.connector.server}</strong><br />{lifecycle ? <>Action : <span>{request.connector.tool}</span></> : <>Outil : <code>{request.connector.tool}</code></>}</p> : <p className="text-sm">{screen ? <>Action : {screenTools[action.tool]}</> : background ? <>Action : {preview?.enabled ? "Installer l’agent de routines" : "Désinstaller l’agent de routines"}</> : publication ? <>Action : {preview?.enabled ? "Publier les webhooks sur Internet" : "Retirer la publication des webhooks"}</> : localToolNames[action.tool] ? <>Action : {localToolLabel(action.tool)}</> : <>Outil : <code>{action.tool}</code></>}</p>}
        {action.tool === "run_shell" && preview && <><p className="text-sm">Commande exacte sur votre Mac (sans sandbox) :</p><pre className="gb-local-command">{preview.command}</pre><p className="text-xs">Dossier : {preview.cwd} · {localLimits(preview)}</p></>}
        {screen && <><p className="text-sm">{screenWarnings[action.tool]}</p><p className="text-xs text-muted-foreground">Ezerbot se masque brièvement pour viser le bureau derrière sa fenêtre. Autoriser reprend explicitement le bot en pause pour ce geste.</p></>}
        <LocalApprovalPreview tool={action.tool} arguments={preview} />
        {action.tool === "document_action" && preview && <div className="gb-document-approval"><strong className="text-sm font-semibold">{preview.document}</strong><p className="text-sm">{preview.action}</p>{preview.destination && <p className="gb-file-path">{preview.destination}</p>}<p className="text-xs text-muted-foreground">{documentApprovalEffect(preview.action_id)}</p></div>}
        {!previewAvailable
          ? <p className="text-sm" role={pending ? "alert" : undefined}>Aperçu de l’action indisponible.{pending ? " Refusez la demande si vous ne pouvez pas la vérifier." : ""}</p>
          : background ? <p className="text-sm">{preview.enabled ? "Autoriser Ezerbot à exécuter les routines dues quand l’app est fermée ? Un agent macOS sera installé pour votre session uniquement. Les résultats et les alertes suivront vos réglages habituels." : "Retirer l’agent macOS de cette session ? Les routines ne s’exécuteront plus quand Ezerbot est fermé. Vos routines, résultats et historiques seront conservés."}</p> : publication ? <p className="text-sm">{preview.enabled ? "Autoriser Ezerbot à publier l’écoute des webhooks de routines sur Internet avec Tailscale Funnel ? Seul le chemin des webhooks devient public, sur son propre port HTTPS : chaque appel exige son jeton Bearer. L’adresse privée de l’iPhone et Tailscale Serve ne changent pas." : "Retirer la publication Funnel des webhooks ? Les appels depuis Internet n’arriveront plus. L’adresse privée de l’iPhone ne change pas."}</p> : lifecycle ? <>
            <dl className="grid gap-3 text-sm" aria-label="Configuration du connecteur">
              {!(preview.catalogue && action.tool.startsWith("mcp_remove_")) && <>
                <div className="min-w-0"><dt className="text-xs text-muted-foreground">{preview.catalogue?.transport === "https" || preview.transport === "https" ? "Adresse du service" : "Commande"}</dt><dd className="mt-1 ml-0 whitespace-pre-wrap break-words">{preview.commande}</dd></div>
                {!preview.catalogue && preview.transport !== "https" && <div className="min-w-0"><dt className="text-xs text-muted-foreground">Arguments</dt><dd className="mt-1 ml-0">{preview.arguments.length ? <ol className="space-y-1">{preview.arguments.map((arg, i) => <li className="whitespace-pre-wrap break-words" key={i}>{arg}</li>)}</ol> : "Aucun argument."}</dd></div>}
              </>}
              <div className="min-w-0"><dt className="text-xs text-muted-foreground">Variables privées</dt><dd className="mt-1 ml-0 break-words">{preview.variables_privées.length ? `${preview.variables_privées.join(", ")} · valeurs masquées` : preview.catalogue ? "Les jetons reçus après votre consentement seront conservés dans le coffre." : "Aucune variable privée."}</dd></div>
            </dl>
            {preview.catalogue && <div className="space-y-2 text-sm"><p><strong>{preview.catalogue.service}</strong> · {preview.catalogue.licence}</p>{account && <p>Compte : <strong>{account.compte}</strong></p>}<p>{preview.catalogue.permissions}</p><p className="text-xs text-muted-foreground">Portées OAuth : {preview.catalogue.portées}</p><p>{action.tool.startsWith("mcp_remove_") ? preview.catalogue.déconnexion : preview.catalogue.connexion}</p></div>}
            {action.tool.startsWith("mcp_remove_") && !preview.catalogue && <p className="text-sm">Le processus sera arrêté. La configuration, les règles permanentes et les variables privées de ce connecteur seront supprimées.</p>}
          </> : localToolNames[action.tool] ? <details><summary className="text-xs">Détails techniques de l’action</summary><p className="text-xs">Identifiant de l’outil : <code>{action.tool}</code></p>{argumentPreview}</details> : argumentPreview}
        <details className="text-xs text-muted-foreground"><summary>Empreinte et expiration</summary><p className="break-all">{action.args_digest}</p><p>{Number.isFinite(action.expires_at) ? new Date(action.expires_at).toLocaleString("fr-CA") : "Échéance indisponible"}</p></details>
        {action.tool === "write_file" && <SensitiveWarning preview={preview?.apercu_local} />}
        {pending && <fieldset className="space-y-3 border-0 p-0 m-0" disabled={disabled}>
          <div className="flex flex-wrap gap-2">
            <button type="button" className="gb-retry gb-retry-primary" disabled={blocked} onClick={() => decide("allow_once")}>{account ? "Confirmer" : screen ? "Autoriser une fois et reprendre" : "Autoriser une fois"}</button>
            {policy?.remember && <button type="button" className="gb-retry" disabled={blocked} data-always-scope={policy.scope} onClick={() => decide({ always_allow: { scope: policy.scope } })}>Toujours autoriser</button>}
            <button type="button" className="gb-retry" onClick={() => decide("deny")}>{account ? "Annuler" : "Refuser"}</button>
          </div>
          {outbound && <p className="text-sm">Cet envoi part sous votre nom. Chaque envoi demande cette confirmation.</p>}
          {destructive && <p className="text-sm">Cette suppression demande une confirmation à chaque fois.</p>}
          {showRemember && <details>
            <summary className="text-sm">Mémoriser une règle…</summary>
            <label className="mt-2 block space-y-1 text-sm"><span>Portée pour ce bot et cet outil</span><select className={inputClass} id={`${message.id}-approval-scope`} value={scope} onChange={(event) => setScope(event.target.value)}><option value="subject">Seulement ces arguments identiques</option><option value="global">Toutes les actions de cet outil</option></select></label>
            <p className="my-2 text-xs text-muted-foreground">Les prochaines demandes correspondantes seront traitées automatiquement. Vous pouvez retirer la règle dans les réglages.</p>
            <LocalApprovalWarning tool={action.tool} scope={scope} />
            <div className="flex flex-wrap gap-2"><button type="button" className="gb-retry" disabled={blocked} onClick={() => decide({ always_allow: { scope } })}>Toujours autoriser</button><button type="button" className="gb-retry" onClick={() => decide({ never: { scope } })}>Toujours refuser</button></div>
          </details>}
        </fieldset>}
        {(hostAction || lifecycle) && !pending && input.status === "resolved" && <p className={`text-sm${configuration?.status === "failed" ? " gb-status-alert" : ""}`} role={configuration?.status === "failed" ? "alert" : "status"}>{configuration?.status === "failed" ? `Échec : ${configuration.reason} Réessayez depuis Paramètres.` : configuration?.status === "done" ? configuration.output?.message : "Configuration en cours…"}</p>}
        {pending && expired && <p className="text-sm">Le délai de décision est écoulé. Actualisation de l’état…</p>}
        {input.status === "expired" && <p className="text-sm" role="status">Cette demande a expiré. Aucune action n’a été faite.</p>}
      </div>
    </HumanInputCard>
  );
}
