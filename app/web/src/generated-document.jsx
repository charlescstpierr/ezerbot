import { useEffect, useRef, useState } from "react";
import { formatAttachmentSize } from "./attachment-presentation.js";
import { documentToolView } from "./generated-presentation.js";

function SaveDialog({ file, pending, error, onSave, onClose }) {
  const dialog = useRef(null);
  const input = useRef(null);
  const [path, setPath] = useState("");
  useEffect(() => {
    dialog.current.showModal();
    let active = true;
    window.GrokbotPanels.api(`/api/generated/${file.id}/save-path`).then((value) => {
      if (active && typeof value.path === "string" && !input.current?.value) {
        setPath(value.path);
        requestAnimationFrame(() => {
          input.current?.focus();
          input.current?.setSelectionRange(value.path.lastIndexOf("/") + 1, value.path.lastIndexOf("."));
          if (input.current) input.current.scrollLeft = input.current.scrollWidth;
        });
      }
    }).catch(() => {});
    return () => { active = false; };
  }, [file.id]);
  return <dialog className="gb-file-save" ref={dialog} onClose={onClose} aria-labelledby={`save-${file.id}`}>
    <form onSubmit={(e) => { e.preventDefault(); onSave(path); }}>
      <h3 id={`save-${file.id}`}>Enregistrer sous…</h3>
      <p>Choisissez le chemin complet d’un nouveau fichier sur votre Mac. Aucun fichier existant ne sera remplacé.</p>
      <label>Chemin de destination<input ref={input} name="document-destination" autoFocus required value={path} onChange={(e) => setPath(e.target.value)} /></label>
      <p className="gb-file-hint">Une carte d’approbation vous demandera de confirmer l’écriture. L’extension doit rester .{file.format}.</p>
      {error && <p className="gb-file-error" role="alert">{error}</p>}
      <div className="gb-file-actions dialog-footer"><button type="button" className="gb-retry" disabled={pending} onClick={onClose}>Annuler</button><button type="submit" className="gb-retry gb-retry-primary" disabled={pending || !path.startsWith("/")}>Demander l’enregistrement</button></div>
    </form>
  </dialog>;
}

function PreviewDialog({ file, onClose }) {
  const dialog = useRef(null);
  useEffect(() => { dialog.current.showModal(); }, []);
  return <dialog className="gb-file-save gb-file-zoom" ref={dialog} onClose={onClose} aria-label={`Aperçu : ${file.name}`}>
    <div className="gb-local-header"><strong>{file.name}</strong><button type="button" className="gb-retry" autoFocus onClick={onClose}>Fermer</button></div>
    <div className={`gb-file-zoom-content${file.format === "svg" ? " gb-file-zoom-diagram" : ""}`}><img src={file.preview_url} alt={`Aperçu agrandi : ${file.name}`} /></div>
  </dialog>;
}

export function GeneratedDocument({ file, message }) {
  const [saving, setSaving] = useState(false);
  const [zoom, setZoom] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [previewError, setPreviewError] = useState(false);
  async function act(action, path = null) {
    setPending(true); setError("");
    try {
      const panels = window.GrokbotPanels;
      const conversation = panels.state.conversations.find((c) => c.messages?.some((m) => m.id === message.id));
      if (!conversation) throw new Error("La conversation de ce document est introuvable.");
      const receipt = await panels.api("/api/generated/action", { id: file.id, action, path, conversation_id: conversation.id });
      setSaving(false);
      await panels.refresh();
      panels.openConversation(receipt.conversation_id);
    } catch (e) { setError(e.message || "L’action sur le document a échoué."); }
    finally { setPending(false); }
  }
  return <article className="gb-message gb-engine-card gb-generated-file" data-message-id={message.id} data-card-kind="generated-document">
    <div className="gb-file-heading"><strong title={file.name}>{file.name}</strong><span>Document créé</span></div>
    <p className="gb-file-metadata">{file.format.toUpperCase()} · {formatAttachmentSize(file.byte_length)}{file.format === "pptx" && ` · ${file.pages} diapositive${file.pages > 1 ? "s" : ""}`}{file.format === "pdf" && ` · ${file.pages} page${file.pages > 1 ? "s" : ""}`}</p>
    {previewError ? <p className="gb-file-hint">Aperçu indisponible. Le fichier peut toujours être ouvert ou enregistré.</p> : <button type="button" className={`gb-file-preview-button${["csv", "xlsx"].includes(file.format) ? " gb-file-preview-table" : ""}`} onClick={() => setZoom(true)} aria-label={`Agrandir l’aperçu de ${file.name}`}><img className={`gb-file-preview${file.format === "svg" ? " gb-file-diagram" : ""}`} src={file.preview_url} alt={file.format === "svg" ? `Diagramme : ${file.name}` : `Aperçu du document : ${file.name}`} onError={() => setPreviewError(true)} /></button>}
    <div className="gb-file-actions"><button type="button" className="gb-retry gb-retry-primary" disabled={pending} onClick={() => act("open")}>Ouvrir</button><button type="button" className="gb-retry" disabled={pending} onClick={() => act("reveal")}>Afficher dans le Finder</button><button type="button" className="gb-retry" disabled={pending} onClick={() => { setError(""); setSaving(true); }}>Enregistrer sous…</button></div>
    <p className="gb-file-hint">Conservé dans le stockage d’Ezerbot. Les actions sur le Mac nécessitent votre approbation.</p>
    {error && !saving && <p className="gb-file-error" role="alert">{error}</p>}
    {saving && <SaveDialog file={file} pending={pending} error={error} onSave={(path) => act("save", path)} onClose={() => { setSaving(false); setError(""); }} />}
    {zoom && <PreviewDialog file={file} onClose={() => setZoom(false)} />}
  </article>;
}

export function DocumentTool({ message, run }) {
  const { call, outcome, authorized_by } = message.payload;
  const view = documentToolView(message.payload, run);
  return <article className="gb-message gb-engine-card gb-local-card gb-document-status" data-message-id={message.id} data-card-kind="tool" data-local-state={view.state}>
    <div className="gb-local-header"><strong>{view.title}</strong><span className="gb-local-status" data-tone={view.tone}>{view.label}</span></div>
    {view.failure ? <p className="gb-local-failure" data-tone={view.tone} role="alert">{view.summary}</p> : <p role="status">{outcome?.output?.message || (view.state === "running" ? (call.name === "create_document" ? "Création du document en cours." : "L’action est en cours sur votre Mac.") : view.label === "Accord requis" ? "Confirmez l’action dans la carte d’approbation." : "Le résultat sera affiché ici.")}</p>}
    {typeof call.arguments?.path === "string" && <pre className="gb-local-command">{call.arguments.path}</pre>}
    {view.failure && view.summary !== view.failure.message && <details><summary>Détails techniques</summary><pre className="gb-local-command">{view.failure.message}</pre></details>}
    {authorized_by?.kind === "rule" && <p className="gb-file-hint">Action autorisée par une règle « Toujours ».</p>}
  </article>;
}
