"use strict";

(() => {
  const host = window.GrokbotPanels;
  if (!host) return;
  const allowed = new Set(["article", "button", "div", "em", "h1", "h2", "h3", "li", "ol", "p", "span", "strong", "table", "tbody", "td", "th", "thead", "tr", "ul"]);
  let shell = null;
  let selectedId = null;
  let pinnedId = null;
  let loadedKey = null;
  let paintedSignature = null;
  let orphan = null;
  let lastState = null;
  let currentConversationId = null;
  let detail = null;
  let viewState = {};
  let full = false;
  let pendingTitle = null;
  let generation = 0;
  let knownPages = [];
  let quote = null;
  let quoteSerial = 0;
  let candidate = null;
  const passageLimit = 500;

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
    const comment = node("button", "secondary-button page-comment", "Commenter");
    comment.type = "button";
    comment.hidden = true;
    stage.append(bar, diagnostic, canvas, comment);
    main.insertBefore(stage, dock);
    // Au-dessus de la boîte de message : la citation part avec le prochain envoi.
    const quoteBar = node("div", "gb-quote-chip");
    quoteBar.hidden = true;
    const area = document.querySelector(".composer-area") || dock;
    area.insertBefore(quoteBar, area.firstChild);
    document.addEventListener("selectionchange", updateComment);
    canvas.addEventListener("scroll", updateComment);
    // Cliquer le bouton ne doit pas défaire la sélection avant que le passage soit lu.
    comment.addEventListener("mousedown", (event) => event.preventDefault());
    comment.addEventListener("click", startQuote);
    comment.addEventListener("blur", updateComment);
    input.addEventListener("change", submitRename);
    input.addEventListener("blur", submitRename);
    fullButton.addEventListener("click", () => {
      full = !full;
      applyChrome();
    });
    shell = { section, list, stage, input, fullButton, diagnostic, canvas, comment, quoteBar, main };
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
      // Refusée, la navigation laisse la Page affichée et sa citation telles quelles.
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
    if (selectedId !== page.id) { quoteSerial++; clearQuote(); }
    selectedId = page.id;
    paintList(pages);
    load(page);
  }

  function showPage(id) {
    const page = knownPages.find((item) => item.id === id);
    if (!page) {
      host.notify("Cette Page n’existe plus.");
      return;
    }
    if (selectedId !== page.id || shell.stage.hidden) openPage(page, knownPages);
    shell.stage.scrollIntoView({ block: "nearest" });
  }

  // L'envoi d'un suivi de sous-tâche (/api/delegation/send) n'emporte pas d'ancre : on ne propose pas de citer.
  function inTask() {
    const conversations = lastState && Array.isArray(lastState.conversations) ? lastState.conversations : [];
    return conversations.some((conversation) => conversation.id === currentConversationId && conversation.kind === "task");
  }

  // Une Page d'un autre bot que celui du fil (groupe) est refusée par le moteur (409) : on ne propose pas de citer.
  function foreignPage() {
    const conversations = lastState && Array.isArray(lastState.conversations) ? lastState.conversations : [];
    const thread = conversations.find((conversation) => conversation.id === currentConversationId);
    return Boolean(thread && thread.bot_id && detail && detail.bot_id && thread.bot_id !== detail.bot_id);
  }

  // Le moteur vérifie le passage dans le texte affiché de la Page à l'état initial, chaque suite de blancs (Unicode
  // White_Space) réduite à un espace : la même règle que `collapsed` dans anchors.rs. Un blanc au milieu d'un mot, ou
  // absent entre deux cellules (l'aperçu en met un à chaque frontière de bloc), ne passe pas.
  const collapsed = (text) => String(text).split(/\p{White_Space}+/u).filter(Boolean).join(" ");

  // Ce que le moteur accepterait pour la Page affichée maintenant : rien dans une sous-tâche ni pour la Page d'un autre
  // bot, et un passage rogné comme le moteur le rogne, de 1 à 500 caractères, présent dans l'aperçu. Rejoué au clic, car
  // la Page peut changer (modification par le bot, autre Page) pendant que « Commenter » garde le focus clavier.
  function quotable(text) {
    if (inTask() || !detail || foreignPage() || detail.id !== selectedId || shell.stage.hidden) return null;
    // Même rognage que `str::trim` de Rust (propriété Unicode White_Space), pas `String.trim`.
    const passage = String(text).replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
    if (!passage || [...passage].length > passageLimit) return null;
    // Un texte changé par l'état interactif de la Page (« Offre choisie : Confort ») n'est pas dans l'aperçu que le moteur contrôle.
    if (!collapsed(detail.preview || "").includes(collapsed(passage))) return null;
    return passage;
  }

  // Le passage est le texte sélectionné ; la sélection doit tenir toute dans le canevas.
  function selectedPassage() {
    const selection = document.getSelection();
    if (!selection || selection.rangeCount !== 1 || selection.isCollapsed) return null;
    const range = selection.getRangeAt(0);
    if (!shell.canvas.contains(range.startContainer) || !shell.canvas.contains(range.endContainer)) return null;
    const passage = quotable(selection.toString());
    if (!passage) return null;
    const target = range.getBoundingClientRect();
    const view = shell.canvas.getBoundingClientRect();
    if (target.bottom < view.top || target.top > view.bottom) return null;
    return { passage, target };
  }

  function updateComment() {
    // Tab vers « Commenter » peut défaire la sélection dans certains navigateurs : le bouton reste tant qu’il a le focus.
    if (!shell || document.activeElement === shell.comment) return;
    const found = selectedPassage();
    candidate = found && { passage: found.passage, page_id: detail.id };
    shell.comment.hidden = !found;
    if (!found) return;
    const stage = shell.stage.getBoundingClientRect();
    const left = found.target.left - stage.left;
    const top = found.target.bottom - stage.top + 6;
    shell.comment.style.left = `${Math.max(8, Math.min(left, stage.width - shell.comment.offsetWidth - 8))}px`;
    shell.comment.style.top = `${Math.max(8, Math.min(top, stage.height - shell.comment.offsetHeight - 8))}px`;
  }

  function paintQuote() {
    shell.quoteBar.replaceChildren();
    shell.quoteBar.hidden = !quote;
    if (!quote) return;
    const label = `Page « ${quote.title} » — « ${quote.passage.replace(/\s+/g, " ")} »`;
    const text = node("span", "gb-quote-text", label);
    text.setAttribute("title", label);
    const remove = node("button", "gb-quote-remove");
    remove.type = "button";
    remove.setAttribute("aria-label", "Retirer la citation");
    remove.innerHTML = '<svg class="icon" aria-hidden="true"><use href="#i-close"/></svg>';
    remove.addEventListener("click", () => { quoteSerial++; clearQuote(); });
    shell.quoteBar.append(text, remove);
  }

  function clearQuote() {
    quote = null;
    paintQuote();
  }

  // La puce nomme la Page par son titre actuel : un renommage (ici ou ailleurs) la repeint.
  function retitleQuote(id, title) {
    if (!quote || quote.page_id !== id || !title || quote.title === title) return;
    quote.title = title;
    paintQuote();
  }

  function startQuote() {
    const passage = candidate && detail && candidate.page_id === detail.id ? quotable(candidate.passage) : null;
    candidate = null;
    shell.comment.hidden = true;
    if (!passage) return;
    quote = { serial: ++quoteSerial, page_id: detail.id, title: detail.title, source_version: detail.source_version, passage };
    document.getSelection().removeAllRanges();
    // La boîte de message est cachée en pleine page : on revient au fil pour écrire la demande.
    if (full) {
      full = false;
      applyChrome();
    }
    paintQuote();
    const editor = document.querySelector("#composer-input [contenteditable='true']");
    if (editor) editor.focus();
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
    const pinned = pages.find((page) => page.id === (quote?.page_id || pinnedId));
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
    if (loadedKey !== key) {
      loadedKey = key;
      viewState = {};
      const states = page.document && page.document.states;
      if (Array.isArray(states)) {
        for (const item of states) viewState[item.name] = item.initial;
      }
    }
    detail = page;
    retitleQuote(page.id, page.title);
    if (document.activeElement !== shell.input) shell.input.value = page.title || "";
    // Un état identique (le fil se rafraîchit toutes les 200 ms pendant une réponse) ne reconstruit pas le DOM :
    // le focus clavier et la sélection de texte restent en place.
    if (paintedSignature !== signature(page)) paintCanvas(page);
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
      retitleQuote(id, next);
      const link = [...shell.list.children].find((item) => item.dataset && item.dataset.pageId === id);
      if (link) link.textContent = next;
      // La liste se repeint depuis l'état de l'hôte : sans le relire, le repeint suivant remettrait l'ancien titre.
      try { await host.refresh(); } catch { /* le prochain rafraîchissement relira la liste */ }
    } catch (error) {
      host.notify(error && error.message ? error.message : "Le nom n’a pas pu être enregistré.");
    } finally {
      if (pendingTitle === title) pendingTitle = null;
    }
  }

  function hideStage() {
    generation++;
    selectedId = null;
    detail = null;
    loadedKey = null;
    paintedSignature = null;
    full = false;
    candidate = null;
    shell.comment.hidden = true;
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
    if (host.busy && !currentConversationId && conversationId && orphan?.anchor === null
        && quote?.page_id === orphan.id && ctx.conversation.kind !== "task"
        && ctx.conversation.bot_id === botId && detail?.bot_id === botId) {
      orphan.anchor = conversationId;
    }
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
    knownPages = pages;
    const chosen = choose(pages, conversationId);
    selectedId = chosen ? chosen.id : null;
    if (quote && (!chosen || chosen.id !== quote.page_id || inTask() || foreignPage())) clearQuote();
    paintList(pages);
    if (!chosen) {
      hideStage();
      return;
    }
    if (host.connected !== false) await load(chosen);
  }

  host.register({
    id: "pages",
    onState,
    openPage: showPage,
    sendExtras: () => quote ? { page_anchor: { page_id: quote.page_id, source_version: quote.source_version, passage: quote.passage } } : {},
    composer: {
      // L'accusé ne retire que la citation partie avec ce message, jamais une citation posée depuis, même passage compris.
      capture: () => quote ? { ...quote } : null,
      async restore({ captured }, ctx) {
        if (!captured || captured.serial !== quoteSerial) return;
        const contextId = ctx.conversation?.id || null;
        const botId = ctx.bot?.id;
        const page = knownPages.find((item) => item.id === captured.page_id);
        const unavailable = (reason) => { clearQuote(); return `La citation n’a pas pu être reprise : ${reason}`; };
        if (!page) return unavailable("la Page n’est plus disponible dans cette conversation.");
        let restored;
        try { restored = (await host.api(`/api/pages/${page.id}`)).page; }
        catch {
          if (captured.serial !== quoteSerial || currentConversationId !== contextId) return;
          return "La citation reste à vérifier : la Page n’a pas pu être relue. Réessayez quand elle sera disponible.";
        }
        if (captured.serial !== quoteSerial || currentConversationId !== contextId) return;
        if (!knownPages.some((item) => item.id === captured.page_id)) return unavailable("la Page n’est plus disponible dans cette conversation.");
        if (inTask() || restored?.bot_id !== botId) return unavailable("son bot ne correspond plus au brouillon.");
        if (restored.status !== "ready" || !collapsed(restored.preview || "").includes(collapsed(captured.passage))) {
          return unavailable("le passage n’apparaît plus dans la Page actuelle. Sélectionnez un nouveau passage.");
        }
        orphan = { id: page.id, anchor: contextId };
        selectedId = page.id;
        shell.stage.hidden = false;
        remember(restored);
        quote = { ...captured, title: restored.title };
        paintQuote();
      },
      onSent({ captured }) {
        if (quote && captured?.serial === quote.serial) clearQuote();
      },
    },
  });
})();
