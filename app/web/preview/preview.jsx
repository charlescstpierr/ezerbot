import { useState } from "react";
import { createRoot } from "react-dom/client";
import "../../ui/styles.css";
import "../../ui/mascot.css";
import "../src/chat.css";
import "./preview.css";
import { Thread } from "../src/Thread.jsx";
import { Composer } from "./Composer.jsx";

const reply = `Oui. Voici un plan simple pour votre **service de notifications**.

## Les trois parties

1. Une file d'attente reçoit les événements.
2. Un *worker* choisit le canal : courriel, SMS ou \`push\`.
3. Un journal garde le résultat de chaque envoi.

### Le parcours d'un message

\`\`\`mermaid
flowchart LR
  A[Événement] --> B{File d'attente}
  B --> C[Worker]
  C -->|courriel| D[Fournisseur courriel]
  C -->|SMS| E[Fournisseur SMS]
  C -->|push| F[Service push]
  D & E & F --> G[(Journal)]
\`\`\`

### Comparaison des canaux

| Canal | Délai moyen | Coût | Bon pour |
| --- | --- | --- | --- |
| Courriel | 5 s | Faible | Les résumés |
| SMS | 2 s | Élevé | Les alertes urgentes |
| Push | 1 s | Très faible | Les rappels |

### Un exemple en Rust

\`\`\`rust
#[derive(Debug, Clone, Copy)]
enum Canal { Courriel, Sms, Push }

fn choisir(urgent: bool, en_ligne: bool) -> Canal {
    match (urgent, en_ligne) {
        (true, _) => Canal::Sms,
        (false, true) => Canal::Push,
        (false, false) => Canal::Courriel,
    }
}
\`\`\`

Le délai moyen se calcule ainsi : $\\bar{d} = \\frac{1}{n}\\sum_{i=1}^{n} d_i$.

> Conseil : commencez par un seul canal. Ajoutez les autres quand le journal fonctionne.

Voulez-vous que je détaille la reprise après une erreur ?`;

const sequence = `Voici l'ordre des échanges.

\`\`\`mermaid
sequenceDiagram
  participant W as Worker
  participant F as Fournisseur
  participant J as Journal
  W->>F: envoyer(message)
  F-->>W: erreur 503
  W->>W: attendre 2 s
  W->>F: envoyer(message)
  F-->>W: accepté
  W->>J: noter « envoyé »
\`\`\``;

const now = Date.now();
const at = (minutes) => new Date(now - minutes * 60000).toISOString();
const starter = [
  { id: "u1", role: "user", state: "accepted", created_at: at(9), body: "Aide-moi à dessiner un service de notifications. Je veux voir le parcours d'un message, et un exemple de code." },
  { id: "a1", role: "assistant", created_at: at(9), body: reply },
  { id: "u2", role: "user", state: "accepted", created_at: at(4), body: "Oui. Montre-moi la reprise après une erreur." },
  { id: "a2", role: "assistant", created_at: at(4), body: sequence },
  { id: "u3", role: "user", state: "unknown", created_at: at(1), body: "Et si le fournisseur ne répond plus du tout ?" },
];
const echo = "Bien reçu. Ceci est une réponse **simulée** pour la page d'essai.\n\n```mermaid\nflowchart LR\n  A[Votre message] --> B[Grokbot] --> C[Réponse]\n```";

function createMascot() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "30 25 270 235");
  svg.innerHTML = `<path d="M68 217C37 194 39 144 55 103C72 61 110 37 151 37C198 33 249 60 270 104C290 145 291 193 267 219C241 248 195 251 153 249C117 248 88 234 68 217Z" fill="#3f4450" stroke="#ffffff1f" stroke-width="2"/><rect x="147" y="116" width="23" height="44" rx="11.5" transform="rotate(-22 158.5 138)" fill="var(--mascot-eye)"/><rect x="207" y="99" width="20" height="39" rx="10" transform="rotate(-22 217 118.5)" fill="var(--mascot-eye)"/>`;
  return svg;
}

const variants = [["bubbles", "1. Bulles"], ["document", "2. Document"], ["cards", "3. Cartes"]];
const accents = [["citron", "#e6ff3d"], ["orange", "#ff5c1f"], ["ambre", "#ffb224"], ["menthe", "#35e0a1"], ["cyan", "#22d3ee"], ["magenta", "#ff3d8b"], ["vermillon", "#ff4530"], ["violet", "#9b7bff"], ["blanc", "#9a9fa9"]];
const root = document.documentElement;

