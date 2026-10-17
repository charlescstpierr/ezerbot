"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const active = (edge) => ["queued", "running"].includes(edge?.status?.status);
  const labels = { queued: "En file d’attente", running: "En cours", done: "Terminée", failed: "En échec", refused: "Refusée", cancelled: "Annulée" };
  let names = new Map();
  let conversations = new Map();
  let taskView = false;
  let navigationKey = "";
  let version = 0;
  // Presentation metadata only; results stay in the server-redacted transcript.
  window.GrokbotDelegation = {
    botName: (id) => names.get(id) || id || "Bot inconnu",
    hasConversation: (id) => conversations.has(id),
    version: () => version,
  };

  const style = document.createElement("style");
  style.textContent = `
    .gb-delegation-bar { flex: 0 0 auto; padding: 10px 22px; border-bottom: 1px solid var(--line); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
    .gb-delegation-bar[hidden] { display: none; }
    .gb-delegation-bar p { margin: 0; }
    .gb-delegation-nav { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }
    .gb-delegation-note { color: var(--muted); }
    .main-panel[data-delegation-task] .composer-area { display: none; }
  `;
  document.head.append(style);
  const bar = document.createElement("section");
  bar.className = "gb-delegation-bar";
  bar.setAttribute("aria-label", "Sous-tâches déléguées");
  bar.hidden = true;
  const heading = document.createElement("p");
  const waiting = document.createElement("p");
  waiting.setAttribute("role", "status");
  waiting.setAttribute("aria-live", "polite");
  const note = document.createElement("p");
  note.className = "gb-delegation-note";
  const nav = document.createElement("nav");
  nav.className = "gb-delegation-nav";
  nav.setAttribute("aria-label", "Navigation du fil de tâche");
  bar.append(heading, waiting, note, nav);
  const scroll = document.getElementById("chat-scroll");
  scroll?.before(bar);
  const main = scroll?.closest(".main-panel");

  // REA delegation-lifecycle-deep.md DL-S05/S06 keeps taskId and parent
  // session separate on send. The local API has no task-steering endpoint:
  // show a read-only task transcript, never fork a new chat in that namespace
  // (also avoids the completed-task nesting defect from wrapper review r3).
  document.getElementById("composer-form")?.addEventListener("submit", (event) => {
    if (!taskView) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    panels.notify("Ce fil de tâche est en lecture seule ; écrivez dans la conversation principale.");
  }, true);

  function open(id) {
    if (!panels.openConversation(id)) panels.notify("Ce fil est indisponible ou une autre action est en cours.");
  }
  function link(label, id) {
    if (!conversations.has(id)) return;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "gb-retry";
    button.textContent = label;
    button.addEventListener("click", () => open(id));
    nav.append(button);
  }
  function rootOf(conversation) {
    const seen = new Set([conversation.id]);
    let current = conversation;
    while (current.parent_id && !seen.has(current.parent_id)) {
      const parent = conversations.get(current.parent_id);
      if (!parent) return null;
      seen.add(parent.id);
      current = parent;
    }
    return current.kind === "task" ? null : current;
  }

  panels.register({
    id: "delegation",
    onState(state, { conversation }) {
      names = new Map((state.bots || []).map((bot) => [bot.id, `${bot.name}${bot.deleted ? " (supprimé)" : ""}`]));
      conversations = new Map((state.conversations || []).map((item) => [item.id, item]));
      taskView = conversation?.kind === "task";
      main?.toggleAttribute("data-delegation-task", taskView);
      const count = state.delegation?.active_by_conversation?.[conversation?.id];
      const hasChildren = Number.isSafeInteger(count) && count > 0;
      bar.hidden = !taskView && !hasChildren;
      heading.hidden = !taskView;
      note.hidden = !taskView;
      waiting.hidden = !hasChildren;
      // REA DL-S07: child activity keeps the parent active, NOT composing.
      // Never manufacture a streaming message or change the shell's busy flag.
      const waitingText = hasChildren ? `En attente de ${count} sous-tâche${count > 1 ? "s" : ""}` : "";
      if (waiting.textContent !== waitingText) waiting.textContent = waitingText;
      const links = [];
      if (taskView) {
        // state.delegation is capped to recent edges; old transcript cards
        // remain authoritative for task navigation and status after reload.
        const edge = (state.delegation?.delegations || []).find((item) => item.conversation_id === conversation.id)
          || conversations.get(conversation.parent_id)?.messages
            ?.find((message) => message.kind === "delegation" && message.payload?.conversation_id === conversation.id)?.payload;
        const status = edge?.status?.status;
        heading.textContent = `Fil de tâche · ${names.get(conversation.bot_id) || conversation.bot_id || "Bot inconnu"} · ${Object.hasOwn(labels, status) ? labels[status] : "État indisponible"}`;
        note.textContent = "Lecture seule. Écrivez et répondez aux demandes d’approbation dans la conversation principale.";
        const root = rootOf(conversation);
        if (conversation.parent_id !== root?.id) links.push(["Retour au fil parent", conversation.parent_id]);
        if (root) links.push(["Conversation principale", root.id]);
        else note.textContent += " La conversation principale est indisponible.";
      }
      // Keep keyboard focus while the shell polls active children.
      const nextKey = JSON.stringify(links.filter(([, id]) => conversations.has(id)));
      if (nextKey !== navigationKey) {
        navigationKey = nextKey;
        nav.replaceChildren();
        for (const [label, id] of JSON.parse(nextKey)) link(label, id);
      }
      version += 1;
      window.dispatchEvent(new Event("grokbot:delegation-change"));
    },
    wantsPolling({ state }) {
      // Include hidden child threads and queued reservations; a parent reply
      // can have stopped streaming while its children still need reconciliation.
      return (state.delegation?.delegations || []).some(active)
        || Object.values(state.delegation?.active_by_conversation || {}).some((count) => Number.isSafeInteger(count) && count > 0);
    },
    async onCardAction(message, action) {
      if (message.kind !== "delegation" || action !== "delegation.open") return false;
      // Use only the server's card target, never a caller-supplied detail/URL.
      const id = message.payload?.conversation_id;
      if (!conversations.has(id)) await panels.refresh();
      if (!conversations.has(id)) panels.notify("Ce fil de tâche n’est plus disponible.");
      else open(id);
      return true;
    },
  });
})();
