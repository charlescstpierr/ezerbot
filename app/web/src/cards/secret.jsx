import { useRef, useState } from "react";
import { HumanInputCard, inputClass, useHumanInput } from "./form.jsx";

export const kind = "secret";

export function Card({ message, busy, onAction }) {
  const input = useHumanInput(message, onAction);
  const { request, pending, status, act } = input;
  const name = request.kind?.name || "";
  const purpose = (request.kind?.purpose || "").trim();
  const label = purpose || name || "Secret demandé";
  const resolved = status === "resolved";
  const storage = window.GrokbotForms?.storage?.();
  const field = useRef(null);
  const [validation, setValidation] = useState("");
  const disabled = busy || input.busy || !pending;
  const confirmation = name ? `Secret ${name} enregistré` : "";
  async function submit(event) {
    event.preventDefault();
    if (disabled) return;
    const value = field.current.value.trim();
    if (value.length < 8 || new TextEncoder().encode(value).length > 4096) {
      setValidation("Utilisez au moins 8 caractères et au plus 4 096 octets.");
      return;
    }
    setValidation("");
    // PS07: clear the DOM before handing the value to the separate secret API.
    // Never retain it in React state, restore it on failure, or send it as chat.
    field.current.value = "";
    await act("forms.secret", { value });
  }
  function cancel() {
    if (field.current) field.current.value = "";
    setValidation("");
    act("forms.cancel");
  }
  return (
    <HumanInputCard message={message} title={resolved && confirmation ? confirmation : label} input={input} statusText={resolved && confirmation ? confirmation : undefined}>
      {pending && <>
        <p className="text-sm">{label}</p>
        {name && <p className="my-2 text-xs text-muted-foreground">{name}</p>}
        <p className="mb-3 text-xs text-muted-foreground">La valeur reste sur ce Mac (trousseau macOS ou fichier local, selon Paramètres), jamais dans un message. Le bot ne reçoit qu’une référence au secret.</p>
        {storage?.backend === "file_fallback" && <p className="gb-secret-storage-warning mb-3 rounded-md border p-2 text-xs" role="status"><strong>Trousseau indisponible.</strong> Les nouveaux secrets ne sont pas enregistrés. Vérifiez le stockage dans Paramètres.</p>}
        <form className="space-y-3" onSubmit={submit} autoComplete="off">
          <fieldset className="space-y-3" disabled={disabled}>
            <label className="block space-y-1 text-sm"><span className="sr-only">Valeur</span><input className={inputClass} id={`${message.id}-secret-value`} ref={field} type="password" autoComplete="off" spellCheck={false} autoCapitalize="none" required aria-describedby={`${message.id}-secret-help`} onChange={() => setValidation("")} /></label>
            <p className="text-xs text-muted-foreground" id={`${message.id}-secret-help`}>De 8 caractères à 4 096 octets. Le champ sera effacé dès l’envoi, même si la connexion échoue.</p>
            {validation && <p className="text-sm" role="alert">{validation}</p>}
            <div className="flex flex-wrap gap-2"><button type="submit" className="gb-retry gb-retry-primary">Enregistrer</button><button type="button" className="gb-retry" onClick={cancel}>Annuler</button></div>
          </fieldset>
        </form>
      </>}
    </HumanInputCard>
  );
}
