// Linux driver for verify-ezerbot: reuses tools/linux-captures.cjs (isolated wrapper, own Chrome
// profile, local fake provider) and drives three entries of the feature map under Xvfb.
// Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>
const { readdirSync, writeFileSync } = require("node:fs");
const { join, resolve } = require("node:path");
const { VIEWS, capture, pointerTarget } = require(resolve(__dirname, "../../../../tools/linux-captures.cjs"));

const QUESTION = "Bonjour, prépare une courte liste pour organiser ma semaine.";
const FOLLOW_UP = "Et en un mot ?";
const question = "[...document.querySelectorAll('#chat-scroll .gb-message')].find(node => node.textContent.includes(" + JSON.stringify(QUESTION) + "))";
const threadId = "GrokbotPanels.state.conversations.find(item => item.title?.startsWith('Fil'))?.id";

const REPLY_ITEM = `[...document.querySelectorAll('[role=menu][aria-label="Actions du message"] [role=menuitem]')].find(item => item.textContent === 'Répondre en fil')`;
const SETTINGS = "document.getElementById('settings-button')";
const ADD_MCP_NAME = "document.querySelector('#settings-dialog[open] details.mcp-add[open] form input[name=name]')";
const ADD_MCP = "[...document.querySelectorAll('#settings-dialog[open] [data-panel=mcp] summary')].find(item => item.textContent.includes('Ajouter un serveur MCP'))";

// Views driven for each entry of features/, in order. The first two come from tools/linux-captures.cjs.
const DRIVEN = Object.freeze([
  { id: "thread-menu", label: "Menu du message", visible: REPLY_ITEM, click: `${question}?.querySelector('button[aria-label="Actions du message"]')`,
    ready: `[...document.querySelectorAll('[role=menu][aria-label="Actions du message"] [role=menuitem]')].some(item => item.textContent === 'Répondre en fil' && (${pointerTarget})(item))` },
  { id: "thread-open", label: "Sous-fil ouvert", click: REPLY_ITEM, visible: "document.getElementById('thread-banner-quote')",
    ready: `!document.getElementById('thread-banner')?.hidden && document.getElementById('thread-banner-quote')?.textContent.includes(${JSON.stringify(QUESTION.slice(0, 20))}) && document.getElementById('conversation-title')?.textContent.startsWith('Fil') && document.querySelectorAll('#chat-scroll .gb-message').length === 0` },
  { id: "thread-reply", label: "Réponse dans le sous-fil", visible: "document.getElementById('thread-banner-quote')", action: `fetch('/api/send', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({conversation_id:${threadId}, message_id:'linux-verify-thread', body:${JSON.stringify(FOLLOW_UP)}})}).then(r => { if (!r.ok) throw Error('thread send failed'); return GrokbotPanels.refresh(); })`,
    ready: `!document.getElementById('thread-banner')?.hidden && document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(FOLLOW_UP)}) && document.getElementById('chat-scroll')?.textContent.includes('Voici votre document.') && !GrokbotPanels.state.conversations.find(item => item.id === ${threadId})?.messages?.some(message => GrokbotChat.isPendingReply(message, false))` },
  { id: "thread-main", label: "Retour au fil principal", click: "document.getElementById('thread-banner-open')", visible: "document.getElementById('conversation-title')",
    ready: `document.getElementById('thread-banner')?.hidden && document.getElementById('conversation-title')?.textContent === 'Préparer la semaine' && document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(QUESTION)}) && !document.getElementById('chat-scroll')?.textContent.includes(${JSON.stringify(FOLLOW_UP)})` },
  { id: "settings-dark", label: "Réglages", click: SETTINGS, visible: "document.querySelector('#settings-dialog[open] #settings-appearance h3')",
    ready: "document.getElementById('settings-dialog')?.open && document.querySelector('#settings-dialog .setting-section')" },
  { id: "settings-connectors", label: "Paramètres, section Connecteurs", keepScroll: true, click: SETTINGS,
    after: "document.querySelector('#settings-dialog [data-panel=mcp]')?.scrollIntoView({block:'start'})",
    visible: "document.querySelector('#settings-dialog[open] [data-panel=mcp] h3')",
    ready: "(() => { const panel = document.querySelector('#settings-dialog[open] [data-panel=mcp]'); return panel?.querySelector('h3')?.textContent === 'Connecteurs' && [...panel.querySelectorAll('summary')].some(item => item.textContent.includes('Ajouter un serveur MCP')); })()" },
  // Real pointer click on Réglages, then on « Ajouter un serveur MCP » (scrolled into view first); the form must open.
  { id: "settings-connectors-add", label: "Connecteurs, ajouter un serveur MCP", keepScroll: true,
    click: [SETTINGS, `(() => { const summary = ${ADD_MCP}; summary?.scrollIntoView({block:'center'}); return summary; })()`],
    visible: ADD_MCP_NAME,
    ready: `${ADD_MCP}?.parentElement?.open === true && Boolean(${ADD_MCP_NAME})` },
]);

// Entry of features/ -> views that must all be captured for the entry to pass.
const FEATURES = Object.freeze([
  { feature: "accueil", views: ["home-dark"] },
  { feature: "reponse-fil", views: ["conversation-dark", "thread-menu", "thread-open", "thread-reply", "thread-main"] },
  { feature: "parametres-connecteurs", views: ["settings-dark", "settings-connectors", "settings-connectors-add"] },
]);

// Views reused from tools/linux-captures.cjs are reached by app API calls (no element click); their result must still be visible.
const SHARED_VISIBLE = Object.freeze({ "home-dark": "document.querySelector('.gb-editor')", "conversation-dark": "document.getElementById('conversation-title')" });

function selectedViews() {
  const registry = [...DRIVEN, ...VIEWS.map(view => ({ ...view, visible: SHARED_VISIBLE[view.id] }))];
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
    await capture(sha, binary, output, controller.signal, views, []);
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
