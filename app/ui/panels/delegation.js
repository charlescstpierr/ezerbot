"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const active = (edge) => ["queued", "running"].includes(edge?.status?.status);
  const labels = { queued: "En file d’attente", running: "En cours", done: "Terminée", failed: "En échec", refused: "Refusée", cancelled: "Annulée" };
  let names = new Map();
  let conversations = new Map();
  let taskView = false;
  let edges = new Map();
  let navigationKey = "";
  const stopping = new Set();
  let stopButtons = new Map();
  let version = 0;
  // Presentation metadata only; results stay in the server-redacted transcript.
  window.GrokbotDelegation = {
    botName: (id) => names.get(id) || id || "Bot inconnu",
    hasConversation: (id) => conversations.has(id),
    version: () => version,
    edge: (conversation) => edges.get(conversation),
    taskId: (conversation) => edges.get(conversation)?.id,
    canFollowUp: (conversation) => active(edges.get(conversation)),
  };

  const style = document.createElement("style");
  style.textContent = `
    .gb-delegation-bar { flex: 0 0 auto; padding: 10px 22px; border-bottom: 1px solid var(--line); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
    .gb-delegation-bar[hidden] { display: none; }
    .gb-delegation-bar p { margin: 0; }
    .gb-delegation-nav { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }
    .gb-delegation-note { color: var(--muted); }
    .main-panel[data-delegation-task] #welcome { display: none; }
    .main-panel[data-delegation-finished] .composer-area { display: none; }
    @media (max-width: 900px) {
      .gb-delegation-note { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
    }
  `;
  document.head.append(style);
  const bar = document.createElement("section");
  bar.className = "gb-delegation-bar";
  bar.setAttribute("aria-label", "Sous-tâches déléguées");
  bar.hidden = true;
  const heading = document.createElement("p");
  heading.tabIndex = -1;
  const waiting = document.createElement("p");
  waiting.setAttribute("role", "status");
  waiting.setAttribute("aria-live", "polite");
  const note = document.createElement("p");
  note.className = "gb-delegation-note";
  const nav = document.createElement("nav");
  nav.className = "gb-delegation-nav";
  nav.setAttribute("aria-label", "Actions et navigation des sous-tâches");
  bar.append(heading, waiting, note, nav);
  const scroll = document.getElementById("chat-scroll");
  scroll?.before(bar);
  const main = scroll?.closest(".main-panel");

  function open(id) {
    if (!panels.openConversation(id)) panels.notify("Ce fil est indisponible ou une autre action est en cours.");
    else if (taskView) heading.focus(); // The shell otherwise focuses the hidden editor.
  }
  function focusView() {
    if (taskView) { heading.focus(); return; }
    const target = document.querySelector("#composer-input [contenteditable='true']") || document.getElementById("conversation-title");
    if (target) {
      if (!target.isContentEditable) target.tabIndex = -1;
      target.focus();
    }
  }
  function updateStops() {
    for (const [key, button] of stopButtons) {
      button.disabled = stopping.has(key);
      button.setAttribute("aria-busy", String(button.disabled));
    }
  }
  function stopButton(label, conversationId, runId, taskId) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "gb-retry";
    button.textContent = label;
    const key = JSON.stringify([conversationId, runId]);
    stopButtons.set(key, button);
    button.addEventListener("click", async () => {
      if (stopping.has(key)) return;
      const focused = document.activeElement === button;
      stopping.add(key);
      updateStops();
      // Disabled/removed controls must not leave keyboard focus on <body>.
      if (focused) focusView();
      try {
        // Both IDs are trusted host metadata; never infer a run from its title.
        if (taskId) await panels.api("/api/delegation/cancel", { parent_conversation_id: conversations.get(conversationId)?.parent_id, task_id: taskId });
        else await panels.api("/api/stop", { conversation_id: conversationId, message_id: runId });
        await panels.refresh();
        panels.notify(taskId ? "Sous-tâche annulée." : "Arrêt demandé.");
      } catch (error) { panels.notify(error.message || "L’arrêt n’a pas pu être confirmé."); }
      finally { stopping.delete(key); updateStops(); }
    });
    nav.append(button);
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

  function principalOf(conversation, state) {
    const botId = state.bots?.find((bot) => bot.is_default)?.id;
    if (!botId) return null;
    const principal = (item) => item && !item.deleted && item.kind === "chat" && item.bot_id === botId;
    const seen = new Set();
    for (let current = conversation; current && !seen.has(current.id); current = conversations.get(current.parent_id)) {
      seen.add(current.id);
      if (principal(current)) return current;
    }
    const time = (value) => Number(value) || Date.parse(value) || 0;
    return [...conversations.values()].filter(principal).sort((a, b) =>
      time(b.updated_at) - time(a.updated_at) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))[0];
  }

  panels.register({
    id: "delegation",
    onState(state, { conversation }) {
      names = new Map((state.bots || []).map((bot) => [bot.id, `${bot.name}${bot.deleted ? " (supprimé)" : ""}`]));
      conversations = new Map((state.conversations || []).map((item) => [item.id, item]));
      edges = new Map((state.delegation?.delegations || []).map((edge) => [edge.conversation_id, edge]));
      for (const parent of conversations.values()) {
        for (const message of parent.messages || []) {
          if (message.kind === "delegation" && !edges.has(message.payload?.conversation_id)) edges.set(message.payload?.conversation_id, message.payload);
        }
      }
      taskView = conversation?.kind === "task";
      main?.toggleAttribute("data-delegation-task", taskView);
      main?.toggleAttribute("data-delegation-finished", taskView && !active(edges.get(conversation?.id)));
      const count = state.delegation?.active_by_conversation?.[conversation?.id];
      const hasChildren = Number.isSafeInteger(count) && count > 0;
      bar.hidden = !taskView && !hasChildren;
      heading.hidden = !taskView;
      note.hidden = !taskView;
      waiting.hidden = !hasChildren;
      // REA DL-S07: child activity keeps the parent active, NOT composing.
      // Never manufacture a streaming message or change the shell's busy flag.
      const runs = state.engine?.runs || [];
      const reconnecting = state.provider?.connected !== true && (state.delegation?.delegations || []).some((edge) =>
        edge.parent_conversation_id === conversation?.id && active(edge)
        && runs.some((run) => run.run_id === edge.child_run_id && ["queued", "interrupted"].includes(run.status)));
      const waitingText = hasChildren ? `En attente de ${count} sous-tâche${count > 1 ? "s" : ""}${reconnecting ? " · reprise à la reconnexion" : ""}` : "";
      if (waiting.textContent !== waitingText) waiting.textContent = waitingText;
      const links = [];
      if (taskView) {
        // state.delegation is capped to recent edges; old transcript cards
        // remain authoritative for task navigation and status after reload.
        const edge = (state.delegation?.delegations || []).find((item) => item.conversation_id === conversation.id)
          || conversations.get(conversation.parent_id)?.messages
            ?.find((message) => message.kind === "delegation" && message.payload?.conversation_id === conversation.id)?.payload;
        const status = edge?.status?.status;
        const childRun = runs.find((run) => run.run_id === edge?.child_run_id);
        heading.textContent = `Fil de tâche · ${names.get(conversation.bot_id) || conversation.bot_id || "Bot inconnu"} · ${Object.hasOwn(labels, status) ? labels[status] : "État indisponible"}`;
        note.textContent = active(edge) ? "Suivis remis à cette sous-tâche au prochain appel au modèle. Approbations dans le fil du bot principal." : "Cette tâche est terminée ; poursuivez dans la conversation principale.";
        const root = rootOf(conversation);
        if (conversation.parent_id !== root?.id) links.push(["Retour au fil parent", conversation.parent_id]);
        if (root) links.push(["Conversation principale", root.id]);
        else note.textContent += " La conversation d’origine est indisponible.";
        // A non-default bot or a routine can be the root. Human requests have
        // a separate, authoritative principal-chat route (forms.rs).
        const request = (state.forms?.requests || []).find((item) => item.run_id === edge?.child_run_id
          && item.status?.status === "pending" && conversations.has(item.route?.target_conversation_id));
        if (request) links.push(["Répondre à la demande", request.route.target_conversation_id]);
        else {
          const principal = principalOf(conversation, state);
          if (principal && principal.id !== root?.id) links.push(["Conversation du bot principal", principal.id]);
        }
        if (active(edge)) {
          links.push([childRun?.status === "awaiting_children" ? "Annuler cette sous-tâche et ses sous-tâches" : "Annuler cette sous-tâche", conversation.id, edge.child_run_id || edge.id, edge.id]);
        }
      }
      note.title = note.textContent;
      if (hasChildren) {
        const parent = runs.find((run) => run.conversation_id === conversation.id && run.status === "awaiting_children");
        if (parent?.run_id) links.push(["Arrêter la réponse et ses sous-tâches", conversation.id, parent.run_id]);
      }
      // Keep keyboard focus while the shell polls active children.
      const stopTargets = new Set();
      const nextKey = JSON.stringify(links.filter(([, id, runId]) => {
        if (!conversations.has(id)) return false;
        if (!runId) return true;
        const key = JSON.stringify([id, runId]);
        if (stopTargets.has(key)) return false;
        stopTargets.add(key);
        return true;
      }));
      if (nextKey !== navigationKey) {
        navigationKey = nextKey;
        const focused = nav.contains(document.activeElement);
        nav.replaceChildren();
        stopButtons = new Map();
        for (const [label, id, runId, taskId] of JSON.parse(nextKey)) {
          if (runId) stopButton(label, id, runId, taskId);
          else link(label, id);
        }
        if (focused) focusView();
      }
      updateStops();
      version += 1;
      window.dispatchEvent(new Event("grokbot:delegation-change"));
    },
    wantsPolling({ state }) {
      // Mirror engine.js: disconnected interrupted/queued children cannot
      // progress. Keep durable waiting state, but don't poll it forever (DL-S10).
      const runs = new Map((state.engine?.runs || []).map((run) => [run.run_id, run]));
      const connected = state.provider?.connected === true;
      return (state.delegation?.delegations || []).some((edge) => active(edge)
        && (connected || ["running", "awaiting_children"].includes(runs.get(edge.child_run_id)?.status)))
        || (connected && Object.values(state.delegation?.active_by_conversation || {}).some((count) => Number.isSafeInteger(count) && count > 0));
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
