// Recherche dans le fil ouvert. La comparaison est NFC puis toLocaleLowerCase("fr"),
// comme la recherche globale. Les index repliés sont ramenés aux offsets UTF-16
// du texte affiché : un « é » NFD ou un « İ » ne décalent pas le surlignage.

const BODY_MARK = "[data-find-body]";
const BODY_CLASS = ".gb-bubble, .gb-markdown, .gb-instruction-text";

function fold(value) {
  return String(value ?? "").normalize("NFC").toLocaleLowerCase("fr");
}

function foldSpans(original) {
  const segmenter = new Intl.Segmenter("fr", { granularity: "grapheme" });
  const spans = [];
  let folded = "";
  for (const { segment, index } of segmenter.segment(original)) {
    const part = fold(segment);
    spans.push({ start: index, end: index + segment.length, foldStart: folded.length, foldEnd: folded.length + part.length });
    folded += part;
  }
  if (folded === fold(original)) return { folded, spans };
  return alignSpans(original);
}

function alignSpans(original) {
  const expected = fold(original);
  const spans = [];
  let folded = "";
  let index = 0;
  while (index < original.length) {
    let end = index + (original.codePointAt(index) > 0xffff ? 2 : 1);
    let part = fold(original.slice(index, end));
    while (end < original.length && !expected.startsWith(folded + part) && end - index < 32) {
      end += original.codePointAt(end) > 0xffff ? 2 : 1;
      part = fold(original.slice(index, end));
    }
    spans.push({ start: index, end, foldStart: folded.length, foldEnd: folded.length + part.length });
    folded += part;
    index = end;
  }
  if (folded !== expected) return { folded: expected, spans: [{ start: 0, end: original.length, foldStart: 0, foldEnd: expected.length }] };
  return { folded, spans };
}

// Les spans sont ordonnés : une recherche dichotomique trouve le premier span
// touché, puis on avance jusqu’à la fin de la correspondance.
function cover(spans, foldStart, foldEnd, length) {
  let low = 0;
  let high = spans.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (spans[middle].foldEnd <= foldStart) low = middle + 1;
    else high = middle;
  }
  let start = length;
  let end = 0;
  for (let index = low; index < spans.length && spans[index].foldStart < foldEnd; index += 1) {
    const span = spans[index];
    if (span.foldStart === span.foldEnd) continue;
    start = Math.min(start, span.start);
    end = Math.max(end, span.end);
  }
  if (end <= start) return null;
  return { start, end };
}

export function matchRanges(text, query) {
  const source = String(text ?? "");
  const needle = fold(query).trim();
  if (!needle) return [];
  const { folded, spans } = foldSpans(source);
  const matches = [];
  let from = 0;
  while (from < folded.length) {
    const at = folded.indexOf(needle, from);
    if (at < 0) break;
    const range = cover(spans, at, at + needle.length, source.length);
    if (range) matches.push(range);
    from = at + needle.length;
  }
  return matches;
}

function isChrome(element) {
  const tag = element.tagName;
  if (tag === "BUTTON") return element.getAttribute?.("data-streamdown") !== "link";
  if (tag === "HEADER" || tag === "FOOTER" || tag === "TIME") return true;
  return String(element.className || "").split(/\s+/).some((name) => name === "gb-meta" || name === "gb-actions" || name === "gb-status");
}

function bodyElements(root) {
  if (!root?.querySelectorAll) return [];
  const marked = [...root.querySelectorAll(BODY_MARK)];
  if (marked.length) return marked;
  return [...root.querySelectorAll(BODY_CLASS)];
}

const BLOCKS = new Set(["ADDRESS", "ARTICLE", "ASIDE", "BLOCKQUOTE", "DD", "DETAILS", "DIV", "DL", "DT", "FIGCAPTION", "FIGURE", "H1", "H2", "H3", "H4", "H5", "H6", "HR", "LI", "OL", "P", "PRE", "SECTION", "TABLE", "TBODY", "TD", "TH", "THEAD", "TR", "UL"]);

// Les blocs, les sauts de ligne et les nœuds faits d’espaces blancs sont séparés par une espace sans nœud texte :
// « Bonjour »/« monde » en deux paragraphes se cherche « Bonjour monde », et le
// texte en ligne reste collé.
function visibleText(body) {
  const pieces = [];
  let full = "";
  const separate = () => { if (full && !full.endsWith(" ")) full += " "; };
  const visit = (node) => {
    if (!node) return;
    if (node.nodeType === 3) {
      if (!node.nodeValue) return;
      // Streamdown laisse un « \n » entre les blocs : il vaut une seule espace.
      if (!node.nodeValue.trim()) { separate(); return; }
      pieces.push({ node, start: full.length, end: full.length + node.nodeValue.length });
      full += node.nodeValue;
      return;
    }
    if (node.nodeType !== 1 || isChrome(node)) return;
    if (node.tagName === "BR") { separate(); return; }
    const block = BLOCKS.has(node.tagName);
    if (block) separate();
    for (const child of node.childNodes || []) visit(child);
    if (block) separate();
  };
  visit(body);
  return { full, ranges: pieces };
}

