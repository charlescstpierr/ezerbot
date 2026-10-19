export const screenTools = {
  computer_screenshot: "Capture de l’écran", computer_move: "Déplacer la souris", computer_click: "Cliquer",
  computer_scroll: "Faire défiler", computer_type: "Saisir du texte", computer_key: "Appuyer sur une touche",
};
const screenKeyLabels = { ENTER: "Entrée", TAB: "Tabulation", ESC: "Échap", BACKSPACE: "Retour arrière", DELETE: "Supprimer", UP: "Haut", DOWN: "Bas", LEFT: "Gauche", RIGHT: "Droite", SPACE: "Espace" };
export function ScreenArguments({ tool, arguments: a = {} }) {
  if (tool === "computer_screenshot") return <p className="text-sm">Capture de l’écran principal, sans saisie ni clic.</p>;
  if (tool === "computer_type") return <div><p className="text-sm">Texte exact à saisir :</p><pre className="gb-engine-data">{a.text}</pre></div>;
  if (tool === "computer_key") return <p className="text-sm">Touche : {screenKeyLabels[a.key] || "Inconnue"} · Peut déclencher une validation ou une suppression.</p>;
  if (tool === "computer_scroll") return <p className="text-sm">Défilement vertical : {a.delta_y} pixels.</p>;
  return <p className="text-sm">Position : {a.x}, {a.y} (points depuis le coin supérieur gauche).{tool === "computer_click" && ` Bouton ${a.button === "right" ? "droit" : "gauche"}.`}</p>;
}
export function ScreenCard({ message }) {
  const { call, outcome, screen_image: image } = message.payload;
  const a = call.arguments || {};
  const action = call.name === "computer_type" ? a.text
    : call.name === "computer_key" ? (screenKeyLabels[a.key] || "Touche inconnue")
    : ["computer_move", "computer_click"].includes(call.name) ? `${a.x}, ${a.y}${call.name === "computer_click" ? (a.button === "right" ? " · bouton droit" : " · bouton gauche") : ""}`
    : call.name === "computer_scroll" ? `${a.delta_y} pixels` : "Écran principal";
  const reason = outcome?.status === "failed" ? outcome.reason : null;
  return <article className="gb-message gb-engine-card screen-action-card" data-message-id={message.id} data-card-kind="tool">
    <div className="screen-action-heading"><strong>{screenTools[call.name]}</strong><span>{reason ? "Non exécutée ou interrompue" : outcome ? "Terminée" : "Accord requis"}</span></div>
    <figure>{typeof image === "string" && image.startsWith("data:image/png;base64,") ? <button type="button" className="screen-thumbnail" aria-label="Agrandir la capture de l’écran" onClick={() => window.EzerbotScreen?.preview(image)}><img src={image} alt="Capture de l’écran après l’action" /></button> : <span aria-label="Capture non disponible">▧</span>}<figcaption>{action}</figcaption></figure>
    {outcome?.output?.fixture && <p>Écran fictif · événement enregistré, aucun effet sur le Mac.</p>}
    {reason && <p role="status">{reason}</p>}
    {!outcome && <p>Confirmez dans la carte ci-dessous. Chaque geste peut déclencher un achat, un envoi ou une suppression.</p>}
    {reason && /permission|Accessibilité|Enregistrement|Surveillance/i.test(reason) && <div className="screen-actions"><button className="gb-retry" onClick={() => window.EzerbotScreen?.pane("accessibility")}>Ouvrir Accessibilité</button><button className="gb-retry" onClick={() => window.EzerbotScreen?.pane("recording")}>Ouvrir Enregistrement de l’écran</button></div>}
  </article>;
}
