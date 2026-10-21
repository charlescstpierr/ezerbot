import { useState } from "react";
import { Ellipsis } from "lucide-react";
import { messageActions, REACTIONS } from "./message-actions.js";

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
  return (
    <div className="gb-message-menu" data-message-id={message.id} data-open={isOpen ? "true" : "false"}>
      {actions.reaction && <span className="gb-reaction" aria-label="Réaction">{actions.reaction}</span>}
      {actions.status && <span className="gb-status gb-status-alert" role="status">{actions.status}</span>}
      <button type="button" className="gb-icon-button" aria-label="Actions du message" aria-haspopup="menu" aria-expanded={isOpen} onClick={() => setOpen(!isOpen)}>
        <Ellipsis />
      </button>
      {isOpen && (
        <div className="gb-message-menu-panel" role="menu" aria-label="Actions du message">
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
      )}
    </div>
  );
}
