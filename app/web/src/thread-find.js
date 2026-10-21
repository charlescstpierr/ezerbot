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

function cover(spans, foldStart, foldEnd, length) {
  let start = length;
  let end = 0;
  for (const span of spans) {
    if (span.foldEnd <= foldStart || span.foldStart >= foldEnd || span.foldStart === span.foldEnd) continue;
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

export function findInThread(messages, query, textOf = (message) => message?.body || "") {
  const matches = [];
  for (const message of messages || []) {
    if (!message?.id) continue;
    matchRanges(textOf(message), query).forEach((range, indexInMessage) => {
      matches.push({ messageId: message.id, indexInMessage, start: range.start, end: range.end });
    });
  }
  return matches;
}

function isChrome(element) {
  const tag = element.tagName;
  if (tag === "BUTTON" || tag === "HEADER" || tag === "FOOTER" || tag === "TIME") return true;
  return String(element.className || "").split(/\s+/).some((name) => name === "gb-meta" || name === "gb-actions" || name === "gb-status");
}

function bodyElements(root) {
  if (!root?.querySelectorAll) return [];
  const marked = [...root.querySelectorAll(BODY_MARK)];
  if (marked.length) return marked;
  return [...root.querySelectorAll(BODY_CLASS)];
}

function visibleText(body) {
  const nodes = [];
  const visit = (node) => {
    if (!node) return;
    if (node.nodeType === 3) {
      if (node.nodeValue) nodes.push(node);
      return;
    }
    if (node.nodeType !== 1 || isChrome(node)) return;
    for (const child of node.childNodes || []) visit(child);
  };
  visit(body);
  let full = "";
  const ranges = nodes.map((node) => {
    const start = full.length;
    full += node.nodeValue;
    return { node, start, end: full.length };
  });
  return { full, ranges };
}

function renderedBody(root) {
  let full = "";
  const ranges = [];
  for (const body of bodyElements(root)) {
    const piece = visibleText(body);
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

export function clearFindMarks(root) {
  if (!root?.querySelectorAll) return;
  for (const mark of [...root.querySelectorAll("mark.gb-find-current")]) {
    const parent = mark.parentNode;
    if (!parent) continue;
    while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
    parent.removeChild(mark);
    parent.normalize?.();
  }
}

function wrapSlice(node, start, end) {
  const text = node.nodeValue || "";
  const mark = node.ownerDocument.createElement("mark");
  mark.className = "gb-find-current";
  mark.textContent = text.slice(start, end);
  const pieces = [];
  if (start > 0) pieces.push(node.ownerDocument.createTextNode(text.slice(0, start)));
  pieces.push(mark);
  if (end < text.length) pieces.push(node.ownerDocument.createTextNode(text.slice(end)));
  node.replaceWith(...pieces);
  return mark;
}

function wrapRange(ranges, start, end) {
  const overlapping = ranges.filter((range) => range.start < end && range.end > start);
  let first = null;
  for (const range of overlapping.reverse()) {
    const localStart = Math.max(0, start - range.start);
    const localEnd = Math.min(range.node.nodeValue.length, end - range.start);
    if (localEnd <= localStart) continue;
    first = wrapSlice(range.node, localStart, localEnd);
  }
  return first;
}

// Surligne la n-ième occurrence du corps affiché. L’en-tête, l’heure et les
// boutons de notation ne comptent pas. Retourne la première marque, ou null.
export function highlightOccurrence(root, query, indexInMessage = 0) {
  if (!root?.querySelectorAll) return null;
  clearFindMarks(root);
  const { full, ranges } = renderedBody(root);
  const match = matchRanges(full, query)[indexInMessage];
  if (!match) return null;
  return wrapRange(ranges, match.start, match.end);
}
