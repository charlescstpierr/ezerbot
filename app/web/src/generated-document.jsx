import { useEffect, useRef, useState } from "react";
import { formatAttachmentSize } from "./attachment-presentation.js";

export const documentActionLabel = (action) => ({ open: "Ouvrir le document", reveal: "Afficher dans le Finder", save: "Enregistrer une copie" })[action] || "Action sur le document";

function SaveDialog({ file, pending, error, onSave, onClose }) {
  const dialog = useRef(null);
  const [path, setPath] = useState("");
  useEffect(() => { dialog.current.showModal(); }, []);
  return <dialog className="gb-file-save" ref={dialog} onCancel={onClose} aria-labelledby={`save-${file.id}`}>
    <form onSubmit={(e) => { e.preventDefault(); onSave(path); }}>
      <h3 id={`save-${file.id}`}>Enregistrer sous…</h3>
      <p>Choisissez le chemin complet d’un nouveau fichier sur votre Mac. Aucun fichier existant ne sera remplacé.</p>
      <label>Chemin de destination<input name="document-destination" autoFocus required value={path} onChange={(e) => setPath(e.target.value)} placeholder={`/Users/votre-nom/Desktop/${file.name}`} /></label>
      <p className="gb-file-hint">Une carte d’approbation vous demandera de confirmer l’écriture.</p>
      {error && <p className="gb-file-error" role="alert">{error}</p>}
      <div className="gb-file-actions"><button type="button" className="gb-retry" disabled={pending} onClick={onClose}>Annuler</button><button type="submit" className="gb-retry gb-retry-primary" disabled={pending || !path.startsWith("/")}>Demander l’enregistrement</button></div>
    </form>
  </dialog>;
}

export function GeneratedDocument({ file, message }) {
  const [saving, setSaving] = useState(false);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [previewError, setPreviewError] = useState(false);
  async function act(action, path = null) {
    setPending(true); setError(""); setNotice("");
    try {
      const panels = window.GrokbotPanels;
      const conversation = panels.state.conversations.find((c) => c.messages?.some((m) => m.id === message.id));
      if (!conversation) throw new Error("La conversation de ce document est introuvable.");
      const receipt = await panels.api("/api/generated/action", { id: file.id, action, path, conversation_id: conversation.id });
      setSaving(false);
      setNotice("Demande enregistrée. Confirmez l’action dans la carte d’approbation (ou consultez sa règle « Toujours »).");
      await panels.refresh();
      panels.openConversation(receipt.conversation_id);
    } catch (e) { setError(e.message || "L’action sur le document a échoué."); }
    finally { setPending(false); }
  }
  return <article className="gb-message gb-engine-card gb-generated-file" data-message-id={message.id} data-card-kind="generated-document">
    <div className="gb-file-heading"><strong title={file.name}>{file.name}</strong><span>Document créé</span></div>
    <p className="gb-file-metadata">{file.format.toUpperCase()} · {formatAttachmentSize(file.byte_length)}{file.format === "pptx" && ` · ${file.pages} diapositive${file.pages > 1 ? "s" : ""}`}{file.format === "pdf" && ` · ${file.pages} page${file.pages > 1 ? "s" : ""}`}</p>
    {previewError ? <p className="gb-file-hint">Aperçu indisponible. Le fichier peut toujours être ouvert ou enregistré.</p> : <img className={`gb-file-preview${file.format === "svg" ? " gb-file-diagram" : ""}`} src={file.preview_url} alt={file.format === "svg" ? `Diagramme : ${file.name}` : `Aperçu de la première ${file.format === "pptx" ? "diapositive" : "page"} : ${file.name}`} onError={() => setPreviewError(true)} />}
    <div className="gb-file-actions"><button type="button" className="gb-retry gb-retry-primary" disabled={pending} onClick={() => act("open")}>Ouvrir</button><button type="button" className="gb-retry" disabled={pending} onClick={() => act("reveal")}>Afficher dans le Finder</button><button type="button" className="gb-retry" disabled={pending} onClick={() => { setError(""); setSaving(true); }}>Enregistrer sous…</button></div>
    <p className="gb-file-hint">Conservé dans le stockage d’Ezerbot. Les actions sur le Mac nécessitent votre approbation.</p>
    {notice && <p className="gb-file-hint" role="status">{notice}</p>}
    {error && <p className="gb-file-error" role="alert">{error}</p>}
    {saving && <SaveDialog file={file} pending={pending} error={error} onSave={(path) => act("save", path)} onClose={() => setSaving(false)} />}
  </article>;
}

export function DocumentTool({ message, run }) {
  const { call, outcome, approval_status, authorized_by } = message.payload;
  const labels = { denied: "Action refusée", expired: "Approbation expirée", cancelled: "Action annulée" };
  const label = outcome?.status === "failed" ? labels[approval_status] || "Échec"
    : outcome?.status === "done" ? "Terminé" : run?.status === "awaiting_approval" ? "Accord requis dans la carte d’approbation" : "En attente";
  const title = call.name === "create_document" ? "Création du document" : documentActionLabel(call.arguments?.action);
  return <article className="gb-message gb-engine-card gb-document-status" data-message-id={message.id} data-card-kind="tool" data-local-state={outcome?.status === "failed" ? "failed" : !outcome && run?.status === "running" ? "running" : ""}>
    <strong>{title}</strong><p role={outcome?.status === "failed" ? "alert" : "status"}>{outcome?.reason || outcome?.output?.message || label}</p>
    {call.arguments?.path && <p className="gb-file-path">{call.arguments.path}</p>}
    {authorized_by?.kind === "rule" && <p className="gb-file-hint">Action autorisée par une règle « Toujours ».</p>}
  </article>;
}
