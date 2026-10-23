import { createPortal } from "react-dom";
import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Calendar, ChevronDown, ExternalLink, FileText, Library, Mail, Sheet, X } from "lucide-react";
import { CITE_TAG, OPEN_UNAVAILABLE, citationState, kindOf, messageSources, sourceFacts, sourceSummary } from "./sources.js";

const ICONS = { mail: Mail, calendar: Calendar, sheet: Sheet, file: FileText, library: Library };

function SourceIcon({ source }) {
  const Icon = source.type === "bibliotheque" ? ICONS.library : ICONS[kindOf(source).icon] || FileText;
  return <Icon aria-hidden="true" />;
}

// Le fil donne à chaque pastille sa réponse et la façon d'ouvrir une source.
export const SourcesContext = createContext({ message: null, onOpenSource: null });

// Ouvre la source par l'identifiant qu'elle porte ; renvoie la raison, en français, quand rien ne s'ouvre.
export async function openSource(source, onOpenSource, message) {
  if (typeof onOpenSource !== "function") return OPEN_UNAVAILABLE;
  try {
    const result = await onOpenSource(source, message);
    return result?.ok === false ? result.message || "La source n’a pas pu être ouverte." : null;
  } catch {
    return "La source n’a pas pu être ouverte.";
  }
}

function OpenSourceButton({ source, message, onOpenSource }) {
  const [problem, setProblem] = useState(null);
  const id = useId();
  return (
    <>
      <button type="button" className="gb-source-open" aria-describedby={problem ? id : undefined} onClick={async () => { const reason = await openSource(source, onOpenSource, message); setProblem(reason); }}>
        <ExternalLink aria-hidden="true" />Ouvrir la source
      </button>
      {problem && <span className="gb-source-problem" id={id} role="status">{problem}</span>}
    </>
  );
}

export function SourceCard({ source, message, onOpenSource, onClose, id, align, style, floating, cardRef }) {
  const facts = sourceFacts(source);
  return (
    <span className="gb-source-card" id={id} ref={cardRef} style={style} data-floating={floating || undefined} role="group" aria-label={`Source ${source.n}`} data-align={align}>
      <span className="gb-source-kind"><SourceIcon source={source} />{kindOf(source).label}</span>
      <strong className="gb-source-title">{source.titre}</strong>
      {facts.length > 0 && (
        <span className="gb-source-facts">
          {facts.map((fact) => <span key={fact.label}><span>{fact.label}</span><span>{fact.value}</span></span>)}
        </span>
      )}
      <span className="gb-source-actions">
        <OpenSourceButton source={source} message={message} onOpenSource={onOpenSource} />
        {onClose && <button type="button" className="gb-source-close" onClick={onClose}>Fermer</button>}
      </span>
    </span>
  );
}

const CARD_GAP = 6;
const CARD_MARGIN = 8;

// La carte est posée en fixe sur le corps de la page, à partir de la pastille : le défilement d'un tableau
// ou d'un bloc de la réponse ne la rogne pas. Sous la pastille, au-dessus si le bas de la fenêtre manque de place.
export function placeSourceCard(anchor, card, viewport) {
  const left = Math.min(Math.max(anchor.left, CARD_MARGIN), Math.max(CARD_MARGIN, viewport.width - card.width - CARD_MARGIN));
  const below = anchor.bottom + CARD_GAP;
  const above = anchor.top - CARD_GAP - card.height;
  const up = below + card.height > viewport.height - CARD_MARGIN && above >= CARD_MARGIN;
  return { top: Math.max(CARD_MARGIN, up ? above : below), left };
}

