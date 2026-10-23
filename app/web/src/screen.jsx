import { screenTools, screenWarnings, screenActionState, screenDecisionLabel } from "./screen-view.js";
import { AUTOMATIC_LABEL, isAutomatic } from "./yolo-view.js";
export { screenTools, screenWarnings };
const screenKeyLabels = { ENTER: "Entrée", TAB: "Tabulation", ESC: "Échap", BACKSPACE: "Retour arrière", DELETE: "Supprimer", UP: "Haut", DOWN: "Bas", LEFT: "Gauche", RIGHT: "Droite", SPACE: "Espace" };
export function ScreenArguments({ tool, arguments: a = {} }) {
  if (tool === "computer_screenshot") return <p className="text-sm">Capture de l’écran principal, sans saisie ni clic.</p>;
  if (tool === "computer_type") return <div><p className="text-sm">Texte exact à saisir :</p><pre className="gb-engine-data">{a.text}</pre></div>;
  if (tool === "computer_key") return <p className="text-sm">Touche : {screenKeyLabels[a.key] || "Inconnue"} · Peut déclencher une validation ou une suppression.</p>;
  if (tool === "computer_scroll") return <p className="text-sm">Défilement vertical : {a.delta_y} pixels.</p>;
  return <p className="text-sm">Position : {a.x}, {a.y} (pixels de la dernière capture, depuis le coin supérieur gauche).{tool === "computer_click" && ` Bouton ${a.button === "right" ? "droit" : "gauche"}.`}</p>;
}
export function ScreenCard({ message }) {
  const { call, outcome, screen_image: image, authorized_by: authority } = message.payload;
  const automatic = isAutomatic(authority);
  const state = screenActionState(message.payload);
  if (state.hidden) return null;
  const a = call.arguments || {};
  const action = call.name === "computer_type" ? a.text
    : call.name === "computer_key" ? (screenKeyLabels[a.key] || "Touche inconnue")
    : ["computer_move", "computer_click"].includes(call.name) ? `${a.x}, ${a.y}${call.name === "computer_click" ? (a.button === "right" ? " · bouton droit" : " · bouton gauche") : ""}`
    : call.name === "computer_scroll" ? `${a.delta_y} pixels` : "Écran principal";
  const { reason } = state;
  return <article className="gb-message gb-engine-card screen-action-card" data-message-id={message.id} data-card-kind="tool" data-screen-error={state.error || undefined}>
    <div className="screen-action-heading"><strong>{screenTools[call.name]}</strong><span>{state.label}</span></div>
    {automatic && <p className="gb-auto-line"><span className="gb-auto-badge">{AUTOMATIC_LABEL}</span> Mode YOLO · la session d’écran reste celle que vous avez démarrée.</p>}
    {typeof image === "string" && image.startsWith("/api/screen/shot?") ? <figure><button type="button" className="screen-thumbnail" aria-label="Agrandir la capture de l’écran" onClick={() => window.EzerbotScreen?.preview(image)}><img src={image} width={outcome?.output?.width || 1280} height={outcome?.output?.height || 800} loading="lazy" alt="Capture de l’écran après l’action" /></button><figcaption>{action}</figcaption></figure> : <p className="screen-action-detail">{action}</p>}
    {outcome?.output?.fixture && <p>Écran fictif · événement enregistré, aucun effet sur le Mac.</p>}
    {reason && <p role="status">{reason}</p>}
    {!outcome && !state.error && (message.payload.approval_status === "resolved" || automatic) && <p>{automatic ? "Geste en cours d’exécution." : "Geste autorisé par vous, en cours d’exécution."}</p>}
    {reason && /permission|Accessibilité|Enregistrement|Surveillance/i.test(reason) && <div className="screen-actions"><button className="gb-retry" onClick={() => window.EzerbotScreen?.pane("accessibility")}>Ouvrir Accessibilité</button><button className="gb-retry" onClick={() => window.EzerbotScreen?.pane("recording")}>Ouvrir Enregistrement de l’écran</button><button className="gb-retry" onClick={() => window.EzerbotScreen?.pane("input")}>Ouvrir Surveillance de l’entrée</button></div>}
    {screenDecisionLabel(message.payload.approval_status) && <footer className="screen-decision"><span className={message.payload.approval_status === "resolved" ? "gb-status-ok" : undefined}>{screenDecisionLabel(message.payload.approval_status)}</span></footer>}
  </article>;
}
