import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { Streamdown } from "streamdown";
import { createMermaidPlugin } from "@streamdown/mermaid";
import { createMathPlugin } from "@streamdown/math";
import * as Tooltip from "@radix-ui/react-tooltip";
import { BookOpen, Check, Clock, RefreshCw, Square, TriangleAlert } from "lucide-react";
import { documentIconPaths, formatAttachmentSize } from "./attachment-presentation.js";
import { code } from "./code.js";
import { useAppearance } from "./appearance.js";
import { CopyIconButton } from "./ui.jsx";
import { Card as FallbackCard } from "./cards/fallback.jsx";
import { orderThread } from "./thread-order.js";
import { splitMentionText } from "./mentions.js";
import { isTeachMessage } from "./teach.js";

// A capability adds only cards/<name>.jsx exporting `kind` and `Card`.
// Cards receive message, busy, and onAction(action, detail), routed by the shell.
const cards = new Map(Object.values(import.meta.glob("./cards/*.jsx", { eager: true }))
  .filter((module) => module.kind && module.Card)
  .map((module) => [module.kind, module.Card]));
const chatKinds = new Set([undefined, null, "message", "submission", "reply"]);

const pendingStates = new Set(["queued", "dispatching", "unknown"]);
const math = createMathPlugin({ singleDollarTextMath: true });

const translations = {
  copyCode: "Copier le code",
  downloadFile: "Télécharger le fichier",
  downloadDiagram: "Télécharger le diagramme",
  downloadDiagramAsSvg: "Télécharger en SVG",
  downloadDiagramAsPng: "Télécharger en PNG",
  downloadDiagramAsMmd: "Télécharger le code Mermaid",
  viewFullscreen: "Afficher en plein écran",
  exitFullscreen: "Quitter le plein écran",
  zoomIn: "Agrandir",
  zoomOut: "Réduire",
  resetView: "Réinitialiser la vue",
  copyTable: "Copier le tableau",
  copyTableAsMarkdown: "Copier en Markdown",
  copyTableAsCsv: "Copier en CSV",
  copyTableAsTsv: "Copier en TSV",
  downloadTable: "Télécharger le tableau",
  downloadTableAsCsv: "Télécharger en CSV",
  downloadTableAsMarkdown: "Télécharger en Markdown",
  imageNotAvailable: "Image non disponible",
  downloadImage: "Télécharger l’image",
  openExternalLink: "Ouvrir ce lien externe ?",
  externalLinkWarning: "Vous allez ouvrir un site externe.",
  close: "Fermer",
  copyLink: "Copier le lien",
  copied: "Copié",
  openLink: "Ouvrir le lien",
};

function mermaidPlugin(appearance) {
  return createMermaidPlugin({
    config: {
      theme: "base",
      fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
      themeVariables: {
        darkMode: appearance.dark,
        background: appearance.canvas,
        primaryColor: appearance.pane,
        primaryTextColor: appearance.text,
        primaryBorderColor: appearance.accent,
        secondaryColor: appearance.soft,
        secondaryTextColor: appearance.text,
        secondaryBorderColor: appearance.muted,
        tertiaryColor: appearance.canvas,
        tertiaryTextColor: appearance.text,
        tertiaryBorderColor: appearance.muted,
        lineColor: appearance.muted,
        textColor: appearance.text,
        mainBkg: appearance.pane,
        nodeBorder: appearance.accent,
        clusterBkg: appearance.soft,
        clusterBorder: appearance.muted,
        edgeLabelBackground: appearance.canvas,
        actorBkg: appearance.pane,
        actorBorder: appearance.accent,
        signalColor: appearance.muted,
        signalTextColor: appearance.text,
        noteBkgColor: appearance.soft,
        noteTextColor: appearance.text,
        noteBorderColor: appearance.muted,
        fontSize: "13px",
      },
    },
  });
}

function Mascot({ create }) {
  const mount = useRef(null);
  useEffect(() => {
    const node = create?.();
    if (node) mount.current.replaceChildren(node);
  }, [create]);
  return <span className="gb-avatar" ref={mount} aria-hidden="true" />;
}