function renderedBody(root) {
  let full = "";
  const ranges = [];
  for (const body of bodyElements(root)) {
    const piece = visibleText(body);
    if (full && piece.full && !full.endsWith(" ")) full += " ";
    const base = full.length;
    for (const range of piece.ranges) ranges.push({ node: range.node, start: base + range.start, end: base + range.end });
    full += piece.full;
  }
  return { full, ranges };
}

export function findInRendered(root, query) {
  if (!root?.querySelectorAll) return null;
  const articles = [];
  if (root.getAttribute?.("data-message-id")) articles.push(root);
  for (const article of root.querySelectorAll("[data-message-id]")) {
    if (article !== root) articles.push(article);
  }
  const hosts = articles.length ? articles : (bodyElements(root).length ? [root] : []);
  if (!hosts.some((host) => bodyElements(host).length)) return null;
  const matches = [];
  for (const host of hosts) {
    const { full } = renderedBody(host);
    const messageId = host.getAttribute?.("data-message-id") || null;
    matchRanges(full, query).forEach((range, indexInMessage) => {
      matches.push({ messageId, indexInMessage, start: range.start, end: range.end });
    });
  }
  return matches;
}

export function shouldRecenterFind(previous, find) {
  const identity = find?.query && find.messageId
    ? `${find.query}\0${find.messageId}\0${find.indexInMessage || 0}`
    : "";
  return { identity, recenter: identity !== previous };
}

export function cycleFind(index, count, direction) {
  if (!count) return 0;
  return (index + direction + count) % count;
}

export function findCountLabel(count, index) {
  if (!count) return "Aucun résultat";
  const total = count === 1 ? "1 résultat" : `${count} résultats`;
  return `${index + 1} sur ${total}`;
}

// WebKit avant Safari 17.2 n’a pas les Custom Highlights : la sélection native
// montre alors l’occurrence, sans toucher aux nœuds que React met à jour. Tant
// qu’un champ de saisie a le focus, on la laisse en place : la déplacer dans le
// fil sortirait la frappe du champ de recherche.
const selected = new WeakMap();

export function clearFindMarks(root) {
  const document = root?.ownerDocument;
  const view = document?.defaultView;
  view?.CSS?.highlights?.delete("gb-find-current");
  const range = selected.get(document);
  if (!range) return;
  selected.delete(document);
  const selection = view?.getSelection?.();
  if (selection?.rangeCount && selection.getRangeAt(0) === range) selection.removeAllRanges();
}

function highlightRange(ranges, start, end) {
  const overlapping = ranges.filter((range) => range.start < end && range.end > start);
  if (!overlapping.length) return null;
  const first = overlapping[0];
  const last = overlapping[overlapping.length - 1];
  const document = first.node.ownerDocument;
  const view = document.defaultView;
  const painted = view?.CSS?.highlights && view.Highlight;
  const selection = painted ? null : view?.getSelection?.();
  if (!painted && !selection) return null;
  const range = document.createRange();
  range.setStart(first.node, Math.max(0, start - first.start));
  range.setEnd(last.node, Math.min(last.node.nodeValue.length, end - last.start));
  if (painted) view.CSS.highlights.set("gb-find-current", new view.Highlight(range));
  else if (!document.activeElement?.matches?.("input, textarea, [contenteditable]")) {
    selection.removeAllRanges();
    selection.addRange(range);
    selected.set(document, range);
  }
  return {
    textContent: range.toString(),
    parentNode: first.node.parentElement,
    scrollIntoView(options) {
      const element = first.node.parentElement;
      element?.scrollIntoView(options);
      const rect = range.getBoundingClientRect();
      for (let parent = element; parent; parent = parent.parentElement) {
        if (parent.scrollHeight <= parent.clientHeight || !/(auto|scroll)/.test(view.getComputedStyle(parent).overflowY)) continue;
        const bounds = parent.getBoundingClientRect();
        parent.scrollBy({ top: rect.top + rect.height / 2 - bounds.top - parent.clientHeight / 2 });
        break;
      }
    },
  };
}

// Surligne la n-ième occurrence du corps affiché. L’en-tête, l’heure et les
// boutons de notation ne comptent pas. Retourne une ancre de défilement, ou null.
export function highlightOccurrence(root, query, indexInMessage = 0) {
  if (!root?.querySelectorAll) return null;
  clearFindMarks(root);
  const { full, ranges } = renderedBody(root);
  const match = matchRanges(full, query)[indexInMessage];
  if (!match) return null;
  return highlightRange(ranges, match.start, match.end);
}
