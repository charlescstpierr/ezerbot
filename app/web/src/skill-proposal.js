// Line diff with a bounded LCS for ordinary revisions. Trim identical edges
// first; cap the matrix at 1M cells and fall back to a replaced middle for huge
// inputs. Full proposed content is always available outside this compact diff.
export function skillText(content) {
  if (!content) return "";
  return [`Nom : ${content.display_name || content.name}`, `Description : ${content.description}`, "", "Contenu :", content.body,
    ...(content.files || []).flatMap((file) => ["", `Fichier : ${file.path}`, file.content])].join("\n");
}

export function skillDiffStats(lines) {
  return lines.reduce((counts, line) => {
    if (line.kind === "added") counts.added++;
    if (line.kind === "removed") counts.removed++;
    return counts;
  }, { added: 0, removed: 0 });
}

export function skillDiff(before, after) {
  const a = skillText(before).split("\n"), b = skillText(after).split("\n");
  let prefix = 0, suffix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  while (suffix < a.length - prefix && suffix < b.length - prefix && a[a.length - 1 - suffix] === b[b.length - 1 - suffix]) suffix++;
  if (prefix === a.length && prefix === b.length) return [];
  const old = a.slice(prefix, a.length - suffix), next = b.slice(prefix, b.length - suffix);
  const lines = a.slice(Math.max(0, prefix - 2), prefix).map((text) => ({ kind: "context", text }));
  const add = (kind, text) => lines.push({ kind, text });
  if ((old.length + 1) * (next.length + 1) <= 1_000_000) {
    const width = next.length + 1;
    const table = new Uint32Array((old.length + 1) * width);
    for (let i = old.length - 1; i >= 0; i--) for (let j = next.length - 1; j >= 0; j--) {
      table[i * width + j] = old[i] === next[j] ? table[(i + 1) * width + j + 1] + 1
        : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    }
    let i = 0, j = 0;
    while (i < old.length || j < next.length) {
      if (i < old.length && j < next.length && old[i] === next[j]) { add("context", old[i++]); j++; }
      else if (i < old.length && (j === next.length || table[(i + 1) * width + j] >= table[i * width + j + 1])) add("removed", old[i++]);
      else add("added", next[j++]);
    }
  } else {
    old.forEach((text) => add("removed", text));
    next.forEach((text) => add("added", text));
  }
  a.slice(a.length - suffix, Math.min(a.length, a.length - suffix + 2)).forEach((text) => add("context", text));
  // Keep two unchanged lines around each change, with explicit gaps.
  return lines.flatMap((line, index) => {
    if (line.kind !== "context" || lines.slice(Math.max(0, index - 2), index + 3).some((item) => item.kind !== "context")) return [line];
    if (index > 0 && lines[index - 1].kind === "context" && lines.slice(Math.max(0, index - 3), index + 2).some((item) => item.kind !== "context")) return [{ kind: "context", text: "…" }];
    return [];
  });
}
