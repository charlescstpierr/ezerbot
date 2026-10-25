import { localResourceURL } from "./local-resource.js";
import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import { documentIconPaths, formatAttachmentSize } from "./attachment-presentation.js";
import { copyGalleryImage, galleryFailureText, galleryImages, galleryKey, saveGalleryImage } from "./image-gallery.js";
import { createMemoPlayer, formatMemoSpeed, startingMemoSpeed } from "./memo-playback.js";

function galleryIO() {
  return {
    fetch: globalThis.fetch?.bind(globalThis),
    showSaveFilePicker: globalThis.showSaveFilePicker?.bind(globalThis),
    native: globalThis.webkit?.messageHandlers?.ezerbotImages,
    clipboard: globalThis.navigator?.clipboard,
  };
}

export function ImageViewer({ images, index, onIndex, onClose }) {
  const dialog = useRef(null);
  const image = images[index];
  const [broken, setBroken] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [pending, setPending] = useState(false);
  useEffect(() => { setBroken(false); setError(""); setNotice(""); }, [image?.url]);
  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (!node.open) node.showModal();
    node.focus();
    return () => { if (node.open) node.close(); };
  }, []);
  if (!image) return null;
  async function run(action, fallback) {
    setPending(true); setError(""); setNotice("");
    try {
      const result = await action();
      if (result) setNotice(result);
    } catch (caught) {
      const text = galleryFailureText(caught, fallback);
      if (text) setError(text);
    } finally { setPending(false); }
  }
  function onKeyDown(event) {
    const next = galleryKey(event.key, index, images.length);
    if (next === null) {
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
      return;
    }
    event.preventDefault();
    onIndex(next);
  }
  return (
    <dialog className="gb-image-viewer" ref={dialog} aria-label="Visionneuse" onKeyDown={onKeyDown} onClose={onClose} onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="gb-image-viewer-bar">
        <p className="gb-image-count">{index + 1} sur {images.length}</p>
        <button type="button" className="gb-image-close" onClick={onClose} aria-label="Fermer"><X /></button>
      </div>
      <div className="gb-image-stage">
        {images.length > 1 && <button type="button" className="gb-image-nav" aria-label="Image précédente" onClick={() => onIndex(galleryKey("ArrowLeft", index, images.length))}><ChevronLeft /></button>}
        {broken
          ? <p className="gb-image-missing" role="alert">Image non disponible</p>
          : <img src={localResourceURL(image.url)} alt={image.caption} onError={() => setBroken(true)} />}
        {images.length > 1 && <button type="button" className="gb-image-nav" aria-label="Image suivante" onClick={() => onIndex(galleryKey("ArrowRight", index, images.length))}><ChevronRight /></button>}
      </div>
      <figcaption className="gb-image-caption">{image.caption}</figcaption>
      <div className="gb-image-actions">
        <button type="button" className="gb-image-action" aria-label="Enregistrer l’image…" disabled={pending} onClick={() => run(() => saveGalleryImage(image, galleryIO()), "L’image n’a pas pu être enregistrée.")}>Enregistrer l’image…</button>
        <button type="button" className="gb-image-action" aria-label="Copier l’image" disabled={pending} onClick={() => run(async () => { await copyGalleryImage(image, galleryIO()); return "Image copiée."; }, "L’image n’a pas pu être copiée.")}>Copier l’image</button>
      </div>
      {notice && <p className="gb-image-notice" role="status">{notice}</p>}
      {error && <p className="gb-image-error" role="alert">{error}</p>}
    </dialog>
  );
}

