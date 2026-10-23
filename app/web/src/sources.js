// Sources citées d'une réponse (#212 les enregistre, #213 les montre) : le registre du tour
// arrive dans `payload.sources`, la vérification de chaque `[n]` dans `payload.citations`.
export const MISSING = "Source introuvable";
export const DISCONNECTED = "Ce compte n’est plus connecté — reconnecte-le pour ouvrir la source.";
export const GONE_FROM_LIBRARY = "Ce fichier n’est plus dans la bibliothèque.";
export const NO_LINK = "Cette source n’a pas de lien à ouvrir.";
export const ATTACHMENT_NO_LINK = "Cette pièce jointe n’a pas de lien à ouvrir : elle reste dans la conversation où elle a été déposée.";
export const OPEN_UNAVAILABLE = "L’ouverture des sources n’est pas disponible ici.";

const KINDS = {
  courriel: { label: "Courriel", who: "Expéditeur", icon: "mail" },
  evenement: { label: "Rendez-vous", who: "Organisateur", icon: "calendar" },
  fichier_drive: { label: "Fichier Drive", who: "Auteur", icon: "file" },
  feuille: { label: "Feuille de calcul", who: "Auteur", icon: "sheet" },
  bibliotheque: { label: "Bibliothèque", who: "Auteur", icon: "file" },
  piece_jointe: { label: "Pièce jointe", who: "Auteur", icon: "file" },
};

export function kindOf(source) {
  return KINDS[source?.type] || { label: "Source", who: "Auteur", icon: "file" };
}

// Le registre du tour, sans entrée que l'interface ne saurait numéroter.
export function messageSources(message) {
  const list = message?.payload?.sources;
  if (!Array.isArray(list)) return [];
  return list.filter((source) => source && Number.isInteger(source.n) && source.n > 0 && typeof source.titre === "string");
}

// Un `[n]` est lisible quand le registre le connaît ; le verdict du moteur ne le contredit pas.
export function citationState(message, n) {
  const source = messageSources(message).find((entry) => entry.n === n);
  const verdict = (message?.payload?.citations || []).find((entry) => entry?.n === n);
  if (!source || verdict?.etat === "introuvable") return { n, found: false, label: verdict?.libelle ? capital(verdict.libelle) : MISSING };
  return { n, found: true, source };
}

function capital(text) {
  return text.charAt(0).toLocaleUpperCase("fr") + text.slice(1);
}

// Reprend les règles de lecture du moteur : le code, un indice (`a[1]`) et un lien (`[1](…)`) ne citent rien.
function afterCode(chars, start) {
  const run = (from) => { let count = 0; while (chars[from + count] === "`") count++; return count; };
  const open = run(start);
  let index = start + open;
  while (index < chars.length) {
    if (chars[index] === "`") {
      const close = run(index);
      if (close === open) return index + close;
      index += close;
    } else index++;
  }
  return start + open;
}

// Past la destination `(…)` ouverte à `start`, parenthèses équilibrées ; non fermée, rien n'est sauté.
function afterDestination(chars, start) {
  let depth = 0;
  for (let index = start; index < chars.length; index++) {
    if (chars[index] === "\\") { index++; continue; }
    if (chars[index] === "(") depth++;
    else if (chars[index] === ")" && --depth === 0) return index + 1;
  }
  return start;
}

const NUMBERS_ONLY = /^[\s0-9,;]+$/;

// `links` : les numéros définis comme liens de référence (`[1]: https://…`), qui ne citent rien.
function citationAt(chars, start, links = new Set()) {
  const before = chars[start - 1];
  if (start > 0 && (/[\p{L}\p{N}]/u.test(before) || before === "_" || before === ")")) return null;
  const close = chars.indexOf("]", start + 1);
  if (close < 0) return null;
  const numbers = [];
  for (const part of chars.slice(start + 1, close).join("").split(/[,;]/)) {
    const digits = part.trim();
    if (!/^[0-9]{1,4}$/.test(digits)) return null;
    numbers.push(Number(digits));
  }
  if (chars[close + 1] === "(" || chars[close + 1] === ":") return null;
  if (numbers.some((n) => links.has(n))) return null;
  // `[1][label]` et `[texte][1]` : un lien de référence, pas une citation suivie d'une autre (`[1][4]`).
  if (chars[close + 1] === "[") {
    const next = chars.indexOf("]", close + 2);
    if (next >= 0 && !NUMBERS_ONLY.test(chars.slice(close + 2, next).join("")) ) return null;
  }
  if (before === "]") {
    const open = chars.lastIndexOf("[", start - 2);
    if (open >= 0 && !NUMBERS_ONLY.test(chars.slice(open + 1, start - 1).join(""))) return null;
  }
  return { numbers, next: close + 1 };
}

