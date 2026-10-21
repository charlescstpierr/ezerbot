// Recherche dans le fil ouvert. Les index suivent toLocaleLowerCase("fr"),
// la même comparaison que la recherche globale des conversations.

function fold(value) {
  return String(value ?? "").normalize("NFC").toLocaleLowerCase("fr");
}

export function findInThread(messages, query, textOf = (message) => message?.body || "") {
  const needle = fold(query).trim();
  if (!needle) return [];
  const matches = [];
  for (const message of messages || []) {
    if (!message?.id) continue;
    const folded = fold(textOf(message));
    let from = 0;
    let indexInMessage = 0;
    while (from < folded.length) {
      const at = folded.indexOf(needle, from);
      if (at < 0) break;
      matches.push({ messageId: message.id, indexInMessage, start: at, end: at + needle.length });
      indexInMessage += 1;
      from = at + needle.length;
    }
  }
  return matches;
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

// Surligne la n-ième occurrence visible. Retourne la première marque, ou null.
export function highlightOccurrence(root, query, indexInMessage = 0) {
  if (!root || typeof document === "undefined") return null;
  const needle = fold(query).trim();
  if (!needle) return null;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes = [];
  let node = walker.nextNode();
  while (node) {
    if (node.nodeValue && !node.parentElement?.closest?.("mark.gb-find-current")) nodes.push(node);
    node = walker.nextNode();
  }
  let full = "";
  const ranges = nodes.map((textNode) => {
    const start = full.length;
    full += textNode.nodeValue;
    return { node: textNode, start };
  });
  const folded = fold(full);
  let from = 0;
  let seen = 0;
  let at = -1;
  while (from < folded.length) {
    const found = folded.indexOf(needle, from);
    if (found < 0) return null;
    if (seen === indexInMessage) { at = found; break; }
    seen += 1;
    from = found + needle.length;
  }
  if (at < 0) return null;
  const end = at + needle.length;
  const overlapping = ranges.filter((range, position) => {
    const rangeEnd = ranges[position + 1]?.start ?? full.length;
    return range.start < end && rangeEnd > at;
  });
  let first = null;
  for (const range of overlapping.reverse()) {
    const rangeEnd = range.start + range.node.nodeValue.length;
    const localStart = Math.max(0, at - range.start);
    const localEnd = Math.min(range.node.nodeValue.length, end - range.start);
    if (localEnd <= localStart || range.start >= rangeEnd) continue;
    const mark = wrapSlice(range.node, localStart, localEnd);
    first = mark;
  }
  return first;
}