function VoiceMemo({ attachment, message, memoSpeed }) {
  const id = attachment.id;
  const audio = useRef(null);
  const player = useRef(null);
  const [speed, setSpeed] = useState(() => startingMemoSpeed(memoSpeed));
  const [text, setText] = useState(() => formatMemoSpeed(startingMemoSpeed(memoSpeed)));
  const [refusal, setRefusal] = useState("");
  const [notice, setNotice] = useState("");
  const words = Array.isArray(attachment.words) ? attachment.words.filter((word) => typeof word?.text === "string" && word.text) : [];
  const playerFor = () => {
    if (!player.current || player.current.element !== audio.current) {
      player.current = Object.assign(createMemoPlayer(audio.current, speed), { element: audio.current });
    }
    return player.current;
  };
  useEffect(() => { if (audio.current) playerFor().apply(); }, []);
  const commitSpeed = () => {
    if (!audio.current) return;
    const result = playerFor().setSpeed(text);
    setRefusal(result.ok ? "" : result.message);
    setSpeed(result.speed);
    setText(formatMemoSpeed(result.speed));
  };
  const jump = (word) => {
    const result = playerFor().jump(word);
    setNotice(result.message);
  };
  return (
    <div className="gb-voice-memo">
      <strong>Mémo vocal · {Math.ceil((attachment.duration_ms || 0) / 1000)} s</strong>
      <audio ref={audio} controls preload="metadata" src={localResourceURL(`/api/voice/audio/${encodeURIComponent(id)}`)} aria-label="Écouter le mémo vocal" onLoadedMetadata={() => playerFor().apply()} />
      <label className="gb-memo-speed">
        <span>Vitesse</span>
        <input type="text" inputMode="decimal" value={text} aria-label="Vitesse de lecture du mémo, de 0,5 à 2" aria-invalid={refusal ? "true" : undefined} data-speed={speed}
          onChange={(event) => setText(event.target.value)} onBlur={commitSpeed}
          onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); commitSpeed(); } }} />
        <span aria-hidden="true">×</span>
      </label>
      {refusal && <p className="gb-memo-refusal" role="alert">{refusal}</p>}
      {words.length > 0
        ? (
          <details className="gb-memo-transcript" open>
            <summary>Transcription</summary>
            <p className="gb-memo-words" lang={attachment.language || undefined}>
              {words.map((word, index) => (
                <span key={index}>
                  {index > 0 && " "}
                  {Number.isFinite(word.start_ms)
                    ? <button type="button" className="gb-memo-word" data-start-ms={word.start_ms} title={`Lire à partir de ce mot (${(word.start_ms / 1000).toFixed(1).replace(".", ",")} s)`} onClick={() => jump(word)}>{word.text}</button>
                    : <span className="gb-memo-word is-untimed" title="Pas d’horodatage pour ce mot" onClick={() => jump(word)}>{word.text}</span>}
                </span>
              ))}
            </p>
          </details>
        )
        : attachment.transcript?.trim() && !message.body?.includes(attachment.transcript.trim()) && <details><summary>Transcription</summary><p>{attachment.transcript}</p></details>}
      {notice && <p className="gb-memo-notice" role="status">{notice}</p>}
    </div>
  );
}

function DocumentChip({ attachment }) {
  return (
    <div className="gb-document-chip">
      <svg width="22" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">{documentIconPaths.map((d) => <path key={d} d={d} />)}</svg>
      <div><strong title={attachment.name}>{attachment.name}</strong>
        <span>{formatAttachmentSize(attachment.byte_length)} · {attachment.page_label}</span>
        {attachment.note && <span className="gb-document-note">{attachment.note}</span>}
      </div>
    </div>
  );
}

export function MessageAttachments({ message, align = "end", markdown = false, openIndex = null, memoSpeed }) {
  const images = galleryImages(message, { markdown });
  const others = (message?.attachments || []).filter((attachment) => attachment && typeof attachment === "object" && (attachment.kind === "voice" || attachment.kind === "document"));
  const [index, setIndex] = useState(openIndex);
  if (!images.length && !others.length) return null;
  const count = images.length === 1 || images.length === 2 || images.length === 3 ? String(images.length) : "many";
  const open = index !== null && images.length ? (index % images.length + images.length) % images.length : null;
  return (
    <div className={`gb-message-files${align === "end" ? " is-end" : ""}`} aria-label="Pièces jointes">
      {images.length > 0 && (
        <div className="gb-image-grid" data-count={count} aria-label="Images du message">
          {images.map((image, imageIndex) => (
            <button key={`${image.id}-${imageIndex}`} type="button" className="gb-image-cell" aria-label={`Agrandir : ${image.caption}`} onClick={() => setIndex(imageIndex)}>
              <img src={localResourceURL(image.url)} alt={image.caption} loading="lazy" />
            </button>
          ))}
        </div>
      )}
      {others.map((attachment, attachmentIndex) => attachment.kind === "voice"
        ? <VoiceMemo key={`${attachment.id}-${attachmentIndex}`} attachment={attachment} message={message} memoSpeed={memoSpeed} />
        : <DocumentChip key={`${attachment.id}-${attachmentIndex}`} attachment={attachment} />)}
      {open !== null && <ImageViewer images={images} index={open} onIndex={setIndex} onClose={() => setIndex(null)} />}
    </div>
  );
}
