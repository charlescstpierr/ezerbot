"use strict";

(() => {
  const host = window.GrokbotPanels;
  if (!host) return;
  const allowed = new Set(["article", "button", "div", "em", "h1", "h2", "h3", "li", "ol", "p", "span", "strong", "table", "tbody", "td", "th", "thead", "tr", "ul"]);
  let shell = null;
  let selectedId = null;
  let pinnedId = null;
  let loadedId = null;
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
      button.addEventListener("click", () => {
        pinnedId = page.id;
        if (page.conversation_id) host.openConversation(page.conversation_id);
        selectedId = page.id;
        paintList(pages);
        load(page);
      });
      shell.list.append(button);
    }
  }

  function choose(pages, conversationId) {
    if (!conversationId) {
      pinnedId = null;
      return null;
    }
    const pinned = pages.find((page) => page.id === pinnedId);
    if (pinned && pinned.conversation_id === conversationId) return pinned;
    pinnedId = null;
    return pages.find((page) => page.conversation_id === conversationId) || null;
  }

  function place(parent, child) {
    if (Array.isArray(child)) {
      for (const item of child) place(parent, item);
      return;
    }
    if (child) parent.append(child);
  }

  function render(spec, row, rowIndex) {
    if (!spec || typeof spec !== "object") return null;
    if (spec.type === "text") return document.createTextNode(spec.text == null ? "" : String(spec.text));
    if (spec.type === "state") {
      const value = viewState[spec.name];
      return document.createTextNode(value == null ? "" : String(value));
    }
    if (spec.type === "field") {
      const value = row && spec.index != null ? row[spec.index] : "";
      return document.createTextNode(value == null ? "" : String(value));
    }
    if (spec.type === "map") {
      const items = Array.isArray(spec.items) ? spec.items : [];
      return items.map((item, index) => render(spec.node, item, index));
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
    for (const child of spec.children || []) place(el, render(child, row, rowIndex));
    return el;
  }

  function paintCanvas(page) {
    const ready = page && page.status === "ready" && page.document && page.document.view;
    if (!ready) {
      shell.diagnostic.hidden = false;
      shell.diagnostic.textContent = (page && page.diagnostic) || "Cette Page n’a pas pu être affichée.";
      shell.canvas.replaceChildren();
      return;
    }
    shell.diagnostic.hidden = true;
    shell.diagnostic.textContent = "";
    shell.canvas.replaceChildren();
    place(shell.canvas, render(page.document.view, null, null));
  }

  function remember(page) {
    if (loadedId !== page.id) {
      loadedId = page.id;
      viewState = {};
      const states = page.document && page.document.states;
      if (Array.isArray(states)) {
        for (const item of states) viewState[item.name] = item.initial;
      }
    }
    detail = page;
    if (document.activeElement !== shell.input) shell.input.value = page.title || "";
    paintCanvas(page);
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
    pendingTitle = title;
    try {
      const result = await host.api("/api/pages/rename", { id, title });
      const next = result && result.page && result.page.title ? result.page.title : title;
      if (detail && detail.id === id) detail.title = next;
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
    loadedId = null;
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
    const botId = ctx && ctx.bot && ctx.bot.id;
    const bucket = botId && state && state.pages && state.pages.by_bot ? state.pages.by_bot[botId] : null;
    const pages = Array.isArray(bucket) ? bucket : [];
    const conversationId = ctx && ctx.conversation ? ctx.conversation.id : null;
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
