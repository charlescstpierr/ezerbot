// Linux driver for verify-ezerbot: reuses tools/linux-captures.cjs (isolated wrapper, own Chrome
// profile, local fake provider) and drives three entries of the feature map under Xvfb.
// Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>
const { readdirSync, writeFileSync } = require("node:fs");
const { join, resolve } = require("node:path");
const { capture, findVisible, expectVisibleText } = require(resolve(__dirname, "../../../../tools/linux-captures.cjs"));

const QUESTION = "Bonjour, prépare une courte liste pour organiser ma semaine.";
const FOLLOW_UP = "Et en un mot ?";
const REPLY = "Voici votre document.";
const q = JSON.stringify;
const threadId = "GrokbotPanels.state.conversations.find(item => item.title?.startsWith('Fil'))?.id";

// Every content assertion below goes through expectVisibleText / findVisible (tools/linux-captures.cjs):
// what is asserted is what the user can see. Element interactions are real pointer clicks (`click`).
const QUESTION_MESSAGE = `findVisible('#chat-scroll .gb-user', ${q(QUESTION)})`;
const MENU_ITEMS = `'[role=menu][aria-label="Actions du message"] [role=menuitem]'`;
const REPLY_ITEM = `findVisible(${MENU_ITEMS}, 'Répondre en fil')`;
const SETTINGS = "findVisible('#settings-button')";
const MCP_PANEL = "'#settings-dialog[open] [data-panel=mcp]'";
const ADD_MCP = "findVisible('#settings-dialog[open] [data-panel=mcp] summary', 'Ajouter un serveur MCP')";
const notPending = `!GrokbotPanels.state.conversations.find(item => item.id === ${threadId})?.messages?.some(message => GrokbotChat.isPendingReply(message, false))`;

// The nine views, in order: id, how it is reached, and what must be visible before the capture.
const DRIVEN = Object.freeze([
  { id: "home-dark", label: "Accueil", action: "GrokbotPanels.leaveConversation()", visible: "findVisible('.gb-editor')",
    ready: "expectVisibleText('#conversation-title', 'Nouvelle conversation') && expectVisibleText('#welcome') && expectVisibleText('.gb-editor')" },
  { id: "conversation-dark", label: "Conversation factice", action: "GrokbotPanels.openConversation(window.captureConversationId)", visible: "findVisible('#conversation-title')",
    ready: `expectVisibleText('#conversation-title', 'Préparer la semaine') && expectVisibleText('#chat-scroll .gb-user', ${q(QUESTION)}) && expectVisibleText('#chat-scroll .gb-assistant', ${q(REPLY)})` },
  { id: "thread-menu", label: "Menu du message", visible: REPLY_ITEM, click: `${QUESTION_MESSAGE}?.querySelector('button[aria-label="Actions du message"]')`,
    ready: `expectVisibleText(${MENU_ITEMS}, 'Répondre en fil')` },
  { id: "thread-open", label: "Sous-fil ouvert", click: REPLY_ITEM, visible: "findVisible('#thread-banner-quote')",
    ready: `expectVisibleText('#thread-banner-quote', ${q(QUESTION.slice(0, 20))}) && expectVisibleText('#conversation-title', 'Fil') && !expectVisibleText('#chat-scroll .gb-message')` },
  { id: "thread-reply", label: "Réponse dans le sous-fil", visible: "findVisible('#thread-banner-quote')", action: `fetch('/api/send', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({conversation_id:${threadId}, message_id:'linux-verify-thread', body:${q(FOLLOW_UP)}})}).then(r => { if (!r.ok) throw Error('thread send failed'); return GrokbotPanels.refresh(); })`,
    ready: `expectVisibleText('#thread-banner-quote', ${q(QUESTION.slice(0, 20))}) && expectVisibleText('#chat-scroll .gb-user', ${q(FOLLOW_UP)}) && expectVisibleText('#chat-scroll .gb-assistant', ${q(REPLY)}) && ${notPending}` },
  { id: "thread-main", label: "Retour au fil principal", click: "findVisible('#thread-banner-open')", visible: "findVisible('#conversation-title')",
    ready: `!expectVisibleText('#thread-banner') && expectVisibleText('#conversation-title', 'Préparer la semaine') && expectVisibleText('#chat-scroll .gb-user', ${q(QUESTION)}) && !expectVisibleText('#chat-scroll .gb-message', ${q(FOLLOW_UP)})` },
  { id: "settings-dark", label: "Réglages", click: SETTINGS, visible: "findVisible('#settings-dialog[open] #settings-appearance h3', 'Apparence')",
    ready: "expectVisibleText('#settings-dialog[open] #settings-appearance h3', 'Apparence')" },
  { id: "settings-connectors", label: "Paramètres, section Connecteurs", keepScroll: true, click: SETTINGS,
    after: "document.querySelector('#settings-dialog [data-panel=mcp]')?.scrollIntoView({block:'start'})",
    visible: `findVisible(${MCP_PANEL} + ' h3', 'Connecteurs')`,
    ready: `expectVisibleText(${MCP_PANEL} + ' h3', 'Connecteurs') && expectVisibleText(${MCP_PANEL} + ' summary', 'Ajouter un serveur MCP')` },
  { id: "settings-connectors-add", label: "Connecteurs, ajouter un serveur MCP", keepScroll: true, click: SETTINGS,
    after: "[...document.querySelectorAll('#settings-dialog[open] [data-panel=mcp] summary')].find(node => node.innerText.includes('Ajouter un serveur MCP'))?.scrollIntoView({block:'center'})",
    visible: ADD_MCP,
    ready: `expectVisibleText(${MCP_PANEL} + ' summary', 'Ajouter un serveur MCP')` },
]);

// Entry of features/ -> views that must all be captured for the entry to pass.
const FEATURES = Object.freeze([
  { feature: "accueil", views: ["home-dark"] },
  { feature: "reponse-fil", views: ["conversation-dark", "thread-menu", "thread-open", "thread-reply", "thread-main"] },
  { feature: "parametres-connecteurs", views: ["settings-dark", "settings-connectors", "settings-connectors-add"] },
]);

// Page-side helpers in scope for every expression a view evaluates.
const inPage = expression => `(() => { const findVisible = ${findVisible}; const expectVisibleText = ${expectVisibleText}; return (${expression}); })()`;

function selectedViews() {
  return FEATURES.flatMap(({ feature, views }) => views.map(id => {
    const found = DRIVEN.find(item => item.id === id);
    if (!found) throw new Error(`View ${id} is missing.`);
    const view = { ...found, feature };
    for (const key of ["ready", "visible", "click"]) if (view[key]) view[key] = inPage(view[key]);
    return view;
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
