import { createElement as h, Fragment } from "react";

export function LocalResult({ name, output, arguments: args }) {
  if (!output || typeof output !== "object") return h("p", null, "Résultat indisponible.");
  const fingerprint = typeof output.sha256 === "string" && h("p", { className: "gb-local-fingerprint" }, "Empreinte : ", h("code", null, output.sha256));
  const technical=h("details",null,h("summary",null,"Détails techniques"),h("pre",{className:"gb-engine-data"},JSON.stringify(output,null,2)));
  if (name === "read_file") return h(Fragment, null,
    h("pre", { className: "gb-local-command gb-local-file-content" }, typeof output.content === "string" ? output.content : "Contenu indisponible."), fingerprint,technical);
  if (name === "list_directory") {
    const entries=[...(Array.isArray(output.entries)?output.entries:[])].sort((a,b)=>(b.kind==="dossier")-(a.kind==="dossier") || String(a.name).localeCompare(String(b.name),"fr-CA",{numeric:true}));
    return h(Fragment, null,
      output.truncated && h("p", null, "Liste limitée aux 1000 premières entrées."),
      entries.length ? h("ul", { className: "gb-local-files" }, ...entries.map((entry, index) => h("li", { key: index }, `${typeof entry.name === "string" ? entry.name : "Nom indisponible"} — ${typeof entry.kind === "string" ? entry.kind : "type inconnu"}`))) : h("p",null,"Dossier vide."),technical);
  }
  if (name === "write_file") return h(Fragment, null,
    h("p", null, `${args?.expected_sha256 ? "Fichier remplacé" : "Fichier créé"} · ${typeof output.bytes === "number" ? output.bytes : "?"} octets écrits.`), fingerprint,technical);
  return h("p", null, "Résultat indisponible.");
}
