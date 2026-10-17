import { useEffect, useMemo, useRef } from "react";
import { Streamdown } from "streamdown";
import { createMermaidPlugin } from "@streamdown/mermaid";
import { createMathPlugin } from "@streamdown/math";
import * as Tooltip from "@radix-ui/react-tooltip";
import { Check, Clock, RefreshCw, Square, TriangleAlert } from "lucide-react";
import { code } from "./code.js";
import { useAppearance } from "./appearance.js";
import { CopyIconButton } from "./ui.jsx";

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

function UserMessage({ message, busy, onCopy, onRetry }) {
  return (
    <article className="gb-message gb-user" data-message-id={message.id}>
      <header className="gb-meta"><strong>Vous</strong><Timestamp value={message.created_at} /></header>
      <div className="gb-bubble">{message.body}</div>
      <footer className="gb-actions">
        {["accepted", "echoed"].includes(message.state) && <span className="gb-status"><Check />Envoyé</span>}
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
      {message.error_kind === "signin"
        ? <button type="button" className="gb-retry" onClick={actions.onSignIn}>Se reconnecter</button>
        : <button type="button" className="gb-retry" onClick={() => actions.onRegenerate(message)}><RefreshCw />Relancer</button>}
    </div>
  );
}

function AssistantMessage({ message, plugins, appearance, portal, actions, createMascot, botName }) {
  const streaming = message.state === "streaming";
  return (
    <article className="gb-message gb-assistant" data-message-id={message.id} aria-busy={streaming}>
      <Mascot create={createMascot} />
      <div className="gb-content">
        <header className="gb-meta"><strong>{botName}</strong><Timestamp value={message.created_at} /></header>
        {message.body
          ? (
            <Streamdown key={appearance.key} className="gb-markdown" mode={streaming ? "streaming" : "static"} isAnimating={streaming} plugins={plugins} shikiTheme={code.getThemes()} translations={translations} portal={portal}>
              {message.body}
            </Streamdown>
          )
          : streaming && <Dots />}
        {message.state === "failed" && <ReplyProblem message={message} actions={actions} />}
        {message.notice && <p className="gb-notice">{message.notice}</p>}
        {!streaming && (
          <footer className="gb-actions">
            {message.body && <CopyIconButton text={message.body} label="Copier la réponse" onCopy={actions.onCopy} />}
            {message.state === "stopped" && <><span className="gb-status"><Square />Réponse arrêtée</span><button type="button" className="gb-text-button" onClick={() => actions.onRegenerate(message)}><RefreshCw />Relancer</button></>}
          </footer>
        )}
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

export function Thread({ messages, busy = false, onCopy, onRetry, onRegenerate, onSignIn, usageUrl, createMascot, botName = "Grokbot" }) {
  const appearance = useAppearance();
  const overlay = useRef(null);
  const portal = useRef(() => overlay.current).current;
  const plugins = useMemo(() => ({ code, math, mermaid: mermaidPlugin(appearance) }), [appearance]);
  const actions = useMemo(() => ({ onCopy, onRegenerate, onSignIn, usageUrl }), [onCopy, onRegenerate, onSignIn, usageUrl]);
  const last = messages[messages.length - 1];
  return (
    <Tooltip.Provider delayDuration={350}>
      <div className="gb-thread">
        {messages.map((message) => message.role === "user"
          ? <UserMessage key={message.id} message={message} busy={busy} onCopy={onCopy} onRetry={onRetry} />
          : <AssistantMessage key={message.id} message={message} plugins={plugins} appearance={appearance} portal={portal} actions={actions} createMascot={createMascot} botName={botName} />)}
        {busy && last?.role === "user" && <Typing createMascot={createMascot} botName={botName} />}
        <div className="gb-overlay" ref={overlay} />
      </div>
    </Tooltip.Provider>
  );
}
