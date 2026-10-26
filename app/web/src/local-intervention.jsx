import { TaskInput } from "./cards/tool.jsx";
import { useRef, useState } from "react";

export function LocalIntervention({ intervention, busy, connected, index, count, onAction }) {
  const { message, background, label } = intervention;
  const title = count > 1 ? `Tâche ${index + 1} · ${label}` : label;
  const lock = useRef(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState("");
  async function stop() {
    if (lock.current || busy || !connected) return;
    lock.current = true;
    setStopping(true);
    setError("");
    try {
      if (await onAction(background ? "local.task.stop" : "local.stop") !== true) setError("L’arrêt n’a pas pu être confirmé. Actualisez l’état avant de réessayer.");
    } catch { setError("L’arrêt n’a pas pu être confirmé. Actualisez l’état avant de réessayer."); }
    finally { lock.current = false; setStopping(false); }
  }
  return <article className="gb-message gb-local-intervention" data-message-id={message.id} data-local-state={background ? "en cours" : "running"} aria-label={title}>
    <header className="gb-local-header"><strong>{title}</strong></header>
    <button type="button" className="gb-retry gb-local-stop" disabled={busy || !connected || stopping} onClick={stop}>{background ? "Arrêter" : "Annuler l’exécution"}</button>
    {background && !window.EzerbotIPhone?.remote && <TaskInput disabled={busy || !connected || stopping} onSend={(line) => onAction("local.task.input", { line })} />}
    {error && <p className="gb-local-failure" role="alert">{error}</p>}
  </article>;
}
