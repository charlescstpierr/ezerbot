export const IMAGE_INDICATOR = "Génération d’une image…";
export const IMAGE_CONNECT = "Connecte ton compte OpenAI";

function imageAttachment(message) {
  return (message.attachments || []).find((item) => typeof item?.url === "string" && String(item.mime_type || "").startsWith("image/"));
}

export function GeneratedImage({ message, onAction }) {
  const outcome = message.payload?.outcome;
  const failed = outcome?.status === "failed";
  const reason = typeof outcome?.reason === "string" ? outcome.reason : message.body;
  if (failed && (reason === IMAGE_CONNECT || message.body === IMAGE_CONNECT)) {
    return <article className="gb-message gb-engine-card gb-generated-image-card" data-message-id={message.id} data-card-kind="tool" data-image-state="connect">
      <h3>{IMAGE_CONNECT}</h3>
      <p className="gb-image-status">La clé du compte sert à générer l’image. Elle reste dans le trousseau.</p>
      <button type="button" className="gb-retry gb-retry-primary" data-action="provider.signin" onClick={() => onAction?.("provider.signin")}>Connecter</button>
    </article>;
  }
  if (outcome?.status === "done") {
    const image = imageAttachment(message);
    return <article className="gb-message gb-engine-card gb-generated-image-card" data-message-id={message.id} data-card-kind="tool" data-image-state="done">
      {image ? <img className="gb-generated-image" src={image.url} alt="Image générée" /> : <p className="gb-image-status">L’image reçue n’est pas utilisable.</p>}
    </article>;
  }
  if (failed) {
    return <article className="gb-message gb-engine-card gb-generated-image-card" data-message-id={message.id} data-card-kind="tool" data-image-state="failed">
      <p className="gb-image-status">{reason || "La génération de l’image a échoué."}</p>
    </article>;
  }
  return <article className="gb-message gb-engine-card gb-generated-image-card" data-message-id={message.id} data-card-kind="tool" data-image-state="pending">
    <p className="gb-image-status">{IMAGE_INDICATOR}</p>
  </article>;
}
