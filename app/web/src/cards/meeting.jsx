import { Streamdown } from "streamdown";
import { formatMeetingDuration } from "../meeting-presentation.js";

export const kind = "meeting";
export function Card({ message, onAction }) {
  const meeting = message.payload || {};
  const streaming = meeting.summary_state === "streaming";
  const retry = ["failed", "stopped"].includes(meeting.summary_state);
  return <article className="gb-message gb-engine-card gb-meeting" data-message-id={message.id} data-card-kind="meeting">
    <header><span className="gb-engine-label">Réunion · Capture consentie</span><h3>{meeting.title || "Réunion"}</h3></header>
    <p className="gb-engine-note">Microphone · {formatMeetingDuration(meeting.duration_ms)} · Transcription sur ce Mac</p>
    {meeting.missing_segments > 0 && <p className="gb-meeting-partial" role="note">Transcription partielle · {meeting.missing_segments} passage{meeting.missing_segments > 1 ? "s" : ""} non transcrit{meeting.missing_segments > 1 ? "s" : ""}. Les passages manquants sont signalés dans la transcription.</p>}
    {message.body ? <div className="gb-meeting-summary"><Streamdown isAnimating={streaming}>{message.body}</Streamdown></div>
      : <p className="gb-engine-note">{streaming ? "Le bot prépare le résumé, les décisions et les actions…" : "Transcription conservée. Le résumé n’a pas encore été produit."}</p>}
    {meeting.summary_error && <p className="gb-meeting-error" role="alert">{meeting.summary_error}</p>}
    {meeting.summary_group && meeting.summary_state !== "pending"
      ? <p className="gb-engine-note">Pour poursuivre ou demander un autre résumé, envoyez un message au bot dans ce groupe.</p>
      : streaming ? <button type="button" className="gb-retry" onClick={() => onAction("meeting-stop-summary")}>Arrêter le résumé</button>
      : <button type="button" className="gb-retry" onClick={() => onAction("meeting-summary")}>{retry ? "Réessayer le résumé" : meeting.summary_state === "pending" ? "Produire le résumé" : "Refaire le résumé"}</button>}
    <details className="gb-meeting-transcript"><summary>Transcription jointe</summary><pre>{meeting.transcript}</pre></details>
    {meeting.audio_url ? <div className="gb-meeting-audio">
      <p className="gb-engine-note">Audio gardé sur ce Mac jusqu’au {new Date(meeting.audio_expires_at).toLocaleDateString("fr-CA", { day: "numeric", month: "long", year: "numeric" })}.</p>
      <audio controls preload="metadata" src={meeting.audio_url} aria-label="Écouter l’audio de la réunion" />
      <button type="button" className="gb-retry" onClick={() => onAction("meeting-delete-audio")}>Supprimer l’audio</button>
    </div> : <p className="gb-engine-note">Audio supprimé · {message.body ? "La transcription et le résumé sont conservés." : "La transcription est conservée."}</p>}
    {message.body && <p className="gb-engine-note">Résumé généré par le bot : vérifiez les décisions, responsables et échéances avant de les utiliser. Aucune action n’est exécutée par cette carte.</p>}
  </article>;
}
