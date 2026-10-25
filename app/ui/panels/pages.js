"use strict";

(() => {
  const host = window.GrokbotPanels;
  if (!host) return;
  const allowed = new Set(["article", "button", "div", "em", "h1", "h2", "h3", "li", "ol", "p", "span", "strong", "table", "tbody", "td", "th", "thead", "tr", "ul"]);
  let shell = null;
  let selectedId = null;
  let pinnedId = null;
  let loadedKey = null;
  let loadedDocumentKey = null;
  let paintedSignature = null;
  let orphan = null;
  let lastState = null;
  let currentConversationId = null;
  let detail = null;
  let viewState = {};
  let full = false;
  let pendingTitle = null;
  let generation = 0;

  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };

  function ensureShell() {
    if (shell) return shell;
    const listAnchor = document.getElementById("conversation-list");
    const sidebar = listAnchor && listAnchor.parentElement;
    const main = document.querySelector(".main-panel");
    const dock = document.querySelector(".composer-dock");
    if (!sidebar || !main || !dock) return null;
    const section = node("section", "sidebar-pages");
    const heading = node("h2", "section-label", "Pages");
    heading.id = "pages-label";
    const list = node("nav", "page-list");
    list.setAttribute("aria-labelledby", "pages-label");
    section.append(heading, list);
    const marker = listAnchor.previousElementSibling;
    const before = marker && marker.classList && marker.classList.contains("sidebar-filter-row") ? marker : listAnchor;
    sidebar.insertBefore(section, before);

    const stage = node("aside", "page-stage");
    stage.setAttribute("aria-label", "Page");
    stage.hidden = true;
    const bar = node("div", "page-stage-bar");
    const rename = node("label", "page-rename", "Renommer");
    const input = node("input", "text-input");
    input.type = "text";
    input.maxLength = 80;
    input.setAttribute("aria-label", "Renommer");
    input.setAttribute("autocomplete", "off");
    rename.append(input);
    const fullButton = node("button", "secondary-button", "Ouvrir en pleine page");
    fullButton.type = "button";
    bar.append(rename, fullButton);
    const diagnostic = node("p", "page-diagnostic");
    diagnostic.hidden = true;
    const canvas = node("div", "page-canvas");
    stage.append(bar, diagnostic, canvas);
    main.insertBefore(stage, dock);
    input.addEventListener("change", submitRename);
    input.addEventListener("blur", submitRename);
    fullButton.addEventListener("click", () => {
      full = !full;
      applyChrome();
    });
    shell = { section, list, stage, input, fullButton, diagnostic, canvas, main };
    return shell;
  }

  function applyChrome() {
    if (!shell) return;
    const chat = document.getElementById("chat-scroll");
    const dock = document.querySelector(".composer-dock");
    shell.main.classList.toggle("page-full", full);
    if (chat) chat.hidden = full;
    if (dock) dock.hidden = full;
    shell.fullButton.textContent = full ? "Revenir au fil" : "Ouvrir en pleine page";
  }

  function conversationExists(id) {
    const conversations = lastState && Array.isArray(lastState.conversations) ? lastState.conversations : [];
    return conversations.some((conversation) => conversation.id === id);
  }

  function openPage(page, pages) {
    if (page.conversation_id && conversationExists(page.conversation_id)) {
      if (!host.openConversation(page.conversation_id)) {
        host.notify("Impossible d’ouvrir la conversation de cette Page pendant qu’une réponse est en cours.");
        return;
      }
      orphan = null;
      pinnedId = page.id;
    } else {
      // La conversation d’origine n’existe plus : la Page reste affichée tant que le fil ouvert ne change pas.
      pinnedId = null;
      orphan = { id: page.id, anchor: currentConversationId };
    }
    selectedId = page.id;
    paintList(pages);
    load(page);
  }

  function paintList(pages) {
    shell.list.replaceChildren();
    if (!pages.length) {
      shell.list.append(node("p", "sidebar-empty", "Aucune Page pour l’instant."));
      return;
    }
    for (const page of pages) {
      const button = node("button", "page-link", page.title || "Page");
      button.type = "button";
      button.dataset.pageId = page.id;
      if (page.id === selectedId) button.setAttribute("aria-current", "page");
      button.addEventListener("click", () => openPage(page, pages));
      shell.list.append(button);
    }
  }

  function choose(pages, conversationId) {
    if (orphan) {
      const kept = pages.find((page) => page.id === orphan.id);
      if (kept && orphan.anchor === (conversationId || null)) return kept;
      orphan = null;
    }
    if (!conversationId) {
      pinnedId = null;
      return null;
    }
    const pinned = pages.find((page) => page.id === pinnedId);
    if (pinned && pinned.conversation_id === conversationId) return pinned;
    pinnedId = null;
    return pages.find((page) => page.conversation_id === conversationId) || null;
  }

  const maxDepth = 64;
  // Keep aligned with compile.rs; reset for every paint, including state changes.
  const maxRenderVisits = 10000;
  const maxTextBytes = 1000000;
  const budgetMessage = "la Page dépasse le budget de rendu";

  function renderText(value, budget) {
    const text = value == null ? "" : String(value);
    // Count UTF-8 bytes without allocating an encoded copy of oversized text.
    for (const character of text) {
      const code = character.codePointAt(0);
      budget.bytes -= code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
      if (budget.bytes < 0) throw new Error(budgetMessage);
    }
    return document.createTextNode(text);
  }

  function place(parent, child) {
    if (Array.isArray(child)) {
      for (const item of child) place(parent, item);
      return;
    }
    if (child) parent.append(child);
  }

  function nests(spec) {
    return !!spec && (spec.type === "element" || spec.type === "map");
  }

  function pastCap(spec, depth) {
    if (!spec || typeof spec !== "object") return false;
    if (spec.type === "map") return pastCap(spec.node, depth);
    if (spec.type !== "element") return false;
    const children = Array.isArray(spec.children) ? spec.children : [];
    for (const child of children) {
      if (nests(child) && depth >= maxDepth) return true;
      if (pastCap(child, nests(child) ? depth + 1 : depth)) return true;
    }
    return false;
  }

  function render(spec, row, rowIndex, depth, budget) {
    if (budget.visits-- <= 0) throw new Error(budgetMessage);
    if (!spec || typeof spec !== "object") return null;
    if (nests(spec) && depth > maxDepth) return null;
    if (spec.type === "text") return renderText(spec.text, budget);
    if (spec.type === "state") {
      const value = viewState[spec.name];
      return renderText(value, budget);
    }
    if (spec.type === "field") {
      const value = row && spec.index != null ? row[spec.index] : "";
      return renderText(value, budget);
    }
    if (spec.type === "map") {
      const items = Array.isArray(spec.items) ? spec.items : [];
      return items.map((item, index) => render(spec.node, item, index, depth, budget));
    }
    if (spec.type !== "element" || !allowed.has(spec.tag)) return null;
    const el = document.createElement(spec.tag);
    if (spec.tag === "button") {
      el.type = "button";
      el.className = "page-choice";
    }
    if (spec.click && spec.tag === "button" && spec.click.state) {
      const action = spec.click;
      const current = row;
      const index = rowIndex;
      el.addEventListener("click", () => {
        let next = action.value;
        if (action.index != null && current) next = current[action.index];
        viewState[action.state] = next;
        paintCanvas(detail);
        const choices = shell.canvas.querySelectorAll("button");
        if (index != null && choices[index] && choices[index].focus) choices[index].focus();
      });
    }
    for (const child of spec.children || []) {
      if (nests(child) && depth >= maxDepth) return null;
      place(el, render(child, row, rowIndex, nests(child) ? depth + 1 : depth, budget));
    }
    return el;
  }

  const signature = (page) => `${sourceKey(page)}:${page && page.status ? page.status : ""}:${page && page.diagnostic ? page.diagnostic : ""}`;

  function paintCanvas(page) {
    paintedSignature = signature(page);
    const ready = page && page.status === "ready" && page.document && page.document.view;
    if (!ready) {
      shell.diagnostic.hidden = false;
      shell.diagnostic.textContent = (page && page.diagnostic) || "Cette Page n’a pas pu être affichée.";
      shell.canvas.replaceChildren();
      return;
    }
    if (pastCap(page.document.view, 1)) {
      shell.diagnostic.hidden = false;
      shell.diagnostic.textContent = "imbrication trop profonde";
      shell.canvas.replaceChildren();
      return;
    }
    shell.diagnostic.hidden = true;
    shell.diagnostic.textContent = "";
    shell.canvas.replaceChildren();
    try {
      const budget = { visits: maxRenderVisits, bytes: maxTextBytes };
      place(shell.canvas, render(page.document.view, null, null, 1, budget));
    } catch (error) {
      if (error.message !== budgetMessage) throw error;
      shell.diagnostic.hidden = false;
      shell.diagnostic.textContent = budgetMessage;
      shell.canvas.replaceChildren();
    }
  }

  function sourceKey(page) {
    const id = page && page.id ? page.id : "";
    const version = page && page.source_version ? page.source_version : "";
    return `${id}:${version}`;
  }

  function remember(page) {
    const key = sourceKey(page);
    const documentKey = page.document && !pastCap(page.document.view, 1) ? JSON.stringify(page.document) : null;
    const changed = loadedKey !== key || loadedDocumentKey !== documentKey;
    if (changed) {
      loadedKey = key;
      loadedDocumentKey = documentKey;
      viewState = {};
      const states = page.document && page.document.states;
      if (Array.isArray(states)) {
        for (const item of states) viewState[item.name] = item.initial;
      }
    }
    const previousTitle = detail && detail.title;
    detail = page;
    if (document.activeElement !== shell.input || shell.input.value === previousTitle) shell.input.value = page.title || "";
    // Un état identique (le fil se rafraîchit toutes les 200 ms pendant une réponse) ne reconstruit pas le DOM :
    // le focus clavier et la sélection de texte restent en place.
    if (changed || paintedSignature !== signature(page)) paintCanvas(page);
  }

  async function load(summary) {
    const token = ++generation;
    const id = summary.id;
    shell.stage.hidden = false;
    try {
      const response = await host.api(`/api/pages/${id}`);
      if (token !== generation || !response || !response.page) return;
      remember(response.page);
    } catch {
      if (token !== generation) return;
      remember({
        id,
        title: summary.title,
        status: "error",
        diagnostic: "Cette Page n’a pas pu être affichée.",
        document: null,
      });
    }
  }

  async function submitRename() {
    if (!shell || !selectedId) return;
    const title = shell.input.value.trim();
    if (!title || title.length > 80 || pendingTitle === title) return;
    if (detail && detail.id === selectedId && detail.title === title) return;
    const id = selectedId;
    const token = ++generation;
    pendingTitle = title;
    try {
      const result = await host.api("/api/pages/rename", { id, title });
      if (token !== generation || selectedId !== id) return;
      const next = result && result.page && result.page.title ? result.page.title : title;
      if (detail && detail.id === id && result && result.page) {
        remember(result.page);
        if (shell.input.value === title) shell.input.value = next;
      }
      const link = [...shell.list.children].find((item) => item.dataset && item.dataset.pageId === id);
      if (link) link.textContent = next;
    } catch (error) {
      host.notify(error && error.message ? error.message : "Le nom n’a pas pu être enregistré.");
    } finally {
      if (pendingTitle === title) pendingTitle = null;
    }
  }

  function hideStage() {
    selectedId = null;
    detail = null;
    loadedKey = null;
    loadedDocumentKey = null;
    paintedSignature = null;
    full = false;
    shell.stage.hidden = true;
    shell.canvas.replaceChildren();
    shell.diagnostic.textContent = "";
    shell.diagnostic.hidden = true;
    applyChrome();
  }

  async function onState(state, ctx) {
    const ui = ensureShell();
    if (!ui) return;
    lastState = state;
    const botId = ctx && ctx.bot && ctx.bot.id;
    const byBot = state && state.pages && state.pages.by_bot ? state.pages.by_bot : {};
    const conversationId = ctx && ctx.conversation ? ctx.conversation.id : null;
    currentConversationId = conversationId;
    const pages = Array.isArray(byBot[botId]) ? [...byBot[botId]] : [];
    // Dans un groupe, chaque participant écrit sous son propre identifiant : les Pages du fil comptent, quel que soit l’auteur.
    if (conversationId) {
      for (const bucket of Object.values(byBot)) {
        if (!Array.isArray(bucket)) continue;
        for (const page of bucket) {
          if (page.conversation_id === conversationId && !pages.some((known) => known.id === page.id)) pages.push(page);
        }
      }
    }
    const chosen = choose(pages, conversationId);
    selectedId = chosen ? chosen.id : null;
    paintList(pages);
    if (!chosen) {
      hideStage();
      return;
    }
    await load(chosen);
  }

  host.register({ id: "pages", onState });
})();