// Chaque `[n]` de la prose, dans l'ordre, avec sa place dans la ligne.
// Le code n'est pas de la prose : clôtures ``` ou ~~~ (fermées par la même marque, au moins aussi longue),
// et bloc indenté de quatre espaces après une ligne vide, sauf sous un élément de liste.
function scan(body, onCitation) {
  const links = new Set([...String(body ?? "").matchAll(/^ {0,3}\[(\d{1,4})\]:/gm)].map((match) => Number(match[1])));
  let fence = null;
  let indentedCode = false;
  let previousBlank = true;
  let listContext = false;
  return String(body ?? "").split("\n").map((line) => {
    const mark = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (mark && mark[1][0] === fence[0] && mark[1].length >= fence.length && !line.slice(mark[0].length).trim()) fence = null;
      return line;
    }
    if (mark) { fence = mark[1]; indentedCode = false; previousBlank = false; return line; }
    const blank = !line.trim();
    const indented = /^( {4}|\t)/.test(line);
    if (indented && (indentedCode || (previousBlank && !listContext))) {
      indentedCode = true;
      previousBlank = false;
      return line;
    }
    if (!blank) {
      indentedCode = false;
      if (!indented) listContext = /^ {0,3}([-*+]|\d+[.)])\s/.test(line);
    }
    previousBlank = blank;
    if (blank) return line;
    const chars = [...line];
    let out = "";
    let index = 0;
    while (index < chars.length) {
      if (chars[index] === "`") {
        const next = afterCode(chars, index);
        out += chars.slice(index, next).join("");
        index = next;
      } else if (chars[index] === "]" && chars[index + 1] === "(") {
        // La destination d'un lien ou d'une image n'est pas de la prose : `[x](https://a.test/[1]/b)`.
        const next = afterDestination(chars, index + 1);
        out += chars.slice(index, next).join("");
        index = next;
      } else if ((chars[index] === "h" || chars[index] === "H") && /^https?:\/\//i.test(chars.slice(index, index + 8).join("")) && !/[\p{L}\p{N}]/u.test(chars[index - 1] ?? "")) {
        let next = index;
        while (next < chars.length && !/[\s<>]/.test(chars[next])) next++;
        out += chars.slice(index, next).join("");
        index = next;
      } else if (chars[index] === "[") {
        const found = citationAt(chars, index, links);
        if (found) { out += onCitation(found.numbers, chars.slice(index, found.next).join("")); index = found.next; }
        else { out += chars[index]; index++; }
      } else { out += chars[index]; index++; }
    }
    return out;
  }).join("\n");
}

export function citedNumbers(body) {
  const found = [];
  scan(body, (numbers, text) => { for (const n of numbers) if (!found.includes(n)) found.push(n); return text; });
  return found;
}

export const CITE_TAG = "sourcepill";

// Le texte que lit Streamdown : chaque `[n]` devient une balise que le fil dessine en pastille.
export function citeMarkdown(body) {
  return scan(body, (numbers) => numbers.map((n) => `<${CITE_TAG} n="${n}"></${CITE_TAG}>`).join(""));
}

export function hasSources(message) {
  return messageSources(message).length > 0 || (message?.payload?.citations || []).length > 0;
}

// « 2026-10-01 » est un jour, pas un instant : il ne glisse pas d'une veille selon le fuseau.
function day(value) {
  const plain = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(value ?? "").trim());
  const date = plain ? new Date(Number(plain[1]), Number(plain[2]) - 1, Number(plain[3])) : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatSourceDate(value) {
  const date = day(value);
  if (!date) return String(value ?? "");
  return date.toLocaleDateString("fr-CA", { day: "numeric", month: "short", year: "numeric" });
}

