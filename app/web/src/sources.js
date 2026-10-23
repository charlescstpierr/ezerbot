export const sourceDetails = source => [source.titre, source.auteur, source.date, source.plage].filter(Boolean).join(' · ');
export function sourceNotes(text, sources = []) {
  return sources.length ? `${text || ''}\n\nSources\n${sources.map(source => `[${source.n}] ${sourceDetails(source)}`).join('\n')}` : text;
}
// Transform rendered prose, never code, links, or Markdown attributes.
export const citationPlugin = citations => () => tree => {
  const known = new Set(citations.map(item => item.n));
  const walk = node => {
    if (!node.children || ['code', 'pre', 'a', 'script', 'style'].includes(node.tagName)) return;
    node.children = node.children.flatMap(child => {
      if (child.type !== 'text') { walk(child); return [child]; }
      const parts = []; let start = 0;
      for (const match of child.value.matchAll(/(?<![\w])\[(\d+(?:,\s*\d+)*)\]/g)) {
        const numbers = match[1].split(/,\s*/).map(Number);
        if (!numbers.every(n => known.has(n))) continue;
        parts.push({ type: 'text', value: child.value.slice(start, match.index) });
        for (const n of numbers) parts.push({ type: 'element', tagName: 'button', properties: { 'data-source': String(n) }, children: [{ type: 'text', value: `[${n}]` }] });
        start = match.index + match[0].length;
      }
      return [...parts, { type: 'text', value: child.value.slice(start) }];
    });
  };
  walk(tree);
};
export async function openSource(source, { fetch = globalThis.fetch, open = url => globalThis.open(url, '_blank', 'noopener'), botId } = {}) {
  if (source.type === 'piece_jointe') return "Cette pièce jointe ne peut pas être ouverte depuis les sources.";
  try {
    const id = encodeURIComponent(source.identifiant || '');
    if (!id) throw new Error();
    if (source.type === 'bibliotheque') {
      const response = await fetch(`/api/library/source?bot_id=${encodeURIComponent(botId || '')}&id=${id}`);
      if (!response.ok) throw new Error();
      const file = await response.json();
      return { text: file.text };
    }
    const response = await fetch('/api/state');
    if (!response.ok) throw new Error();
    const state = await response.json();
    const service = { courriel: 'gmail', feuille: 'drive', fichier_drive: 'drive', evenement: 'calendar' }[source.type];
    const accounts = (state.mcp?.catalogue || []).filter(item => item.id === service).flatMap(item => [item, ...(item.accounts || [])]);
    if (!accounts.some(item => item.status === 'connected' && item.account === source.compte)) return "Ce compte n'est plus connecté — reconnecte-le pour ouvrir la source.";
    const account = encodeURIComponent(source.compte);
    if (source.type === 'courriel') open(`https://mail.google.com/mail/u/${account}/#all/${id}`);
    else if (['fichier_drive', 'feuille'].includes(source.type)) open(`https://drive.google.com/file/d/${id}/view?authuser=${account}`);
    else if (source.type === 'evenement') open(`https://calendar.google.com/calendar/r/event?eid=${encodeURIComponent(btoa(`${source.identifiant} ${source.compte}`))}&authuser=${account}`);
    else throw new Error();
    return '';
  } catch {
    return source.compte ? "Ce compte n'est plus connecté — reconnecte-le pour ouvrir la source." : "Ce fichier n'est plus accessible dans la bibliothèque.";
  }
}