// `[n]` dans la prose : une pastille qui ouvre la carte de sa source, ou un numéro barré sans source.
export function SourcePill({ n, defaultOpen = false }) {
  const { message, onOpenSource } = useContext(SourcesContext);
  const number = Number(n);
  const state = citationState(message, number);
  const [open, setOpen] = useState(defaultOpen);
  const [place, setPlace] = useState(null);
  const root = useRef(null);
  const card = useRef(null);
  const id = useId();
  const floating = typeof document !== "undefined" && Boolean(document.body?.appendChild);
  useLayoutEffect(() => {
    if (!open || !floating) return undefined;
    const measure = () => {
      const pill = root.current?.querySelector(".gb-cite-pill");
      if (!pill || !card.current) return;
      const view = pill.ownerDocument.defaultView;
      const anchor = pill.getBoundingClientRect();
      setPlace(placeSourceCard(anchor, { width: card.current.offsetWidth, height: card.current.offsetHeight }, { width: view.innerWidth, height: view.innerHeight }));
    };
    measure();
    const view = root.current?.ownerDocument.defaultView;
    view?.addEventListener("resize", measure);
    view?.addEventListener("scroll", measure, true);
    return () => { view?.removeEventListener("resize", measure); view?.removeEventListener("scroll", measure, true); };
  }, [open, floating]);
  useEffect(() => {
    if (!open) return undefined;
    const view = root.current?.ownerDocument;
    const key = (event) => { if (event.key === "Escape") { setOpen(false); root.current?.querySelector(".gb-cite-pill")?.focus(); } };
    const away = (event) => { if (!root.current?.contains(event.target) && !card.current?.contains(event.target)) setOpen(false); };
    view?.addEventListener("keydown", key);
    view?.addEventListener("pointerdown", away);
    return () => { view?.removeEventListener("keydown", key); view?.removeEventListener("pointerdown", away); };
  }, [open]);
  if (!state.found) {
    return <span className="gb-cite gb-cite-missing" tabIndex={0} title={state.label} data-tip={state.label} aria-label={`${number} : ${state.label}`}>[{number}]</span>;
  }
  const shown = open && (
    <SourceCard source={state.source} message={message} onOpenSource={onOpenSource} onClose={() => setOpen(false)} id={id} cardRef={card} floating={floating}
      style={floating ? { top: place?.top ?? 0, left: place?.left ?? 0, visibility: place ? undefined : "hidden" } : undefined} />
  );
  return (
    <span className="gb-cite" ref={root}>
      <button type="button" className="gb-cite-pill" aria-expanded={open} aria-controls={open ? id : undefined} aria-label={`Source ${number} : ${state.source.titre}`} title={sourceSummary(state.source)} onClick={() => setOpen(!open)}>{number}</button>
      {floating ? shown && createPortal(shown, document.body) : shown}
    </span>
  );
}

// Pour Streamdown : la balise que `citeMarkdown` écrit à la place de chaque `[n]`.
export const citeTags = { [CITE_TAG]: ["n"] };
export const citeComponents = { [CITE_TAG]: ({ n }) => <SourcePill n={n} /> };

// Sous la réponse : « Sources 3 », repliée, puis la liste de toutes les sources du tour.
export function SourcesLine({ message, onOpenSource, defaultOpen = false }) {
  const sources = messageSources(message);
  if (!sources.length) return null;
  const shown = [];
  for (const source of sources) if (!shown.some((entry) => kindOf(entry).icon === kindOf(source).icon)) shown.push(source);
  return (
    <details className="gb-sources" open={defaultOpen || undefined}>
      <summary>
        <span className="gb-sources-icons" aria-hidden="true">{shown.slice(0, 3).map((source) => <span key={source.n}><SourceIcon source={source} /></span>)}</span>
        <span className="gb-sources-label">Sources <span className="gb-sources-count">{sources.length}</span></span>
        <ChevronDown className="gb-sources-chevron" aria-hidden="true" />
      </summary>
      <ol className="gb-sources-list">
        {sources.map((source) => (
          <li key={source.n} data-source={source.n}>
            <span className="gb-sources-n">{source.n}</span>
            <span className="gb-sources-copy">
              <strong>{source.titre}</strong>
              <small>{[kindOf(source).label, ...sourceFacts(source).map((fact) => fact.value)].join(" · ")}</small>
            </span>
            <span className="gb-sources-open"><OpenSourceButton source={source} message={message} onOpenSource={onOpenSource} /></span>
          </li>
        ))}
      </ol>
    </details>
  );
}
