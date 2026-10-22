import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Ellipsis } from "lucide-react";
import { bindMessageMenuDismiss, layoutMessageMenu, messageActions, readMessageMenuBounds, REACTIONS } from "./message-actions.js";

export function MessageMenu({ message, busy = false, text = "", open = false, expanded, onOpenChange, onCopy, onReact, onResend, onDelete }) {
  const actions = messageActions(message, { busy });
  const [localOpen, setLocalOpen] = useState(open);
  const controlled = expanded !== undefined;
  const isOpen = controlled ? expanded : localOpen;
  const setOpen = (next) => {
    if (controlled) onOpenChange?.(next);
    else setLocalOpen(next);
  };
  const close = () => setOpen(false);
  const closeRef = useRef(close);
  closeRef.current = close;
  const rootRef = useRef(null);
  const panelRef = useRef(null);

  useLayoutEffect(() => {
    if (!isOpen) return;
    const anchor = rootRef.current?.querySelector("[aria-haspopup='menu']");
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    layoutMessageMenu(panel, anchor.getBoundingClientRect(), readMessageMenuBounds(window));
  }, [isOpen, actions.resend, actions.delete, actions.reaction, actions.status]);

  useEffect(() => {
    if (!isOpen) return undefined;
    return bindMessageMenuDismiss(window, () => closeRef.current());
  }, [isOpen]);

  const panel = isOpen && (
    <div ref={panelRef} className="gb-message-menu-panel" role="menu" aria-label="Actions du message">
      <div className="gb-reaction-row" role="group" aria-label="Réaction">
        {REACTIONS.map((emoji) => (
          <button type="button" key={emoji} aria-label={`Réagir ${emoji}`} aria-pressed={actions.reaction === emoji} onClick={() => { onReact?.(message, emoji); close(); }}>
            {emoji}
          </button>
        ))}
      </div>
      <button type="button" role="menuitem" onClick={async () => { await onCopy?.(text); close(); }}>Copier</button>
      {actions.resend && <button type="button" role="menuitem" disabled={busy} onClick={() => { onResend?.(message); close(); }}>Renvoyer</button>}
      {actions.delete && <button type="button" role="menuitem" disabled={busy} onClick={() => { onDelete?.(message); close(); }}>Supprimer</button>}
    </div>
  );
  const host = typeof document !== "undefined" ? document.body : null;
  return (
    <div className="gb-message-menu" ref={rootRef} data-message-id={message.id} data-open={isOpen ? "true" : "false"}>
      {actions.reaction && <span className="gb-reaction" aria-label="Réaction">{actions.reaction}</span>}
      {actions.status && <span className="gb-status gb-status-alert" role="status">{actions.status}</span>}
      <button type="button" className="gb-icon-button" aria-label="Actions du message" aria-haspopup="menu" aria-expanded={isOpen} onClick={() => setOpen(!isOpen)}>
        <Ellipsis />
      </button>
      {panel && (host ? createPortal(panel, host) : panel)}
    </div>
  );
}
