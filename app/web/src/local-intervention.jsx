import { TaskInput } from "./cards/tool.jsx";
import { useRef, useState } from "react";

export function LocalIntervention({ intervention, busy, connected, index, count, onAction }) {
  const { message } = intervention;
  const title = count > 1 ? `Tâche ${index + 1}` : "Contrôles de la tâche";
  const lock = useRef(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState("");
  async function stop() {
    if (lock.current || busy || !connected) return;
    lock.current = true;
    setStopping(true);
    setError("");
    try {
      if (await onAction("local.task.stop") !== true) setError("L’arrêt n’a pas pu être confirmé. Actualisez l’état avant de réessayer.");
    } catch { setError("L’arrêt n’a pas pu être confirmé. Actualisez l’état avant de réessayer."); }
    finally { lock.current = false; setStopping(false); }
  }
  return <article className="gb-message gb-local-intervention" data-message-id={message.id} data-local-state="en cours" aria-label={title}>
    <button type="button" className="gb-retry gb-local-stop" disabled={busy || !connected || stopping} onClick={stop}>Arrêter</button>
    {!window.EzerbotIPhone?.remote && <TaskInput disabled={busy || !connected || stopping} onSend={(line) => onAction("local.task.input", { line })} />}
    {error && <p className="gb-local-failure" role="alert">{error}</p>}
  </article>;
}
