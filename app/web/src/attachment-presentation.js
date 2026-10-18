// Shared by the React thread and the plain-DOM composer panel.
const sizeNumber = new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 });
export function formatAttachmentSize(bytes) {
  if (bytes < 1024) return `${sizeNumber.format(bytes)} o`;
  if (bytes < 1024 * 1024) return `${sizeNumber.format(bytes / 1024)} Ko`;
  return `${sizeNumber.format(bytes / (1024 * 1024))} Mo`;
}

export function documentBudgetMessage(limit) {
  return `Ces documents dépassent ensemble ce que le modèle peut lire en un message (environ ${sizeNumber.format(limit)} caractères). Retirez-en un ou joignez des extraits.`;
}

// Lucide FileText (ISC), the same outline in the composer and the thread.
export const documentIconPaths = [
  "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",
  "M14 2v5a1 1 0 0 0 1 1h5", "M10 9H8", "M16 13H8", "M16 17H8",
];
export function createDocumentIcon(document) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  for (const [name, value] of Object.entries({ width: 22, height: 26, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": 2, "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": true, focusable: false })) svg.setAttribute(name, String(value));
  for (const d of documentIconPaths) {
    const path = document.createElementNS(ns, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}