// Les lignes de la carte : de qui, quand, quelle plage.
export function sourceFacts(source) {
  const facts = [];
  if (source.auteur) facts.push({ label: kindOf(source).who, value: source.auteur });
  if (source.date) facts.push({ label: "Date", value: formatSourceDate(source.date) });
  if (source.plage) facts.push({ label: "Plage", value: source.plage });
  return facts;
}

export function sourceSummary(source) {
  return [source.titre, source.auteur, source.date ? formatSourceDate(source.date) : "", source.plage].filter(Boolean).join(" · ");
}

// Copier ou exporter une réponse : ses `[n]` suivis des sources, en notes.
export function sourceNotes(message) {
  const body = String(message?.body ?? "");
  const cited = citedNumbers(body);
  if (!cited.length) return "";
  const lines = cited.map((n) => {
    const state = citationState(message, n);
    return state.found ? `[${n}] ${kindOf(state.source).label} : ${sourceSummary(state.source)}` : `[${n}] ${state.label}`;
  });
  return `\n\nSources :\n${lines.join("\n")}`;
}

export function textWithSources(message, text = message?.body) {
  const body = String(text ?? "");
  return body && hasSources(message) ? `${body}${sourceNotes({ ...message, body })}` : body;
}

const GOOGLE = { courriel: "gmail", evenement: "calendar", fichier_drive: "drive", feuille: "drive" };

// Agenda et Drive lisent par le compte et le consentement Gmail : Gmail connecté suffit à ouvrir leurs sources.
function connected(connectors, service, account) {
  return ["gmail", service].some((id) => {
    const item = connectors.find((entry) => entry?.id === id);
    if (!item) return false;
    const accounts = [{ account: item.account, status: item.status }, ...(item.accounts || [])];
    return accounts.some((entry) => entry.status === "connected" && (!account || !entry.account || entry.account === account));
  });
}

function googleUrl(source) {
  const id = encodeURIComponent(source.identifiant);
  const user = source.compte ? encodeURIComponent(source.compte) : "0";
  if (source.type === "courriel") return `https://mail.google.com/mail/u/${user}/#all/${id}`;
  if (source.type === "feuille") return `https://docs.google.com/spreadsheets/d/${id}/edit${source.compte ? `?authuser=${user}` : ""}`;
  if (source.type === "fichier_drive") return `https://drive.google.com/file/d/${id}/view${source.compte ? `?authuser=${user}` : ""}`;
  // Un événement s'ouvre par `eid` : son identifiant et l'agenda (le compte), en base64 sans remplissage.
  if (source.compte && /^[\x21-\x7e]+$/.test(source.identifiant) && /^[\x21-\x7e]+$/.test(source.compte)) {
    const eid = btoa(`${source.identifiant} ${source.compte}`).replace(/=+$/, "");
    return `https://calendar.google.com/calendar/u/${user}/event?eid=${encodeURIComponent(eid)}`;
  }
  const date = day(source.date);
  const path = date ? `r/day/${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}` : "r";
  return `https://calendar.google.com/calendar/u/${user}/${path}`;
}

// Où mène « Ouvrir la source » : un lien Google, un fichier de la bibliothèque, ou la raison pour laquelle rien ne s'ouvre.
// `connectors` est le catalogue des connecteurs : absent, on n'empêche pas l'ouverture.
export function sourceOpening(source, { connectors } = {}) {
  if (!source) return { problem: NO_LINK };
  if (source.type === "piece_jointe") return { problem: ATTACHMENT_NO_LINK };
  if (!source.identifiant) return { problem: NO_LINK };
  if (source.type === "bibliotheque") return { library: source.identifiant };
  if (!GOOGLE[source.type]) return { problem: NO_LINK };
  if (Array.isArray(connectors) && !connected(connectors, GOOGLE[source.type], source.compte)) return { problem: DISCONNECTED };
  return { url: googleUrl(source) };
}
