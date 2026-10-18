import { useEffect, useRef, useState } from "react";

export const kind = "form";
export const inputClass = "w-full rounded-md border border-input bg-background p-2 text-sm text-foreground";

// Shared card boundary: only an engine delivery receipt can say “Repris”.
export function useHumanInput(message, onAction) {
  const [, update] = useState(0);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const lock = useRef(false);
  useEffect(() => {
    const refresh = () => update((value) => value + 1);
    window.addEventListener("grokbot:forms-change", refresh);
    return () => window.removeEventListener("grokbot:forms-change", refresh);
  }, []);
  const request = message.payload || {};
  const activity = window.GrokbotForms?.view(request.entry_id);
  const receipt = activity?.receipt || request.receipt;
  const status = request.status?.status;
  const pending = status === "pending" && !receipt;
  const busy = sending || activity?.busy;
  async function act(action, detail) {
    if (lock.current) return false;
    lock.current = true;
    setSending(true);
    setError("");
    try {
      if (!onAction || await onAction(action, detail) !== true) throw new Error("Unavailable");
      return true;
    } catch {
      setError("L’action n’a pas pu être confirmée. Actualisez l’état avant de réessayer.");
      return false;
    } finally { lock.current = false; setSending(false); }
  }
  return { request, receipt, status, pending, busy, error: status && status !== "pending" ? "" : activity?.error || error, act };
}

export function HumanInputCard({ message, title, input, children, statusText }) {
  const { request, receipt, status, busy, error, act } = input;
  const labels = { pending: "En attente de vous", resolved: "Réponse enregistrée", denied: "Refusée", expired: "Expirée", cancelled: "Annulée" };
  const wake = receipt?.wake;
  const delivered = wake?.status === "delivered" && typeof wake.run_id === "string" && Number.isInteger(wake.first_event_seq);
  const available = request.entry_id && request.route && request.kind?.kind === message.kind && labels[status];
  const recorded = message.kind === "approval" ? "Autorisée" : message.kind === "secret" ? "Secret enregistré" : "Réponse enregistrée";
  const statusLabel = status === "resolved" ? recorded : status === "pending" && receipt ? "Résolution reçue · Vérification de l’état…" : labels[status];
  const requesterId = request.route?.requesting_bot_id;
  const requesterName = window.GrokbotPanels?.state?.bots?.find((bot) => bot.id === requesterId)?.name || requesterId;
  const requesterLabel = /^[aeiouyàâäéèêëîïôöùûü]/i.test(requesterName) ? `d’${requesterName}` : `de ${requesterName}`;
  const failureReason = ["run already ended", "run ended before wake delivery"].includes(wake?.reason)
    ? "Cette tâche du bot est déjà terminée." : "La reprise n’a pas abouti. La demande est conservée.";
  return (
    <article className="gb-message rounded-xl border border-border bg-card p-4 text-foreground" data-message-id={message.id} data-card-kind={message.kind} aria-busy={Boolean(busy)}>
      <header className="gb-meta"><strong>{title}</strong></header>
      {!available ? <p role="alert">Cette demande est indisponible. Actualisez l’état.</p> : <>
        <p className="mb-3 text-xs text-muted-foreground">{request.connector?.origin === "settings" ? "Demande depuis Paramètres" : `Demande ${requesterLabel}`} · {statusText || statusLabel}</p>
        {children}
        <div className="mt-3 text-sm" role="status" aria-live="polite">
          {busy && "Envoi en cours…"}
          {!busy && wake?.status === "pending" && "Le bot reprend…"}
          {!busy && delivered && ({ denied: "Refus pris en compte", cancelled: "Annulation prise en compte", expired: "Expiration prise en compte" }[status] || "Repris")}
          {!busy && wake?.status === "failed" && `La reprise a échoué. ${failureReason}`}
        </div>
      </>}
      {error && <p className="mt-2 text-sm text-muted-foreground" role="alert">{error}</p>}
      {(!available || error) && <button type="button" className="gb-retry mt-2" disabled={busy} onClick={() => act("forms.refresh")}>Actualiser l’état</button>}
    </article>
  );
}

export function Card({ message, busy, onAction }) {
  const input = useHumanInput(message, onAction);
  const { request, pending, act } = input;
  const fields = request.kind?.fields || [];
  const disabled = busy || input.busy || !pending;
  async function submit(event) {
    event.preventDefault();
    const form = event.currentTarget;
    if (disabled || !form.reportValidity()) return;
    const data = new FormData(form);
    // Object.fromEntries handles even external names such as “__proto__” as data.
    const values = Object.fromEntries(fields.flatMap((field, index) => {
      const value = data.get(`field-${index}`);
      if (field.field_type.kind === "boolean") return [[field.name, value === "on"]];
      if (value === "" && !field.required) return [];
      return [[field.name, field.field_type.kind === "number" ? Number(value) : value]];
    }));
    await act("forms.submit", { values });
  }
  return (
    <HumanInputCard message={message} title="Informations demandées" input={input}>
      {pending && <form className="space-y-3" onSubmit={submit}>
        <fieldset className="space-y-3" disabled={disabled}>
          {fields.map((field, index) => <label className="block space-y-1 text-sm" key={field.name}>
            <span>{field.label}{field.required ? " (requis)" : " (facultatif)"}</span>
            {field.field_type.kind === "choice"
              ? <select className={inputClass} name={`field-${index}`} required={field.required} defaultValue=""><option value="" disabled={field.required}>Choisir…</option>{field.field_type.options.map((option) => <option key={option} value={option}>{option}</option>)}</select>
              : field.field_type.kind === "boolean"
                // Required boolean means a value is required, not that it must be true.
                ? <input type="checkbox" name={`field-${index}`} />
                : <input className={inputClass} name={`field-${index}`} type={field.field_type.kind === "number" ? "number" : "text"} step={field.field_type.kind === "number" ? "any" : undefined} required={field.required} pattern={field.required && field.field_type.kind === "text" ? ".*\\S.*" : undefined} />}
          </label>)}
          <div className="flex flex-wrap gap-2"><button className="gb-retry gb-retry-primary" type="submit">Envoyer les réponses</button><button className="gb-retry" type="button" onClick={() => act("forms.cancel")}>Annuler</button></div>
        </fieldset>
      </form>}
    </HumanInputCard>
  );
}