function Preview() {
  const [variant, setVariant] = useState("bubbles");
  const [theme, setTheme] = useState("dark");
  const [accent, setAccent] = useState("menthe");
  const [note, setNote] = useState("");
  const [status, setStatus] = useState("Rien n'est envoyé pour l'instant.");
  const [messages, setMessages] = useState(starter);
  const [busy, setBusy] = useState(false);

  function send(body) {
    const id = String(Date.now());
    setMessages((list) => [...list, { id: `u${id}`, role: "user", state: "dispatching", created_at: new Date().toISOString(), body }]);
    setBusy(true);
    setTimeout(() => {
      setMessages((list) => [...list.map((item) => item.id === `u${id}` ? { ...item, state: "accepted" } : item), { id: `a${id}`, role: "assistant", created_at: new Date().toISOString(), body: echo }]);
      setBusy(false);
    }, 1400);
  }
  const set = (key, value, update) => { root.dataset[key] = value; update(value); };
  const label = variants.find(([id]) => id === variant)[1];

  function submit(event) {
    event.preventDefault();
    const text = `Design du chat : ${label}` + (note.trim() ? ` · Remarque : ${note.trim()}` : "");
    if (!window.lavish) { setStatus(`Lavish n'est pas actif. Copiez ceci dans le chat : ${text}`); return; }
    window.lavish.queuePrompt(`Applique ce design de chat dans Grokbot. ${text}`, { tag: "choice", text, element: event.currentTarget, queueKey: "design-chat", data: { variant, note: note.trim() } });
    window.lavish.sendQueuedPrompts?.();
    setStatus(`Envoyé : ${text}`);
  }

  return (
    <div className="pv-page">
      <h1>Le nouveau design du chat</h1>
      <p className="pv-lead">Ceci est le vrai composant, pas un dessin. Le texte riche, le code coloré, le tableau, la formule et les diagrammes fonctionnent. Vous pouvez écrire un message en bas et l'envoyer : une réponse simulée arrive. Essayez les trois styles, puis envoyez votre choix en bas.</p>
      <div className="pv-bar">
        <div className="pv-group">Style
          <div className="pv-choice" role="radiogroup" aria-label="Style du chat">
            {variants.map(([id, name]) => <label key={id}><input type="radio" name="variant" checked={variant === id} onChange={() => set("chatVariant", id, setVariant)} />{name}</label>)}
          </div>
        </div>
        <div className="pv-group">Thème
          <div className="pv-choice" role="radiogroup" aria-label="Thème">
            {[["dark", "Sombre"], ["light", "Clair"]].map(([id, name]) => <label key={id}><input type="radio" name="theme" checked={theme === id} onChange={() => set("theme", id, setTheme)} />{name}</label>)}
          </div>
        </div>
        <div className="pv-group">Accent
          {accents.map(([id, color]) => <button key={id} type="button" className="pv-dot" style={{ "--c": color }} title={id} aria-label={id} aria-pressed={accent === id} onClick={() => set("accent", id, setAccent)} />)}
        </div>
      </div>
      <div className="pv-panel">
        <div className="pv-top"><span>Espace personnel / <b>Service de notifications</b></span><span>Local</span></div>
        <Thread messages={messages} busy={busy} onCopy={async (text) => { try { await navigator.clipboard.writeText(text); return true; } catch { return false; } }} onRetry={() => {}} createMascot={createMascot} />
        <Composer disabled={busy} onSend={send} />
      </div>
      <form className="pv-form" data-lavish-question="design-chat" onSubmit={submit}>
        <h2>Votre choix : {label}</h2>
        <p>Vous pouvez ajouter une remarque. Exemple : « le style 1, mais avec des bulles plus carrées ».</p>
        <textarea value={note} onChange={(event) => setNote(event.target.value)} placeholder="Remarque (facultatif)" />
        <div className="pv-send"><button type="submit">Envoyer mon choix</button><span>{status}</span></div>
      </form>
    </div>
  );
}

createRoot(document.getElementById("root")).render(<Preview />);
