export const kind = "voice_call";

// The call's text only: the audio went to the voice provider and is kept nowhere on the Mac.
export function Card({ message }) {
  const call = message.payload || {};
  const lines = Array.isArray(call.lines) ? call.lines : [];
  const bot = window.GrokbotPanels?.state?.bots?.find?.((candidate) => candidate.id === call.bot_id);
  const name = bot?.name || "Le bot";
  return <article className="gb-message gb-engine-card gb-voice-call" data-message-id={message.id} data-card-kind="voice_call">
    <header><span className="gb-engine-label">{call.label || "Appel vocal"}</span><h3>Appel avec {name}</h3></header>
    <p className="gb-engine-note">Transcript texte · Aucun fichier audio n’est gardé sur ce Mac.</p>
    {call.status === "interrupted" && <p className="gb-voice-call-partial" role="note">Appel interrompu : le transcript s’arrête à la dernière ligne écrite.</p>}
    {lines.length
      ? <ol className="gb-voice-call-lines">{lines.map((line, index) => <li key={index} data-role={line.role}><strong>{line.role === "user" ? "Vous" : name}</strong><span>{line.text}</span></li>)}</ol>
      : <p className="gb-engine-note">Aucune parole n’a été transcrite.</p>}
  </article>;
}
