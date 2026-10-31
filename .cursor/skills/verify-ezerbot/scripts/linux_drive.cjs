// Linux driver for verify-ezerbot: reuses tools/linux-captures.cjs (isolated wrapper, own Chrome
// profile, local fake provider) and drives three entries of the feature map under Xvfb.
// Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>
const { readdirSync, writeFileSync } = require("node:fs");
const { join, resolve } = require("node:path");
const { VIEWS, capture } = require(resolve(__dirname, "../../../../tools/linux-captures.cjs"));

const QUESTION = "Bonjour, prépare une courte liste pour organiser ma semaine.";
const FOLLOW_UP = "Et en un mot ?";
const question = "[...document.querySelectorAll('#chat-scroll .gb-message')].find(node => node.textContent.includes(" + JSON.stringify(QUESTION) + "))";
const threadId = "GrokbotPanels.state.conversations.find(item => item.title?.startsWith('Fil'))?.id";

// Views driven for each entry of features/, in order. The first two come from tools/linux-captures.cjs.
const DRIVEN = Object.freeze([
  { id: "thread-menu", label: "Menu du message", action: `${question}.querySelector('button[aria-label="Actions du message"]').click()`,
    ready: `[...document.querySelectorAll('[role=menu][aria-label="Actions du message"] [role=menuitem]')].some(item => item.textContent === 'Répondre en fil' && !item.disabled)` },
  { id: "thread-open", label: "Sous-fil ouvert", action: `[...document.querySelectorAll('[role=menu][aria-label="Actions du message"] [role=menuitem]')].find(item => item.textContent === 'Répondre en fil').click()`,
    ready: `!document.getElementById('thread-banner')?.hidden && document.getElementById('thread-banner-quote')?.textContent.includes(${JSON.stringify(QUESTION.slice(0, 20))}) && document.getElementById('conversation-title')?.textContent.startsWith('Fil') && !document.getElementById('chat-scroll')?.textContent.includes('Voici votre document.')` },
  { id: "thread-reply", label: "Réponse dans le sous-fil", action: `fetch('/api/send', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({conversation_id:${threadId}, message_id:'linux-verify-thread', body:${JSON.stringify(FOLLOW_UP)}})}).then(r => { if (!r.ok) throw Error('thread send failed'); return GrokbotPanels.refresh(); })`,
    ready: `!document.getElementById('thread-banner')?.hidden && document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(FOLLOW_UP)}) && document.getElementById('chat-scroll')?.textContent.includes('Voici votre document.') && !GrokbotPanels.state.conversations.find(item => item.id === ${threadId})?.messages?.some(message => GrokbotChat.isPendingReply(message, false))` },
  { id: "thread-main", label: "Retour au fil principal", action: "document.getElementById('thread-banner-open').click()",
    ready: `document.getElementById('thread-banner')?.hidden && document.getElementById('conversation-title')?.textContent === 'Préparer la semaine' && document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(QUESTION)}) && !document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(FOLLOW_UP)})` },
  { id: "settings-connectors", label: "Paramètres, section Connecteurs", keepScroll: true,
    action: "document.getElementById('settings-button').click(); document.querySelector('#settings-dialog [data-panel=mcp]')?.scrollIntoView({block:'start'})",
    ready: "(() => { const panel = document.querySelector('#settings-dialog[open] [data-panel=mcp]'); if (!panel) return false; const box = panel.getBoundingClientRect(); return panel.querySelector('h3')?.textContent === 'Connecteurs' && [...panel.querySelectorAll('summary')].some(item => item.textContent.includes('Ajouter un serveur MCP')) && box.top >= 0 && box.top < innerHeight; })()" },
]);

// Entry of features/ -> views that must all be captured for the entry to pass.
const FEATURES = Object.freeze([
  { feature: "accueil", views: ["home-dark"] },
  { feature: "reponse-fil", views: ["conversation-dark", "thread-menu", "thread-open", "thread-reply", "thread-main"] },
  { feature: "parametres-connecteurs", views: ["settings-dark", "settings-connectors"] },
]);

function selectedViews() {
  const registry = [...VIEWS, ...DRIVEN];
  return FEATURES.flatMap(({ feature, views }) => views.map(id => {
    const found = registry.find(item => item.id === id);
    if (!found) throw new Error(`View ${id} is missing.`);
    return { ...found, feature };
  }));
}

async function main([sha, binary, output]) {
  const views = selectedViews();
  const controller = new AbortController();
  for (const name of ["SIGINT", "SIGTERM"]) process.once(name, () => controller.abort());
  let error = null;
  try {
    await capture(sha, binary, output, controller.signal, views);
  } catch (failure) {
    error = failure.message;
  }
  const files = new Set(readdirSync(output));
  // A view passes once its PNG exists; a failure before any view (launch, readiness) fails them all.
  const features = views.map(view => ({
    feature: view.feature,
    view: view.id,
    file: `${view.id}.png`,
    status: files.has(`${view.id}.png`) && (error === null || !error.startsWith(`${view.id} `)) ? "pass" : "fail",
  }));
  const result = { sha, error, features, status: error === null && features.every(item => item.status === "pass") ? "pass" : "fix" };
  writeFileSync(join(output, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
  console.log(`linux-drive: ${result.status}${error ? ` (${error})` : ""}`);
  return result.status === "pass" ? 0 : 1;
}

if (require.main === module) {
  if (process.argv.length !== 5) {
    console.error("Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>");
    process.exitCode = 2;
  } else main(process.argv.slice(2)).then(code => { process.exitCode = code; }, failure => { console.error(`linux-drive: ${failure.message}`); process.exitCode = 2; });
}

module.exports = { DRIVEN, FEATURES, selectedViews };
