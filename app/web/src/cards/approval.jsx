import { HumanInputCard, inputClass, useHumanInput } from "./form.jsx";
import { useState } from "react";

export const kind = "approval";

export function Card({ message, busy, onAction }) {
  const input = useHumanInput(message, onAction);
  const { request, pending, act } = input;
  const [scope, setScope] = useState("subject");
  const action = request.kind || {};
  const expired = Number.isFinite(action.expires_at) && action.expires_at <= Date.now();
  const disabled = busy || input.busy || !pending || expired;
  // PS01/PS02: the action is read-only. The preview may be redacted; entry_id,
  // not an echo of that preview, binds the server's canonical action digest.
  const decide = (decision) => act("forms.decide", { decision });
  return (
    <HumanInputCard message={message} title="Approbation demandée" input={input}>
      <div className="space-y-3">
        <p className="text-sm">Outil : <code>{action.tool}</code></p>
        {request.arguments == null
          ? <p className="text-sm" role={pending ? "alert" : undefined}>Aperçu de l’action indisponible.{pending ? " Refusez la demande si vous ne pouvez pas la vérifier." : ""}</p>
          : <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-background p-3 text-xs" aria-label="Arguments de l’action">{JSON.stringify(request.arguments, null, 2)}</pre>}
        <details className="text-xs text-muted-foreground"><summary>Empreinte et expiration</summary><p className="break-all">{action.args_digest}</p><p>{Number.isFinite(action.expires_at) ? new Date(action.expires_at).toLocaleString("fr-CA") : "Échéance indisponible"}</p></details>
        {pending && <fieldset className="space-y-3" disabled={disabled}>
          <div className="flex flex-wrap gap-2">
            <button type="button" className="gb-retry gb-retry-primary" disabled={request.arguments == null} onClick={() => decide("allow_once")}>Autoriser une fois</button>
            <button type="button" className="gb-retry" onClick={() => decide("deny")}>Refuser</button>
          </div>
          <details>
            <summary className="text-sm">Mémoriser une règle…</summary>
            <label className="mt-2 block space-y-1 text-sm"><span>Portée pour ce bot et cet outil</span><select className={inputClass} id={`${message.id}-approval-scope`} value={scope} onChange={(event) => setScope(event.target.value)}><option value="subject">Seulement ces arguments identiques</option><option value="global">Toutes les actions de cet outil</option></select></label>
            <p className="my-2 text-xs text-muted-foreground">Les prochaines demandes correspondantes seront traitées automatiquement. Vous pouvez retirer la règle dans les réglages.</p>
            <div className="flex flex-wrap gap-2"><button type="button" className="gb-retry" disabled={request.arguments == null} onClick={() => decide({ always_allow: { scope } })}>Toujours autoriser</button><button type="button" className="gb-retry" onClick={() => decide({ never: { scope } })}>Toujours refuser</button></div>
          </details>
        </fieldset>}
        {pending && expired && <p className="text-sm">Le délai de décision est écoulé. Actualisation de l’état…</p>}
      </div>
    </HumanInputCard>
  );
}