function Timestamp({ value }) {
  const parsed = new Date(value);
  const date = Number.isNaN(parsed.getTime()) ? new Date() : parsed;
  return <time dateTime={date.toISOString()}>{date.toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" })}</time>;
}

function PendingCard({ message, busy, onRetry }) {
  const queued = message.state === "queued";
  return (
    <div className="gb-pending">
      <Clock />
      <div className="gb-pending-copy">
        <strong>{queued ? "Ce message attend son envoi." : "Cet envoi attend confirmation."}</strong>
        <p>{queued ? "Votre message est sauvegardé. Reprenez l’envoi pour poursuivre." : "Votre message est sauvegardé. Vérifiez l’envoi pour poursuivre."}</p>
      </div>
      <button type="button" className="gb-retry" disabled={busy} onClick={() => onRetry(message)}>
        <RefreshCw />
        {queued ? "Reprendre l’envoi" : "Vérifier l’envoi"}
      </button>
    </div>
  );
}

function MessageAttachments({ message, align = "end" }) {
  return message.attachments?.length > 0 && (
        <div className={`flex flex-wrap items-start ${align === "start" ? "justify-start" : "justify-end"} gap-2`} aria-label="Pièces jointes">
          {message.attachments.map((attachment, index) => {
            const id = typeof attachment === "string" ? attachment : attachment.id || attachment.attachment_id;
            if (!id) return null;
            if (attachment.kind === "document") return <div key={`${id}-${index}`} className="gb-document-chip">
              <svg width="22" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">{documentIconPaths.map((d) => <path key={d} d={d} />)}</svg>
              <div><strong title={attachment.name}>{attachment.name}</strong>
                <span>{formatAttachmentSize(attachment.byte_length)} · {attachment.page_label}</span>
                {attachment.note && <span className="gb-document-note">{attachment.note}</span>}
              </div>
            </div>;
            const description = attachment.caption || attachment.name || "Image jointe";
            const url = `/api/attachments/${encodeURIComponent(id)}`;
            return <img key={`${id}-${index}`} className="rounded-lg object-cover" src={url} width="160" height="120" alt={description} loading="lazy" />;
          })}
        </div>
  );
}

function DelegationInstruction({ message, createMascot, botName }) {
  return (
    <article className="gb-message gb-assistant gb-delegation-instruction" data-message-id={message.id} aria-label={`Consigne de ${botName}`}>
      <Mascot create={createMascot} />
      <div className="gb-content">
        <header className="gb-meta"><strong>Consigne de {botName}</strong><Timestamp value={message.created_at} /></header>
        {message.body && <p className="gb-instruction-text">{message.body}</p>}
        <MessageAttachments message={message} align="start" />
      </div>
    </article>
  );
}

function UserMessage({ message, busy, onCopy, onRetry }) {
  if (isTeachMessage(message)) return (
    <article className="gb-message gb-teach-marker" data-message-id={message.id} aria-label="Apprentissage depuis cette conversation">
      <div className="gb-meta"><BookOpen aria-hidden="true" /><span>Apprendre ceci comme skill</span><Timestamp value={message.created_at} /></div>
      {pendingStates.has(message.state) && (!busy || message.state === "unknown") && <PendingCard message={message} busy={busy} onRetry={onRetry} />}
      {message.state === "refused" && <p className="gb-status-alert">Demande d’apprentissage non envoyée.</p>}
    </article>
  );
  const followUp = message.payload?.source === "follow_up";
  return (
    <article className="gb-message gb-user" data-message-id={message.id} aria-label={followUp ? "Suivi de votre part" : undefined}>
      <header className="gb-meta"><strong>Vous</strong><Timestamp value={message.created_at} /></header>
      {followUp && <span className="gb-followup-label">Suivi</span>}
      {message.body && <div className="gb-bubble">{splitMentionText(message.body, message.payload?.mentions || message.mentions).map((part, index) => part.mention
        ? <span className="gb-mention" key={index} data-bot-mention={part.mention.id} title={part.mention.id === "@everyone" ? "Tous les membres à l’envoi" : `Bot : ${part.mention.id}`}>{part.text}</span>
        : part.text)}</div>}
      <MessageAttachments message={message} />
      <footer className="gb-actions">
        {["accepted", "echoed"].includes(message.state) && <span className="gb-status"><Check />{followUp ? "Enregistré" : "Envoyé"}</span>}
        {["queued", "dispatching"].includes(message.state) && <span className="gb-status">Envoi en cours…</span>}
        {message.state === "refused" && <span className="gb-status gb-status-alert"><TriangleAlert />Envoi refusé</span>}
        <CopyIconButton text={message.body} label="Copier votre message" onCopy={onCopy} />
      </footer>
      {pendingStates.has(message.state) && (!busy || message.state === "unknown") && <PendingCard message={message} busy={busy} onRetry={onRetry} />}
    </article>
  );
}

function Dots() {
  return <div className="gb-dots" aria-hidden="true"><i /><i /><i /></div>;
}

function ReplyProblem({ message, actions }) {
  const limit = message.error_kind === "limit";
  return (
    <div className="gb-pending gb-problem" role="alert">
      <TriangleAlert />
      <div className="gb-pending-copy">
        <strong>{limit ? "Limite d’utilisation atteinte" : "La réponse n’a pas abouti"}</strong>
        <p>{message.error}</p>
      </div>
      {/* OpenAI asks for "Manage usage" as the primary action on a usage limit. */}
      {limit && <a className="gb-retry gb-retry-primary" href={actions.usageUrl} target="_blank" rel="noreferrer">Gérer l’utilisation</a>}
      {(message.payload?.turn_id || actions.isGroup) && message.error_kind !== "signin"
        ? <p className="gb-reply-advice">Envoyez un nouveau message pour demander une nouvelle réponse.</p>
        : actions.isTask ? <p className="gb-reply-advice">Poursuivez dans la conversation principale.</p>
        : message.error_kind === "context_budget" ? null : message.error_kind === "signin"
        ? <button type="button" className="gb-retry" onClick={actions.onSignIn}>Se reconnecter</button>
        : <button type="button" className="gb-retry" onClick={() => actions.onRegenerate(message)}><RefreshCw />Relancer</button>}
    </div>
  );
}

function TurnLabel({ message }) {
  return message.payload?.turn_id && <small title={`Chaîne : ${message.payload.root_id}`}>{message.payload.parent_turn_id ? "Échange entre bots" : "Tour de groupe"}</small>;
}

function AssistantMessage({ message, plugins, appearance, portal, actions, createMascot, botName }) {
  const content = useRef(null);
  const streaming = message.state === "streaming";
  const queued = message.payload?.turn_status === "queued";
  const group = Boolean(message.payload?.turn_id || actions.isGroup);
  const copyText = "copyText" in message ? message.copyText : message.body;
  useEffect(() => {
    const root = content.current;
    if (!root) return;
    let active = true;
    const measure = () => {
      if (!active) return;
      for (const body of root.querySelectorAll('[data-streamdown="code-block-body"]')) {
        const overflow = body.scrollWidth > body.clientWidth + 1;
        body.closest('[data-streamdown="code-block"]')?.toggleAttribute("data-gb-overflow", overflow);
        body.tabIndex = overflow ? 0 : -1;
        if (overflow) body.setAttribute("aria-label", "Code — faites défiler horizontalement pour voir la suite");
        else body.removeAttribute("aria-label");
        resize.observe(body);
      }
    };
    const resize = new ResizeObserver(measure);
    const mutations = new MutationObserver(measure);
    resize.observe(root);
    mutations.observe(root, { childList: true, subtree: true, characterData: true });
    document.fonts?.ready.then(measure);
    measure();
    return () => { active = false; resize.disconnect(); mutations.disconnect(); };
  }, [message.body, appearance.key]);
  if (!message.body?.trim() && !message.notice && !["streaming", "failed", "stopped"].includes(message.state)) return null;
  return (
    <article className="gb-message gb-assistant" data-message-id={message.id} aria-busy={streaming && !queued}>
      {message.continuation ? <span className="gb-avatar" aria-hidden="true" /> : <Mascot create={createMascot} />}
      <div className="gb-content" ref={content}>
        {!message.continuation && <header className="gb-meta"><strong>{botName}</strong><Timestamp value={message.created_at} /><TurnLabel message={message} /></header>}
        {message.body
          ? (
            <Streamdown key={appearance.key} className="gb-markdown" mode={streaming ? "streaming" : "static"} isAnimating={streaming} plugins={plugins} shikiTheme={code.getThemes()} translations={translations} portal={portal}>
              {message.body}
            </Streamdown>
          )
          : queued ? <span className="gb-queued"><Clock />{message.payload?.wait_reason === "models" ? "En attente des modèles ChatGPT" : message.payload?.wait_reason === "signin" ? "En attente de connexion" : "En attente"}</span> : streaming && <Dots />}
        {message.state === "failed" && <ReplyProblem message={message} actions={actions} />}
        {message.notice && (!queued || message.payload?.parent_turn_id) && <p className="gb-notice" title={message.payload?.root_id ? `Chaîne : ${message.payload.root_id}` : undefined}>{message.notice}</p>}
        {!streaming && (copyText || message.state === "stopped") && (
          <footer className="gb-actions">
            {copyText && <CopyIconButton text={copyText} label="Copier la réponse" onCopy={actions.onCopy} />}
            {message.state === "stopped" && <><span className={group ? "gb-status gb-group-stopped" : "gb-status"}>{message.payload?.never_started ? <Clock /> : <Square className={group ? "gb-stop-solid" : undefined} />}{actions.isTask ? "Sous-tâche annulée" : message.payload?.never_started ? "Tour annulé avant de commencer" : "Réponse arrêtée"}</span>{!group && !actions.isTask && <button type="button" className="gb-text-button" onClick={() => actions.onRegenerate(message)}><RefreshCw />Relancer</button>}</>}
          </footer>
        )}
        {group && message.state === "stopped" && <p className="gb-reply-advice">Envoyez un nouveau message pour reprendre.</p>}
      </div>
    </article>
  );
}

function Typing({ createMascot, botName }) {
  return (
    <article className="gb-message gb-assistant gb-typing" aria-label={`${botName} prépare sa réponse`}>
      <Mascot create={createMascot} />
      <Dots />
    </article>
  );
}

export function Thread({ messages, busy = false, onCopy, onRetry, onRegenerate, onSignIn, onCardAction, usageUrl, createMascot, botName = "Ezerbot", bots = [], createBotMascot, isGroup = false, isTask = false, onLayout }) {
  const appearance = useAppearance();
  const overlay = useRef(null);
  const portal = useRef(() => overlay.current).current;
  const plugins = useMemo(() => ({ code, math, mermaid: mermaidPlugin(appearance) }), [appearance]);
  const actions = useMemo(() => ({ onCopy, onRegenerate, onSignIn, usageUrl, isGroup, isTask }), [onCopy, onRegenerate, onSignIn, usageUrl, isGroup, isTask]);
  const ordered = useMemo(() => orderThread(messages), [messages]);
  const last = ordered[ordered.length - 1];
  useLayoutEffect(() => { onLayout?.(); }, [ordered, onLayout]);
  return (
    <Tooltip.Provider delayDuration={350}>
      <div className="gb-thread">
        {ordered.map((message) => {
          const delegated = message.payload?.source === "delegation";
          const author = bots.find((bot) => bot.id === (delegated ? message.payload.from_bot : message.payload?.bot_id));
          const authorName = author?.name || (delegated ? "bot parent" : botName);
          const authorMascot = author && createBotMascot ? createBotMascot(author) : createMascot;
          if (message.kind === "turn-heading") return (
            <div className="gb-message gb-assistant gb-turn-heading" key={message.id}>
              <Mascot create={authorMascot} />
              <header className="gb-meta"><strong>{authorName}</strong><Timestamp value={message.created_at} /><TurnLabel message={message} /></header>
            </div>
          );
          if (delegated) return <DelegationInstruction key={message.id} message={message} createMascot={authorMascot} botName={authorName} />;
          const Card = cards.get(message.kind) || (!chatKinds.has(message.kind) ? FallbackCard : null);
          if (Card) return <div className="gb-card-slot" key={message.id}><Card message={message} busy={busy} botName={authorName} isGroup={isGroup} onAction={(action, detail) => onCardAction?.(message, action, detail)} onCardAction={onCardAction} /></div>;
          return message.role === "user"
            ? <UserMessage key={message.id} message={message} busy={busy} onCopy={onCopy} onRetry={onRetry} />
            : <AssistantMessage key={message.key || message.id} message={message} plugins={plugins} appearance={appearance} portal={portal} actions={actions} createMascot={authorMascot} botName={authorName} />;
        })}
        {busy && last?.role === "user" && chatKinds.has(last.kind) && <Typing createMascot={createMascot} botName={botName} />}
        <div className="gb-overlay" ref={overlay} />
      </div>
    </Tooltip.Provider>
  );
}
