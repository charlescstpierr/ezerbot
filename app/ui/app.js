"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const ui = {
    list: $("conversation-list"), search: $("search"), input: $("composer-input"),
    send: $("send-button"), welcome: $("welcome"), messages: $("messages"),
    scroll: $("chat-scroll"), title: $("conversation-title"), menu: $("conversation-menu"),
    menuButton: $("conversation-menu-button"), hint: $("composer-hint"),
    banner: $("connection-banner"), bannerText: $("connection-message"), toast: $("toast"),
  };
  let state = { account: { name: "Espace personnel" }, mode: "simulated", conversations: [] };
  let selectedId = readPreference("grokbot.selected", null);
  let favoriteOnly = false;
  let connected = false;
  let connectionStatus = "loading";
  let busy = false;
  let submissionActivity = null;
  const mascot = window.GrokbotMascot;
  let toastTimer;
  let simulateLoss = false;
  let themeChoice = readPreference("grokbot.theme", "light");
  let preferencesLoaded = false;
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  const pendingStates = new Set(["queued", "dispatching", "unknown"]);

  function readPreference(key, fallback) {
    try { return localStorage.getItem(key) ?? fallback; } catch { return fallback; }
  }

  function writePreference(key, value) {
    try { localStorage.setItem(key, value); } catch { /* The chat still works when browser storage is unavailable. */ }
  }

  function removePreference(key) {
    try { localStorage.removeItem(key); } catch { /* SQLite remains the source of conversation history. */ }
  }

  function draftKey(id = selectedId) { return `grokbot.draft.${id || "new"}`; }
  function currentConversation() { return state.conversations.find((item) => item.id === selectedId); }

  function icon(name, className = "icon") {
    const element = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    element.setAttribute("class", className);
    element.setAttribute("aria-hidden", "true");
    const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
    use.setAttribute("href", `#i-${name}`);
    element.append(use);
    return element;
  }

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function timestamp(value) {
    if (typeof value === "number") return new Date(value < 1e12 ? value * 1000 : value);
    if (typeof value === "string" && /^\d+$/.test(value)) return timestamp(Number(value));
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? new Date() : date;
  }

  function notify(message) {
    clearTimeout(toastTimer);
    ui.toast.textContent = message;
    ui.toast.hidden = false;
    toastTimer = setTimeout(() => { ui.toast.hidden = true; }, 4000);
  }

  async function copyText(text) {
    if (navigator.clipboard?.writeText) {
      try { await navigator.clipboard.writeText(text); return; } catch { /* Native WebKit may require the selection fallback. */ }
    }
    const textarea = element("textarea", "visually-hidden");
    textarea.value = text;
    textarea.setAttribute("readonly", "");
    document.body.append(textarea);
    textarea.select();
    try { if (!document.execCommand("copy")) throw new Error("Copy unavailable"); }
    finally { textarea.remove(); }
  }

  async function api(path, body) {
    const options = body === undefined ? {} : {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" },
      body: JSON.stringify(body),
    };
    let response;
    try { response = await fetch(path, options); }
    catch (error) {
      showConnectionError(error);
      throw new Error("La connexion au moteur local est interrompue.");
    }
    let data;
    try { data = await response.json(); } catch { throw new Error("Le moteur local a renvoyé une réponse illisible."); }
    if (!response.ok) throw new Error(data.error || "L’action n’a pas pu être terminée.");
    return data;
  }

  async function refresh() {
    state = await api("/api/state");
    if (!preferencesLoaded) {
      preferencesLoaded = true;
      applyTheme(state.preferences?.theme || themeChoice);
    }
    connected = true;
    connectionStatus = "online";
    ui.banner.hidden = true;
    if (selectedId && !state.conversations.some((item) => item.id === selectedId)) selectedId = null;
    writePreference("grokbot.selected", selectedId || "");
    render();
  }

  function showConnectionError(error) {
    connected = false;
    connectionStatus = "offline";
    ui.bannerText.textContent = "Le moteur local ne répond plus. Vos conversations enregistrées sont conservées.";
    ui.banner.hidden = false;
    updateComposer();
    if (error) console.warn("Grokbot local connection:", error.message);
  }

  function upsertConversation(conversation) {
    const index = state.conversations.findIndex((item) => item.id === conversation.id);
    if (index === -1) state.conversations.unshift(conversation);
    else state.conversations[index] = conversation;
    connected = true;
    connectionStatus = "online";
    ui.banner.hidden = true;
  }

  function saveDraft() {
    if (ui.input.value) writePreference(draftKey(), ui.input.value);
    else removePreference(draftKey());
  }

  function resizeComposer() {
    ui.input.style.height = "auto";
    ui.input.style.height = `${Math.min(ui.input.scrollHeight, 180)}px`;
  }

  function updateComposer() {
    const pendingMessage = currentConversation()?.messages.find((message) => message.role === "user" && pendingStates.has(message.state));
    const pending = Boolean(pendingMessage);
    const tooLong = new TextEncoder().encode(ui.input.value.trim()).length > 16 * 1024;
    ui.send.disabled = busy || !connected || !ui.input.value.trim() || pending || tooLong;
    ui.send.setAttribute("aria-label", busy ? "Envoi en cours" : "Envoyer le message");
    ui.input.setAttribute("aria-invalid", String(tooLong));
    ui.hint.dataset.alert = String(Boolean(pending || tooLong));
    ui.hint.textContent = pending ? "Vérifiez le dernier envoi pour poursuivre la conversation." : tooLong ? "Ce message est trop long. Raccourcissez-le pour l’envoyer." : "Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne";
    $("new-chat").disabled = busy;
    ui.menuButton.disabled = busy || !currentConversation();
    mascot?.setContext({ connection: connected ? "online" : connectionStatus,
      activity: submissionActivity, pending: pendingMessage?.state || null, drafting: Boolean(ui.input.value.trim()) });
    resizeComposer();
  }

  function selectConversation(id, focus = false) {
    saveDraft();
    selectedId = id;
    writePreference("grokbot.selected", id || "");
    ui.input.value = readPreference(draftKey(), "");
    closeMenu();
    closeSidebar();
    render();
    scrollToLatest();
    if (focus) ui.input.focus();
  }

  function renderList() {
    ui.list.replaceChildren();
    const query = ui.search.value.trim().toLocaleLowerCase("fr");
    const items = state.conversations
      .filter((item) => !favoriteOnly || item.favorite)
      .filter((item) => !query || item.title.toLocaleLowerCase("fr").includes(query) || item.messages.some((message) => message.body.toLocaleLowerCase("fr").includes(query)))
      .sort((a, b) => timestamp(b.updated_at) - timestamp(a.updated_at));
    if (!items.length) {
      const text = query ? "Aucune conversation trouvée." : favoriteOnly ? "Ajoutez une conversation aux favoris pour la retrouver ici." : "Vos conversations apparaîtront ici.";
      ui.list.append(element("p", "sidebar-empty", text));
      return;
    }
    let previousGroup;
    const today = new Date().toDateString();
    const yesterday = new Date(Date.now() - 86400000).toDateString();
    for (const conversation of items) {
      const date = timestamp(conversation.updated_at).toDateString();
      const group = date === today ? "Aujourd’hui" : date === yesterday ? "Hier" : "Précédemment";
      if (group !== previousGroup) { ui.list.append(element("p", "list-group-title", group)); previousGroup = group; }
      const button = element("button", `conversation-item${conversation.id === selectedId ? " active" : ""}`);
      button.type = "button";
      button.dataset.conversationId = conversation.id;
      button.title = conversation.title;
      if (conversation.id === selectedId) button.setAttribute("aria-current", "page");
      button.append(icon("chat"), element("span", "conversation-name", conversation.title || "Nouvelle conversation"));
      if (conversation.messages.some((message) => message.state === "unknown")) button.append(element("span", "pending-indicator"));
      if (conversation.favorite) button.append(icon("star", "icon favorite-indicator"));
      button.addEventListener("click", () => { if (!busy) selectConversation(conversation.id); });
      ui.list.append(button);
    }
  }

  function renderMessage(message) {
    const isUser = message.role === "user";
    const article = element("article", `message ${isUser ? "message-user" : "message-assistant"}`);
    article.dataset.messageId = message.id;
    const avatar = element("div", "message-avatar", isUser ? "V" : undefined);
    avatar.setAttribute("aria-hidden", "true");
    if (!isUser) avatar.append(mascot ? mascot.create() : icon("mark"));
    const content = element("div", "message-content");
    const heading = element("div", "message-heading");
    const date = timestamp(message.created_at);
    const time = element("time", "", date.toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" }));
    time.dateTime = date.toISOString();
    heading.append(element("strong", "", isUser ? "Vous" : "Grokbot"), time);
    content.append(heading, element("div", "message-body", message.body));
    const actions = element("div", "message-actions");
    const copy = element("button", "message-copy");
    copy.type = "button";
    copy.setAttribute("aria-label", isUser ? "Copier votre message" : "Copier la réponse");
    copy.append(icon("copy"), element("span", "", "Copier"));
    copy.addEventListener("click", async () => {
      try {
        await copyText(message.body);
        copy.replaceChildren(icon("check"), element("span", "", "Copié"));
        setTimeout(() => { if (copy.isConnected) copy.replaceChildren(icon("copy"), element("span", "", "Copier")); }, 1800);
      } catch { notify("La copie n’est pas disponible. Sélectionnez le texte pour le copier."); }
    });
    actions.append(copy);
    if (isUser && ["accepted", "echoed"].includes(message.state)) {
      const status = element("span", "message-status");
      status.append(icon("check"), document.createTextNode("Envoyé"));
      actions.append(status);
    }
    if (isUser && ["queued", "dispatching"].includes(message.state)) actions.append(element("span", "message-status", "Envoi en cours…"));
    if (message.state === "refused") actions.append(element("span", "message-status", "Envoi refusé"));
    content.append(actions);
    if (isUser && pendingStates.has(message.state) && (!busy || message.state === "unknown")) {
      const card = element("div", "pending-card");
      const copy = element("div", "pending-copy");
      const queued = message.state === "queued";
      copy.append(element("strong", "", queued ? "Ce message attend son envoi." : "Cet envoi attend confirmation."), element("p", "", queued ? "Votre message est sauvegardé. Reprenez l’envoi pour poursuivre." : "Votre message est sauvegardé. Vérifiez l’envoi pour poursuivre."));
      const retry = element("button", "retry-button");
      retry.type = "button";
      retry.disabled = busy;
      retry.append(icon("refresh"), document.createTextNode(queued ? "Reprendre l’envoi" : "Vérifier l’envoi"));
      retry.addEventListener("click", () => queued ? resumeQueued(message) : reconcile(message.id));
      card.append(copy, retry);
      content.append(card);
    }
    article.append(avatar, content);
    return article;
  }

  function render() {
    const conversation = currentConversation();
    const hasMessages = Boolean(conversation?.messages.length);
    const name = state.account.name || "Espace personnel";
    $("account-name").textContent = name;
    $("account-avatar").textContent = name.charAt(0).toLocaleUpperCase("fr");
    ui.title.textContent = conversation?.title || "Nouvelle conversation";
    document.title = conversation ? `${conversation.title} · Grokbot` : "Grokbot";
    ui.welcome.hidden = hasMessages;
    ui.messages.hidden = !hasMessages;
    ui.messages.replaceChildren(...(conversation?.messages || []).map(renderMessage));
    $("favorite-button").querySelector("span").textContent = conversation?.favorite ? "Retirer des favoris" : "Ajouter aux favoris";
    renderList();
    updateComposer();
  }

  function scrollToLatest() {
    requestAnimationFrame(() => { ui.scroll.scrollTop = ui.scroll.scrollHeight; });
  }

  async function newConversation() {
    if (busy) return;
    busy = true;
    updateComposer();
    try {
      const { conversation } = await api("/api/conversations", {});
      upsertConversation(conversation);
      selectConversation(conversation.id, true);
    } catch (error) { notify(error.message); }
    finally { busy = false; updateComposer(); }
  }

  async function sendMessage(event) {
    event.preventDefault();
    if (ui.send.disabled) return;
    const body = ui.input.value.trim();
    let conversation = currentConversation();
    busy = true;
    submissionActivity = "send";
    updateComposer();
    let messageId;
    let delivered = false;
    let previousDraftKey = draftKey();
    try {
      if (!conversation) {
        const created = await api("/api/conversations", {});
        upsertConversation(created.conversation);
        conversation = created.conversation;
        selectedId = conversation.id;
        writePreference("grokbot.selected", selectedId);
      }
      messageId = crypto.randomUUID();
      conversation.messages.push({ id: messageId, role: "user", body, state: "queued", created_at: new Date().toISOString() });
      ui.input.value = "";
      removePreference(previousDraftKey);
      removePreference(draftKey());
      render();
      scrollToLatest();
      const response = await api("/api/send", {
        conversation_id: conversation.id, message_id: messageId, body, simulate_loss: simulateLoss,
      });
      simulateLoss = false;
      $("simulate-loss").checked = false;
      upsertConversation(response.conversation);
      delivered = response.conversation.messages.some((message) => message.id === messageId && ["accepted", "echoed"].includes(message.state));
      render();
      scrollToLatest();
    } catch (error) {
      if (messageId) {
        try {
          await refresh();
          const savedMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          if (!savedMessage) { ui.input.value = body; saveDraft(); }
          notify(error.message);
        } catch {
          const optimisticMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          if (optimisticMessage) optimisticMessage.state = "unknown";
          render();
          showConnectionError(error);
        }
      } else { notify(error.message); }
    } finally {
      busy = false;
      submissionActivity = null;
      render();
      if (delivered) mascot?.celebrate();
      ui.input.focus();
    }
  }

  async function reconcile(messageId) {
    const conversation = currentConversation();
    if (busy || !conversation) return;
    busy = true;
    submissionActivity = "verify";
    let confirmed = false;
    render();
    try {
      const response = await api("/api/reconcile", { conversation_id: conversation.id, message_id: messageId });
      upsertConversation(response.conversation);
      render();
      scrollToLatest();
      const pending = response.conversation.messages.some((message) => message.id === messageId && pendingStates.has(message.state));
      confirmed = response.conversation.messages.some((message) => message.id === messageId && ["accepted", "echoed"].includes(message.state));
      notify(pending ? "Cet envoi attend encore confirmation." : "L’envoi est vérifié. Vous pouvez continuer.");
    } catch (error) { notify(error.message); }
    finally { busy = false; submissionActivity = null; render(); if (confirmed) mascot?.celebrate(); }
  }

  async function resumeQueued(message) {
    const conversation = currentConversation();
    if (busy || !conversation) return;
    busy = true;
    submissionActivity = "send";
    let delivered = false;
    render();
    try {
      const response = await api("/api/send", {
        conversation_id: conversation.id, message_id: message.id, body: message.body,
      });
      upsertConversation(response.conversation);
      delivered = response.conversation.messages.some((item) => item.id === message.id && ["accepted", "echoed"].includes(item.state));
      render();
      scrollToLatest();
    } catch (error) { notify(error.message); }
    finally { busy = false; submissionActivity = null; render(); if (delivered) mascot?.celebrate(); }
  }

  function closeMenu() {
    ui.menu.hidden = true;
    ui.menuButton.setAttribute("aria-expanded", "false");
  }

  function closeSidebar() {
    document.body.classList.remove("sidebar-open");
    $("sidebar-toggle").setAttribute("aria-expanded", "false");
  }

  function applyTheme(choice) {
    themeChoice = ["light", "dark", "system"].includes(choice) ? choice : "light";
    document.documentElement.dataset.theme = themeChoice === "system" ? (systemTheme.matches ? "dark" : "light") : themeChoice;
    writePreference("grokbot.theme", themeChoice);
    document.querySelectorAll("[data-theme-choice]").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.themeChoice === themeChoice)));
  }

  async function updateConversation(changes) {
    const conversation = currentConversation();
    if (!conversation || busy) return;
    busy = true;
    updateComposer();
    try {
      const response = await api("/api/conversations/update", { id: conversation.id, ...changes });
      upsertConversation(response.conversation);
      render();
      return true;
    } catch (error) { notify(error.message); return false; }
    finally { busy = false; updateComposer(); }
  }

  function exportConversation() {
    const conversation = currentConversation();
    if (!conversation) return;
    const blob = new Blob([JSON.stringify(conversation, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = element("a");
    anchor.href = url;
    anchor.download = `grokbot-${conversation.id}.json`;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    closeMenu();
  }

  $("new-chat").addEventListener("click", newConversation);
  $("home-link").addEventListener("click", (event) => { event.preventDefault(); if (!busy) selectConversation(null, true); });
  ui.search.addEventListener("input", renderList);
  $("favorites-filter").addEventListener("click", () => {
    favoriteOnly = !favoriteOnly;
    $("favorites-filter").setAttribute("aria-pressed", String(favoriteOnly));
    $("favorites-filter").setAttribute("aria-label", favoriteOnly ? "Afficher toutes les conversations" : "Afficher les favoris");
    renderList();
  });
  ui.input.addEventListener("input", () => { saveDraft(); updateComposer(); });
  ui.input.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      if (!ui.send.disabled) $("composer-form").requestSubmit();
    }
  });
  $("composer-form").addEventListener("submit", sendMessage);
  document.querySelectorAll(".suggestion").forEach((button) => button.addEventListener("click", () => {
    ui.input.value = button.dataset.prompt.replace(/\\n/g, "\n");
    saveDraft();
    updateComposer();
    ui.input.focus();
    ui.input.setSelectionRange(ui.input.value.length, ui.input.value.length);
  }));
  ui.menuButton.addEventListener("click", () => {
    ui.menu.hidden = !ui.menu.hidden;
    ui.menuButton.setAttribute("aria-expanded", String(!ui.menu.hidden));
  });
  document.addEventListener("click", (event) => { if (!ui.menu.contains(event.target) && !ui.menuButton.contains(event.target)) closeMenu(); });
  $("sidebar-toggle").addEventListener("click", () => {
    const open = document.body.classList.toggle("sidebar-open");
    $("sidebar-toggle").setAttribute("aria-expanded", String(open));
    if (open) ui.search.focus();
  });
  $("sidebar-close").addEventListener("click", closeSidebar);
  $("sidebar-scrim").addEventListener("click", closeSidebar);
  $("settings-button").addEventListener("click", () => { closeSidebar(); $("settings-dialog").showModal(); });
  document.querySelectorAll("[data-close-dialog]").forEach((button) => button.addEventListener("click", () => button.closest("dialog").close()));
  document.querySelectorAll("dialog").forEach((dialog) => dialog.addEventListener("click", (event) => {
    if (event.target !== dialog) return;
    const rect = dialog.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) dialog.close();
  }));
  document.querySelectorAll("[data-theme-choice]").forEach((button) => button.addEventListener("click", async () => {
    const theme = button.dataset.themeChoice;
    const buttons = document.querySelectorAll("[data-theme-choice]");
    buttons.forEach((item) => { item.disabled = true; });
    try {
      const response = await api("/api/preferences", { theme });
      applyTheme(response.preferences?.theme || response.theme || theme);
    } catch (error) { notify(error.message); }
    finally { buttons.forEach((item) => { item.disabled = false; }); }
  }));
  systemTheme.addEventListener("change", () => { if (themeChoice === "system") applyTheme("system"); });
  $("simulate-loss").addEventListener("change", (event) => { simulateLoss = event.target.checked; });
  $("rename-button").addEventListener("click", () => {
    closeMenu();
    $("rename-input").value = currentConversation()?.title || "";
    $("rename-dialog").showModal();
    $("rename-input").select();
  });
  $("rename-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const title = $("rename-input").value.trim();
    if (!title) { $("rename-input").focus(); return; }
    if (await updateConversation({ title })) $("rename-dialog").close();
  });
  $("favorite-button").addEventListener("click", async () => {
    closeMenu();
    const conversation = currentConversation();
    if (conversation) await updateConversation({ favorite: !conversation.favorite });
  });
  $("export-button").addEventListener("click", exportConversation);
  $("delete-button").addEventListener("click", () => { closeMenu(); $("delete-dialog").showModal(); });
  $("confirm-delete").addEventListener("click", async () => {
    const conversation = currentConversation();
    if (!conversation || busy) return;
    busy = true;
    $("confirm-delete").disabled = true;
    updateComposer();
    try {
      await api("/api/conversations/delete", { id: conversation.id });
      state.conversations = state.conversations.filter((item) => item.id !== conversation.id);
      removePreference(draftKey(conversation.id));
      ui.input.value = "";
      selectedId = null;
      writePreference("grokbot.selected", "");
      ui.input.value = readPreference(draftKey(), "");
      $("delete-dialog").close();
      render();
      notify("Conversation supprimée.");
    } catch (error) { notify(error.message); }
    finally { busy = false; $("confirm-delete").disabled = false; updateComposer(); }
  });
  $("reconnect-button").addEventListener("click", async () => {
    $("reconnect-button").disabled = true;
    try { await refresh(); scrollToLatest(); }
    catch (error) { showConnectionError(error); }
    finally { $("reconnect-button").disabled = false; }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") { closeMenu(); closeSidebar(); }
    if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
    const dialogOpen = Boolean(document.querySelector("dialog[open]"));
    if (event.key.toLowerCase() === "n" && !dialogOpen) { event.preventDefault(); newConversation(); }
    if (event.key.toLowerCase() === "k" && !dialogOpen) {
      event.preventDefault();
      if (window.matchMedia("(max-width: 760px)").matches) {
        document.body.classList.add("sidebar-open");
        $("sidebar-toggle").setAttribute("aria-expanded", "true");
      }
      ui.search.focus();
      ui.search.select();
    }
  });

  applyTheme(themeChoice);
  ui.input.value = readPreference(draftKey(), "");
  render();
  refresh().then(() => {
    ui.input.value = readPreference(draftKey(), "");
    updateComposer();
    scrollToLatest();
  }).catch(showConnectionError);
})();
