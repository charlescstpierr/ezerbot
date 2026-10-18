"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const ui = {
    list: $("conversation-list"), search: $("search"), input: null,
    send: $("send-button"), welcome: $("welcome"), messages: $("messages"),
    scroll: $("chat-scroll"), title: $("conversation-title"), menu: $("conversation-menu"),
    menuButton: $("conversation-menu-button"), hint: $("composer-hint"),
    banner: $("connection-banner"), bannerText: $("connection-message"), toast: $("toast"),
  };
  let state = { account: { name: "Espace personnel" }, mode: "simulated", conversations: [], bots: [] };
  let selectedId = readPreference("grokbot.selected", null);
  let chosenBotId = readPreference("grokbot.bot", "grokbot");
  // Le bot ouvert dans le dialogue; `null` pour une création, dont l'identifiant reste stable entre deux essais.
  let editingBotId = null;
  let draftBotId = null;
  const defaultWelcome = $("welcome-description").textContent;
  let favoriteOnly = false;
  let connected = false;
  let connectionStatus = "loading";
  let busy = false;
  let submissionActivity = null;
  const mascot = window.GrokbotMascot;
  let toastTimer;
  let simulateLoss = false;
  let themeChoice = readPreference("grokbot.theme", "dark");
  let accentChoice = readPreference("grokbot.accent", "citron");
  const accents = ["citron", "orange", "ambre", "menthe", "cyan", "magenta", "vermillon", "violet", "blanc"];
  let preferencesLoaded = false;
  let pollTimer;
  let pickerKind = null;
  const usageUrl = "https://chatgpt.com/settings/usage";
  const effortNames = { none: "Aucun", minimal: "Minimal", low: "Léger", medium: "Équilibré", high: "Élevé", xhigh: "Très élevé", max: "Maximum", ultra: "Ultra" };
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  const pendingStates = new Set(["queued", "dispatching", "unknown"]);

  const panels = new Map();
  const panelFailed = Symbol("panel-failed");
  const botCleanups = [];
  // Keep the original extras until a queued submission settles, even if state refresh replaces its DTO.
  const pendingSends = new Map();
  // Section hooks mount once: settingsSection(container, context), botSection(bot, container, context).
  // botSection may return a cleanup function, called before remount and on dialog close.
  // onState(state, context), onCardAction(message, action, detail, context); only true claims an action.
  // wantsPolling(context) and sendExtras(context) are synchronous; sendExtras returns extra /api/send fields.
  // composer: mount(container, context), onFiles(files, event, context), blocked(context), onSent({messageId, extras}, context).
  // onSent runs once the server holds the message, including unknown/queued states, not on confirmation.
  // onFiles returns false to preserve normal paste/drop. Async uploads call composerChanged() when ready.
  const panelContext = () => ({ state, conversation: currentConversation(), bot: currentBot(), draft: ui.input?.value || "" });
  function panelError(id, error) { console.warn(`Ezerbot panel ${id}:`, error); }
  function panelCall(panel, hook, ...args) {
    try {
      const result = hook?.(...args, panelContext());
      if (result?.then) result.catch((error) => panelError(panel.id, error));
      return result;
    } catch (error) { panelError(panel.id, error); return panelFailed; }
  }
  function mountSection(panel, hook, target, ...args) {
    if (!hook) return;
    const container = element("div");
    container.dataset.panel = panel.id;
    target.append(container);
    const content = panelCall(panel, hook, ...args, container);
    if (content instanceof Node) container.append(content);
    if (typeof content === "function") return () => panelCall(panel, content);
  }
  function clearBotExtensions() {
    for (const cleanup of botCleanups.splice(0)) cleanup();
    $("bot-extensions").replaceChildren();
  }
  function mountBotSection(panel, bot) {
    const cleanup = mountSection(panel, panel.botSection, $("bot-extensions"), bot);
    if (cleanup) botCleanups.push(cleanup);
  }
  function renderBotExtensions(bot) {
    clearBotExtensions();
    for (const panel of panels.values()) mountBotSection(panel, bot);
  }
  function observeSavedConversation(conversation) {
    for (const message of conversation.messages) {
      const pending = pendingSends.get(message.id);
      if (!pending) continue;
      if (!pending.consumed) {
        pending.consumed = true;
        for (const panel of panels.values()) panelCall(panel, panel.composer?.onSent, { messageId: message.id, extras: pending.extras });
      }
      if (message.state !== "queued") pendingSends.delete(message.id);
    }
  }
  function sendExtras() {
    const extras = { mentions: ui.input?.mentions || [] };
    for (const panel of panels.values()) {
      const fields = panelCall(panel, panel.sendExtras);
      if (fields && typeof fields === "object" && !fields.then && !Array.isArray(fields)) Object.assign(extras, fields);
    }
    return extras;
  }
  function composerFiles(files, event) {
    if (!files?.length) return false;
    let handled = false;
    for (const panel of panels.values()) {
      if (!panel.composer?.onFiles) continue;
      const result = panelCall(panel, panel.composer.onFiles, Array.from(files), event);
      if (result !== false && result !== panelFailed) handled = true;
    }
    if (handled) { event?.preventDefault(); updateComposer(); schedulePoll(); }
    return handled;
  }
  async function cardAction(message, action, detail) {
    for (const panel of panels.values()) {
      if (!panel.onCardAction) continue;
      try {
        if (await panel.onCardAction(message, action, detail, panelContext()) === true) return true;
      } catch (error) { panelError(panel.id, error); notify(error.message); return false; }
    }
    notify("Cette action n’est pas disponible.");
    return false;
  }
  window.GrokbotPanels = {
    register(panel) {
      if (!panel?.id || panels.has(panel.id)) {
        panelError(panel?.id, new Error("Duplicate or missing panel id"));
        return false;
      }
      panels.set(panel.id, panel);
      mountSection(panel, panel.settingsSection, $("settings-extensions"));
      if ($("bot-dialog").open) mountBotSection(panel, botById(editingBotId) || null);
      panelCall(panel, panel.composer?.mount, $("composer-attachments"));
      panelCall(panel, panel.onState, state);
      updateComposer();
      schedulePoll();
      return true;
    },
    openConversation(id) {
      if (busy || !state.conversations.some((conversation) => conversation.id === id)) return false;
      selectConversation(id, true);
      return true;
    },
    draft: () => ui.input?.value || "",
    get state() { return state; },
    api, refresh, notify, createBotMascot, botSummary,
    composerChanged() { updateComposer(); schedulePoll(); },
  };

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
  function botById(id) { return state.bots.find((bot) => bot.id === id); }
  // Les bots supprimés restent dans l'état pour que leurs conversations gardent nom et couleur.
  function liveBots() { return state.bots.filter((bot) => !bot.deleted); }
  // Un choix devenu caduc (bot supprimé ailleurs) retombe sur le bot par défaut.
  function chosenBot() { const bot = botById(chosenBotId); return bot && !bot.deleted ? bot : liveBots()[0]; }
  function currentBot() { const conversation = currentConversation(); return conversation ? botById(conversation.bot_id) : chosenBot(); }
  function botName(id) { const bot = botById(id); return bot ? `${bot.name}${bot.deleted ? " (supprimé)" : ""}` : "Bot inconnu"; }
  function provider() { return state.provider || { connected: false, signin: "idle", models: [] }; }
  function streamingMessage(conversation = currentConversation()) { return conversation?.messages.find((message) => window.GrokbotChat.isPendingReply(message, conversation.kind === "group")); }
  function effortName(effort) { return effortNames[effort] || (effort ? effort.charAt(0).toUpperCase() + effort.slice(1) : "Par défaut"); }

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
    const previous = currentConversation();
    const groupWasActive = previous?.kind === "group" && Boolean(streamingMessage(previous));
    const previousTail = previous?.messages.at(-1)?.id;
    state = await api("/api/state");
    for (const conversation of state.conversations) observeSavedConversation(conversation);
    if (!preferencesLoaded) {
      preferencesLoaded = true;
      applyTheme(state.preferences?.theme || themeChoice);
      applyAccent(state.preferences?.accent || accentChoice);
    }
    connected = true;
    connectionStatus = "online";
    ui.banner.hidden = true;
    if (selectedId && !state.conversations.some((item) => item.id === selectedId)) selectedId = null;
    writePreference("grokbot.selected", selectedId || "");
    const pinned = ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90;
    render();
    const current = currentConversation();
    if (pinned && (streamingMessage() || (current?.kind === "group" && current.id === previous?.id && (groupWasActive || current.messages.at(-1)?.id !== previousTail)))) scrollToLatest();
    schedulePoll();
  }

  // The engine writes a reply while it streams; the shell re-reads it until every reply settles.
  function schedulePoll() {
    clearTimeout(pollTimer);
    const live = state.conversations.some((conversation) => streamingMessage(conversation));
    const waiting = provider().signin === "pending" || provider().titling || (provider().connected && !provider().models_ready);
    const panelWaiting = [...panels.values()].some((panel) => panelCall(panel, panel.wantsPolling) === true);
    if (!live && !waiting && !panelWaiting) return;
    pollTimer = setTimeout(async () => {
      if (busy) { schedulePoll(); return; }
      try { await refresh(); } catch { /* showConnectionError already told the user. */ }
    }, live ? 200 : 800);
  }

  function showConnectionError(error) {
    connected = false;
    connectionStatus = "offline";
    ui.bannerText.textContent = "Le moteur local ne répond plus. Vos conversations enregistrées sont conservées.";
    ui.banner.hidden = false;
    updateComposer();
    if (error) console.warn("Ezerbot local connection:", error.message);
  }

  function upsertConversation(conversation) {
    const index = state.conversations.findIndex((item) => item.id === conversation.id);
    if (index === -1) state.conversations.unshift(conversation);
    else state.conversations[index] = conversation;
    observeSavedConversation(conversation);
    connected = true;
    connectionStatus = "online";
    ui.banner.hidden = true;
  }

  function setComposerText(text) {
    if (text) ui.input.value = text;
    // The textarea adapter turns "" into invalid Tiptap content ([]).
    // Use the mounted editor's clear command, without emitting a draft update.
    else $("composer-input").querySelector(".gb-editor").editor.commands.clearContent(false);
  }

  function saveDraft() {
    if (ui.input.value) {
      writePreference(draftKey(), ui.input.value);
      writePreference(`${draftKey()}.document`, JSON.stringify(ui.input.document));
    } else {
      removePreference(draftKey());
      removePreference(`${draftKey()}.document`);
    }
  }
  function restoreDraftDocument() {
    try {
      const document = JSON.parse(readPreference(`${draftKey()}.document`, "null"));
      if (document && readPreference(draftKey(), "") === ui.input.value) ui.input.document = document;
    } catch { /* Plain text remains usable when an older draft has no rich document. */ }
  }

  function updateComposer() {
    const pendingMessage = currentConversation()?.messages.find((message) => message.role === "user" && pendingStates.has(message.state));
    const pending = Boolean(pendingMessage);
    const group = currentConversation()?.kind === "group";
    const task = currentConversation()?.kind === "task";
    const taskFinished = task && !window.GrokbotDelegation?.canFollowUp(currentConversation().id);
    ui.input.setReadOnly(taskFinished);
    ui.input.setPlaceholder(task ? "Écrire un suivi pour cette sous-tâche…" : "Qu’avez-vous en tête ?");
    if (taskFinished) {
      if (ui.input.value) setComposerText("");
      removePreference(draftKey());
      removePreference(`${draftKey()}.document`);
      for (const panel of panels.values()) panelCall(panel, panel.composer?.discard);
      if (document.activeElement?.closest?.(".composer-area")) document.querySelector(".gb-delegation-bar p")?.focus();
    }
    const tooLong = new TextEncoder().encode(ui.input.value.trim()).length > 16 * 1024;
    const hasAttachments = Boolean(sendExtras().attachments?.length);
    const attachmentsBlocked = [...panels.values()].some((panel) => Boolean(panelCall(panel, panel.composer?.blocked)));
    // The task banner is the single cancellation control. The composer only sends follow-ups.
    const live = !task && Boolean(streamingMessage()) && (!group || (!ui.input.value.trim() && !hasAttachments));
    const noMembers = group && !currentConversation().member_ids?.length;
    ui.send.disabled = taskFinished || (live ? ((group || task) && (busy || !connected)) : busy || !connected || noMembers || (!ui.input.value.trim() && !hasAttachments) || pending || tooLong || attachmentsBlocked);
    ui.send.dataset.mode = live ? "stop" : "send";
    ui.send.title = live ? group ? "Arrêter les tours" : "Arrêter la réponse" : "Envoyer le message";
    ui.send.setAttribute("aria-label", live ? ui.send.title : busy ? "Envoi en cours" : "Envoyer le message");
    ui.send.querySelector("use").setAttribute("href", live ? "#i-stop" : "#i-send");
    ui.input.setInvalid(tooLong);
    ui.hint.dataset.alert = String(Boolean(pending || tooLong));
    ui.hint.textContent = taskFinished ? "Cette tâche est terminée ; poursuivez dans la conversation principale." : noMembers ? "Ajoutez un bot au groupe pour poursuivre." : pending ? "Vérifiez le dernier envoi pour poursuivre la conversation." : tooLong ? "Ce message est trop long. Raccourcissez-le pour l’envoyer." : task ? "Suivi réservé à cette sous-tâche · Entrée pour envoyer" : group && live ? "Écrivez pour envoyer un nouveau message, ou arrêtez les tours." : "Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne";
    $("new-chat").disabled = busy;
    ui.menuButton.disabled = busy || !currentConversation();
    mascot?.setContext({ connection: connected ? "online" : connectionStatus,
      activity: taskFinished ? null : submissionActivity, pending: taskFinished ? null : pendingMessage?.state || null, drafting: !taskFinished && Boolean(ui.input.value.trim()) });
  }

  function selectConversation(id, focus = false) {
    saveDraft();
    selectedId = id;
    writePreference("grokbot.selected", id || "");
    setComposerText(readPreference(draftKey(), ""));
    restoreDraftDocument();
    closeMenu();
    closeSidebar();
    render(true);
    scrollToLatest();
    if (focus) ui.input.focus();
  }

  // Images staged before the first message belong to the current bot/thread.
  function stagedOnEmptyThread() {
    return Boolean(!currentConversation()?.messages.length && $("composer-attachments").querySelector(".attachment-chip"));
  }

  function leaveEmptyThread() {
    if (!currentConversation() || currentConversation().messages.length || stagedOnEmptyThread()) return;
    const draft = ui.input.value;
    // Move the text, not a copy: selectConversation will remove the old draft key.
    setComposerText("");
    selectConversation(null);
    const welcome = ui.input.value;
    if (draft) {
      setComposerText(welcome ? `${welcome}\n${draft}` : draft);
      saveDraft();
    }
  }

  function renderList() {
    ui.list.replaceChildren();
    const query = ui.search.value.trim().toLocaleLowerCase("fr");
    const items = state.conversations
      .filter((item) => item.kind !== "task" && item.kind !== "routine")
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
      const bot = botById(conversation.bot_id);
      const date = timestamp(conversation.updated_at).toDateString();
      const group = date === today ? "Aujourd’hui" : date === yesterday ? "Hier" : "Précédemment";
      if (group !== previousGroup) { ui.list.append(element("p", "list-group-title", group)); previousGroup = group; }
      const button = element("button", `conversation-item${conversation.id === selectedId ? " active" : ""}`);
      button.type = "button";
      button.dataset.conversationId = conversation.id;
      button.title = `${conversation.title} · ${conversation.kind === "group" ? `Groupe : ${groupMembers(conversation).map((b) => b.name).join(", ")}` : botName(conversation.bot_id)}`;
      if (conversation.id === selectedId) button.setAttribute("aria-current", "page");
      button.append(conversation.kind === "group" ? groupAvatars(conversation) : botAvatar(bot), element("span", "conversation-name", conversation.title || "Nouvelle conversation"));
      if (conversation.messages.some((message) => message.state === "unknown")) button.append(element("span", "pending-indicator"));
      if (conversation.favorite) button.append(icon("star", "icon favorite-indicator"));
      button.addEventListener("click", () => { if (!busy) selectConversation(conversation.id); });
      ui.list.append(button);
    }
  }

  // Une mascotte immobile ; un bot inconnu garde un galet neutre.
  function botAvatar(bot) {
    const avatar = element("span", "bot-avatar");
    avatar.setAttribute("aria-hidden", "true");
    if (bot) avatar.dataset.botColor = bot.color;
    const figure = mascot?.create(false, bot?.shape);
    if (figure) avatar.append(figure);
    return avatar;
  }

  function botSummary(bot) {
    if (bot.description) return bot.description;
    const date = new Date(bot.created_at);
    return Number.isNaN(date.getTime()) ? "Bot personnel" : `Créé le ${date.toLocaleString("fr-CA", { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" })}`;
  }
  function groupMembers(conversation) { return (conversation.member_ids || []).map(botById).filter(Boolean); }
  function groupAvatars(conversation) {
    const stack = element("span", "group-avatar-stack");
    stack.setAttribute("aria-hidden", "true");
    const members = groupMembers(conversation);
    for (const bot of members.slice(0, 3)) stack.append(botAvatar(bot));
    if (members.length > 3) stack.append(element("span", "group-avatar-more", `+${members.length - 3}`));
    if (!members.length) stack.append(botAvatar(null));
    return stack;
  }

  function renderBots() {
    const chosen = currentBot();
    const chips = liveBots().flatMap((bot) => {
      const chip = element("button", "bot-chip");
      chip.type = "button";
      chip.dataset.botColor = bot.color;
      chip.setAttribute("aria-pressed", String(bot.id === chosen?.id));
      if (bot.description) chip.title = bot.description;
      chip.append(botAvatar(bot), element("span", "", bot.name));
      chip.addEventListener("click", () => {
        if (busy) return;
        if (stagedOnEmptyThread()) {
          notify(`Retirez l’image jointe pour changer de bot, ou envoyez-la à ${currentBot()?.name || "ce bot"}.`);
          return;
        }
        chosenBotId = bot.id;
        writePreference("grokbot.bot", bot.id);
        // Preserve both drafts when leaving a persisted empty thread from an old version.
        leaveEmptyThread();
        render();
      });
      if (bot.id !== chosen?.id) return [chip];
      // Le crayon est un frère de la puce : un bouton ne peut pas en contenir un autre.
      const edit = element("button", "bot-chip-edit");
      edit.type = "button";
      edit.setAttribute("aria-label", `Modifier ${bot.name}`);
      edit.title = "Modifier la personnalité";
      edit.append(icon("edit"));
      edit.addEventListener("click", () => openBotDialog(bot));
      return [chip, edit];
    });
    const add = element("button", "bot-chip add-bot");
    add.type = "button";
    add.append(icon("plus"), element("span", "", "Nouveau bot"));
    add.addEventListener("click", () => openBotDialog(null));
    $("bot-chips").replaceChildren(...chips, add);
    $("welcome-description").textContent = currentConversation()?.kind === "group"
      ? "Un fil partagé pour croiser vos idées. Sans mention, le coordinateur répond."
      : chosen?.description || defaultWelcome;

    $("bot-list").replaceChildren(...liveBots().map((bot) => {
      const row = element("div", "bot-row");
      const copy = element("div");
      copy.append(element("strong", "", bot.name), element("small", "", bot.description || (bot.is_default ? "Le bot par défaut." : "Aucune description.")));
      const edit = element("button", "secondary-button", "Modifier");
      edit.type = "button";
      edit.setAttribute("aria-label", `Modifier ${bot.name}`);
      edit.addEventListener("click", () => openBotDialog(bot));
      row.append(botAvatar(bot), copy, edit);
      return row;
    }));
  }

  function openBotDialog(bot) {
    closeMenu();
    editingBotId = bot?.id || null;
    if (!bot && !draftBotId) draftBotId = crypto.randomUUID();
    const values = bot || { name: "", color: chosenBot()?.color || "citron", shape: "galet", description: "", instructions: "" };
    $("bot-dialog-title").textContent = bot ? `Modifier ${bot.name}` : "Nouveau bot";
    $("bot-save").textContent = bot ? "Enregistrer" : "Créer le bot";
    $("bot-name").value = values.name;
    $("bot-description").value = values.description;
    $("bot-instructions").value = values.instructions;
    selectBotColor(values.color);
    selectBotShape(values.shape);
    $("bot-delete-button").hidden = !bot || bot.is_default;
    renderBotExtensions(bot);
    $("bot-dialog").showModal();
    $("bot-name").focus();
  }

  function selectBotColor(color) {
    $("bot-colors").querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.botColor === color)));
    $("bot-shapes").dataset.botColor = color;
  }

  function selectBotShape(shape) {
    $("bot-shapes").querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.shape === shape)));
  }

  function upsertBot(bot) {
    const index = state.bots.findIndex((item) => item.id === bot.id);
    if (index === -1) state.bots.push(bot);
    else state.bots[index] = bot;
  }

  async function saveBot(event) {
    event.preventDefault();
    const name = $("bot-name").value.trim();
    if (!name) { notify("Donnez un nom à votre bot."); $("bot-name").focus(); return; }
    const values = {
      name,
      color: $("bot-colors").querySelector("[aria-pressed=true]")?.dataset.botColor || "citron",
      shape: $("bot-shapes").querySelector("[aria-pressed=true]")?.dataset.shape || "galet",
      description: $("bot-description").value.trim(),
      instructions: $("bot-instructions").value.trim(),
    };
    const existing = botById(editingBotId);
    // Seuls les champs modifiés partent : enregistrer sans rien changer ne personnalise pas le bot par défaut.
    const changes = existing ? Object.fromEntries(Object.entries(values).filter(([key, value]) => existing[key] !== value)) : values;
    if (existing && !Object.keys(changes).length) { $("bot-dialog").close(); return; }
    $("bot-save").disabled = true;
    try {
      const { bot } = existing
        ? await api("/api/bots/update", { id: existing.id, ...changes })
        : await api("/api/bots", { id: draftBotId, ...changes });
      upsertBot(bot);
      const keepStagedBot = !existing && stagedOnEmptyThread();
      if (!existing) {
        draftBotId = null;
        if (!keepStagedBot) {
          chosenBotId = bot.id;
          writePreference("grokbot.bot", bot.id);
          leaveEmptyThread();
        }
      }
      $("bot-dialog").close();
      render();
      notify(existing ? "Bot enregistré." : keepStagedBot
        ? `${bot.name} est prêt. Retirez l’image jointe pour le choisir, ou envoyez-la à ${currentBot()?.name || "ce bot"}.`
        : `${bot.name} est prêt.`);
    } catch (error) { notify(error.message); }
    finally { $("bot-save").disabled = false; }
  }

  async function deleteBot() {
    const bot = botById(editingBotId);
    if (!bot) return;
    $("confirm-bot-delete").disabled = true;
    try {
      await api("/api/bots/delete", { id: bot.id });
      upsertBot({ ...bot, deleted: true });
      $("bot-delete-dialog").close();
      $("bot-dialog").close();
      await refresh(); // Memberships and coordinators changed with the tombstone.
      notify(`${bot.name} a été supprimé.`);
    } catch (error) { notify(error.message); }
    finally { $("confirm-bot-delete").disabled = false; }
  }

  // Une fabrique stable par forme : le fil ne recrée ses mascottes que si la forme change.
  const threadMascots = {};
  function threadMascot(shape) { return threadMascots[shape] ||= () => mascot?.create(false, shape); }
  const authorMascots = {};
  function createBotMascot(bot) {
    const key = `${bot.id}:${bot.shape}:${bot.color}`;
    return authorMascots[key] ||= () => {
      const wrapper = element("span", "gb-author-mascot");
      wrapper.dataset.botColor = bot.color;
      const figure = mascot?.create(false, bot.shape);
      if (figure) wrapper.append(figure);
      return wrapper;
    };
  }

  const thread = {
    createMascot: threadMascot("galet"),
    onRetry: (message) => message.state === "queued" ? resumeQueued(message) : reconcile(message.id),
    onRegenerate: (message) => generation("/api/regenerate", message.id.replace(/^reply-/, "")),
    onSignIn: () => signIn(),
    onCardAction: cardAction,
    usageUrl,
    async onCopy(text) {
      try { await copyText(text); return true; }
      catch { notify("La copie n’est pas disponible. Sélectionnez le texte pour le copier."); return false; }
    },
  };

  function render(followGroup = false) {
    const conversation = currentConversation();
    // React commits asynchronously: a frame callback alone can run before the
    // admitted turns/cards exist, making the next poll wrongly think we scrolled up.
    const follow = conversation?.kind === "group" && (followGroup || ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90);
    const onLayout = follow ? () => { if (currentConversation()?.id === conversation.id) ui.scroll.scrollTop = ui.scroll.scrollHeight; } : undefined;
    const hasMessages = Boolean(conversation?.messages.length);
    const name = state.account.name || "Espace personnel";
    $("account-name").textContent = name;
    $("account-avatar").textContent = name.charAt(0).toLocaleUpperCase("fr");
    ui.title.textContent = conversation?.title || "Nouvelle conversation";
    const bot = currentBot();
    const label = $("workspace-label");
    label.textContent = conversation?.kind === "group" ? "Groupe" : conversation ? botName(conversation.bot_id) : bot?.name || "Espace personnel";
    // Le groupe ne désigne pas la personnalité de son seul coordinateur.
    label.disabled = !conversation || conversation.kind === "group" || !bot || bot.deleted;
    if (label.disabled) label.removeAttribute("title");
    else label.title = "Modifier la personnalité";
    if (bot) document.querySelector(".main-panel").dataset.botColor = bot.color;
    else delete document.querySelector(".main-panel").dataset.botColor;
    const hero = document.querySelector("[data-mascot=hero]");
    hero.setAttribute("aria-label", `Faire sourire ${bot?.name || "Ezerbot"}`);
    hero.hidden = conversation?.kind === "group";
    hero.parentElement.querySelector(".group-hero")?.remove();
    if (hero.hidden) {
      const stack = groupAvatars(conversation); stack.classList.add("group-hero");
      hero.parentElement.append(stack);
    }
    // La marque et la barre latérale gardent le galet : c'est l'identité de l'app.
    for (const node of document.querySelectorAll("[data-mascot=hero] .mascot, [data-mascot=composer] .mascot")) mascot.setShape(node, bot?.shape);
    thread.createMascot = threadMascot(bot?.shape || "galet");
    $("edit-bot-button").disabled = !bot || bot.deleted;
    document.title = conversation ? `${conversation.title} · Ezerbot` : "Ezerbot";
    ui.welcome.hidden = hasMessages;
    document.querySelector(".bot-picker").hidden = conversation?.kind === "group";
    ui.messages.hidden = !hasMessages;
    window.GrokbotChat.renderThread(ui.messages, { messages: conversation?.messages || [], busy, botName: bot?.name || "Ezerbot", bots: state.bots, createBotMascot, isGroup: conversation?.kind === "group", isTask: conversation?.kind === "task", onLayout, ...thread });
    $("favorite-button").querySelector("span").textContent = conversation?.favorite ? "Retirer des favoris" : "Ajouter aux favoris";
    renderList();
    renderBots();
    renderProvider();
    for (const panel of panels.values()) panelCall(panel, panel.onState, state);
    updateComposer();
  }

  function pill(kind, label, value, enabled = true) {
    const button = element("button", "tool-pill");
    button.type = "button";
    button.dataset.picker = kind;
    button.disabled = !enabled;
    button.setAttribute("aria-haspopup", "menu");
    button.setAttribute("aria-expanded", String(pickerKind === kind));
    button.append(element("small", "", label), element("b", "", value), icon("chevron"));
    return button;
  }

  function option(label, hint, checked, change) {
    const button = element("button");
    button.type = "button";
    button.setAttribute("role", "menuitemradio");
    button.setAttribute("aria-checked", String(checked));
    const copy = element("span", "", label);
    if (hint) copy.append(element("small", "", hint));
    button.append(copy, icon("check"));
    button.addEventListener("click", async () => {
      closePicker();
      try {
        const response = await api("/api/preferences", change);
        state.preferences = { ...state.preferences, ...response.preferences };
      } catch (error) { notify(error.message); }
      renderProvider();
    });
    return button;
  }

  function closePicker() { pickerKind = null; $("picker").hidden = true; }

  function renderPicker() {
    const picker = $("picker");
    picker.hidden = !pickerKind;
    if (!pickerKind) return;
    const { models = [] } = provider();
    const { model, effort, fast } = state.preferences || {};
    const selected = models.find((item) => item.slug === model);
    const title = { model: "Modèle", effort: "Effort de réflexion", speed: "Vitesse" }[pickerKind];
    const options = pickerKind === "model" ? models.map((item) => option(item.name, item.description, item.slug === model, { model: item.slug }))
      : pickerKind === "effort" ? (selected?.efforts || []).map((level) => option(effortName(level.effort), level.description, level.effort === effort, { effort: level.effort }))
      : [option("Standard", "", !fast, { fast: false }), ...(selected?.tiers || []).slice(0, 1).map((tier) => option(tier.name, tier.description, Boolean(fast), { fast: true }))];
    const foot = element("div", "picker-foot");
    const link = element("a", "usage-link", "Gérer l’utilisation");
    link.href = usageUrl; link.target = "_blank"; link.rel = "noreferrer";
    foot.append(element("span", "", "Abonnement ChatGPT utilisé"), link);
    picker.replaceChildren(element("h4", "", title), ...options, foot);
  }

  function renderProvider() {
    const current = provider();
    const { model, effort, fast } = state.preferences || {};
    const selected = (current.models || []).find((item) => item.slug === model);
    $("mode-badge-text").textContent = current.connected ? "ChatGPT" : "Local";
    $("plan-banner").hidden = current.connected;
    document.querySelectorAll("[data-provider-signin]").forEach((button) => { button.disabled = current.signin === "pending"; });

    const tools = $("composer-tools");
    if (current.connected) {
      tools.replaceChildren(
        pill("model", "Modèle", selected?.name || (current.models_ready ? "Aucun modèle" : "Chargement…"), Boolean(current.models?.length)),
        pill("effort", "Effort", selected?.efforts.length ? effortName(effort) : "Par défaut", Boolean(selected?.efforts.length)),
        // Speed is offered only by models that list a service tier.
        ...(selected?.tiers.length ? [pill("speed", "Vitesse", fast ? selected.tiers[0].name : "Standard")] : []),
      );
      if (pickerKind === "speed" && !selected?.tiers.length) pickerKind = null;
    } else {
      closePicker();
      tools.replaceChildren(document.createTextNode(`${currentBot()?.name || "Ezerbot"} `), element("span", "composer-local", "Local"));
    }
    renderPicker();

    const note = $("mode-note");
    if (current.connected) {
      const link = element("a", "", "Gérer l’utilisation");
      link.href = usageUrl; link.target = "_blank"; link.rel = "noreferrer";
      note.replaceChildren(document.createTextNode("Abonnement ChatGPT utilisé · "), link);
    } else {
      note.replaceChildren(element("span", "simulation-dot"), document.createTextNode("Réponses simulées"));
    }

    const card = element("div", "provider-card");
    const copy = element("div");
    const actions = element("div", "provider-actions");
    const heading = element("h3", "", current.connected ? "Connecté à ChatGPT" : "Utiliser votre abonnement ChatGPT");
    if (current.connected) heading.prepend(element("span", "status-dot"));
    copy.append(heading, element("p", "", current.connected ? current.email || "Compte ChatGPT"
      : current.signin === "pending" ? "Terminez la connexion dans votre navigateur. Cette fenêtre se met à jour toute seule."
      : "Les réponses d’Ezerbot utilisent l’usage inclus dans votre abonnement ChatGPT. Sans connexion, les réponses restent simulées."));
    if (current.signin === "failed" && current.error) copy.append(element("p", "provider-error", current.error));
    const action = (label, className, run) => { const button = element("button", className, label); button.type = "button"; button.addEventListener("click", run); return button; };
    if (current.connected) {
      const link = element("a", "secondary-button", "Gérer l’utilisation");
      link.href = usageUrl; link.target = "_blank"; link.rel = "noreferrer";
      actions.append(link, action("Se déconnecter", "secondary-button", signOut));
    } else if (current.signin === "pending") {
      actions.append(action("Annuler", "secondary-button", () => providerAction("/api/provider/cancel")));
    } else {
      actions.append(action("Continuer avec ChatGPT", "chatgpt-button", signIn));
    }
    card.append(copy, actions);
    $("provider-section").replaceChildren(card);

    const dialog = $("plan-dialog");
    if (current.connected && !current.welcomed && !dialog.open) dialog.showModal();
  }

  async function providerAction(path) {
    try {
      const response = await api(path, {});
      state.provider = response.provider;
      renderProvider();
      schedulePoll();
      return response;
    } catch (error) { notify(error.message); return null; }
  }

  function signIn() { return providerAction("/api/provider/signin"); }

  async function signOut() {
    const response = await providerAction("/api/provider/signout");
    if (response && !response.revoked) notify("Déconnecté sur ce Mac. OpenAI n’a pas confirmé : vous pouvez retirer Ezerbot dans les réglages de ChatGPT.");
    if (response) await refresh().catch(() => {});
  }

  async function generation(path, messageId) {
    const conversation = currentConversation();
    if (!conversation) return;
    const pinned = ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90;
    try {
      const response = await api(path, { conversation_id: conversation.id, message_id: messageId });
      upsertConversation(response.conversation);
      render();
      if (conversation.kind === "group" && currentConversation()?.id === conversation.id && pinned) scrollToLatest();
      schedulePoll();
    } catch (error) { notify(error.message); }
  }

  function scrollToLatest() {
    requestAnimationFrame(() => { ui.scroll.scrollTop = ui.scroll.scrollHeight; });
  }

  function newConversation() {
    if (busy) return;
    // Keep this a local draft; sendMessage creates the thread with the final bot choice.
    selectConversation(null, true);
  }

  function sendRequest(conversation, messageId, body, extras, loss = false) {
    if (conversation.kind === "task") {
      return api("/api/delegation/send", {
        parent_conversation_id: conversation.parent_id,
        task_id: window.GrokbotDelegation?.taskId(conversation.id),
        message_id: messageId, body, attachments: extras.attachments || [],
      });
    }
    return api("/api/send", { ...extras, conversation_id: conversation.id, message_id: messageId, body, simulate_loss: loss });
  }

  async function sendMessage(event) {
    event.preventDefault();
    const live = streamingMessage();
    if (live && ui.send.dataset.mode === "stop") { await generation("/api/stop", currentConversation()?.kind === "group" ? "groupe" : live.id.replace(/^reply-/, "")); return; }
    if (ui.send.disabled) return;
    const submittedDraft = ui.input.value;
    const submittedDocument = ui.input.document;
    const body = submittedDraft.trim();
    const extras = structuredClone(sendExtras());
    let conversation = currentConversation();
    busy = true;
    submissionActivity = "send";
    updateComposer();
    let messageId;
    let delivered = false;
    const previousDraftKey = draftKey();
    const restoreDraft = () => {
      const remainder = ui.input.value;
      const remainderDocument = ui.input.document;
      setComposerText(submittedDraft && remainder ? `${submittedDraft}\n${remainder}` : submittedDraft || remainder);
      ui.input.document = submittedDraft && remainder
        ? { type: "doc", content: [...submittedDocument.content, ...remainderDocument.content] }
        : submittedDraft ? submittedDocument : remainderDocument;
      saveDraft();
    };
    try {
      // Consume only the submitted text, before creation can yield to another edit.
      setComposerText("");
      removePreference(previousDraftKey);
      removePreference(`${previousDraftKey}.document`);
      if (!conversation) {
        const created = await api("/api/conversations", { bot_id: chosenBot()?.id });
        upsertConversation(created.conversation);
        conversation = created.conversation;
        selectedId = conversation.id;
        writePreference("grokbot.selected", selectedId);
      }
      messageId = crypto.randomUUID();
      pendingSends.set(messageId, { extras, consumed: false });
      conversation.messages.push({ ...extras, ...(conversation.kind === "task" ? { payload: { source: "follow_up" } } : {}), id: messageId, role: "user", body, state: "queued", created_at: new Date().toISOString() });
      removePreference(previousDraftKey);
      saveDraft(); // Keep any text typed while conversation creation was in flight.
      render(true);
      scrollToLatest();
      const response = await sendRequest(conversation, messageId, body, extras, simulateLoss);
      simulateLoss = false;
      $("simulate-loss").checked = false;
      upsertConversation(response.conversation);
      delivered = response.conversation.messages.some((message) => message.id === messageId && ["accepted", "echoed"].includes(message.state));
      render(true);
      scrollToLatest();
      schedulePoll();
    } catch (error) {
      if (messageId) {
        try {
          await refresh();
          const savedMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          if (!savedMessage) { pendingSends.delete(messageId); restoreDraft(); }
          notify(error.message);
        } catch {
          const optimisticMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          if (optimisticMessage) optimisticMessage.state = "unknown";
          render();
          showConnectionError(error);
        }
      } else { restoreDraft(); notify(error.message); }
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
      const response = await sendRequest(conversation, message.id, message.body,
        pendingSends.get(message.id)?.extras || (message.attachments?.length ? { attachments: message.attachments.map((item) => typeof item === "string" ? item : item.id || item.attachment_id) } : {}));
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
    themeChoice = ["light", "dark", "system"].includes(choice) ? choice : "dark";
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
    anchor.download = `ezerbot-${conversation.id}.json`;
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
  ui.input = window.GrokbotChat.mountInput($("composer-input"), {
    placeholder: "Qu’avez-vous en tête ?",
    onInput: () => { saveDraft(); updateComposer(); },
    createMentionAvatar: (bot) => createBotMascot(bot)(),
    mentionSummary: botSummary,
    getMentionCandidates: () => {
      const conversation = currentConversation();
      const ids = conversation?.member_ids || [chosenBot()?.id];
      return [
        ...(ids.length > 1 ? [{ id: "@everyone", name: "everyone" }] : []),
        ...liveBots().filter((bot) => ids.includes(bot.id)),
        ...(conversation ? liveBots().filter((bot) => !ids.includes(bot.id)).map((bot) => ({ ...bot, outside: true })) : []),
      ];
    },
    onAddMember: async (botId) => {
      const conversation = currentConversation();
      if (!conversation) return false;
      try {
        const response = await api("/api/conversations/members/add", { conversation_id: conversation.id, bot_id: botId, command_id: crypto.randomUUID() });
        if (currentConversation()?.id !== conversation.id) return false;
        upsertConversation(response.conversation); render(); return true;
      } catch (error) { notify(error.message); return false; }
    },
    onSubmit: () => { if (!ui.send.disabled && ui.send.dataset.mode !== "stop") $("composer-form").requestSubmit(); },
    onFiles: composerFiles,
  });
  $("composer-form").addEventListener("dragover", (event) => {
    if (event.dataTransfer?.types.includes("Files") && [...panels.values()].some((panel) => panel.composer?.onFiles)) event.preventDefault();
  });
  $("composer-form").addEventListener("drop", (event) => {
    if (!event.defaultPrevented) composerFiles(event.dataTransfer?.files, event);
  });
  $("composer-form").addEventListener("submit", sendMessage);
  document.querySelectorAll(".suggestion").forEach((button) => button.addEventListener("click", () => {
    setComposerText(button.dataset.prompt.replace(/\\n/g, "\n"));
    saveDraft();
    updateComposer();
    ui.input.focus();
  }));
  ui.menuButton.addEventListener("click", () => {
    ui.menu.hidden = !ui.menu.hidden;
    ui.menuButton.setAttribute("aria-expanded", String(!ui.menu.hidden));
  });
  document.addEventListener("click", (event) => {
    if (!ui.menu.contains(event.target) && !ui.menuButton.contains(event.target)) closeMenu();
    const opener = event.target.closest("[data-picker]");
    if (opener) { pickerKind = pickerKind === opener.dataset.picker ? null : opener.dataset.picker; renderProvider(); }
    else if (pickerKind && !$("picker").contains(event.target)) { closePicker(); renderProvider(); }
    if (event.target.closest("[data-provider-signin]")) signIn();
  });
  $("plan-acknowledge").addEventListener("click", async () => { $("plan-dialog").close(); await providerAction("/api/provider/welcomed"); });
  $("sidebar-toggle").addEventListener("click", () => {
    const open = document.body.classList.toggle("sidebar-open");
    $("sidebar-toggle").setAttribute("aria-expanded", String(open));
    if (open) ui.search.focus();
  });
  $("sidebar-close").addEventListener("click", closeSidebar);
  $("sidebar-scrim").addEventListener("click", closeSidebar);
  $("settings-button").addEventListener("click", () => { closeSidebar(); $("settings-dialog").showModal(); });
  document.querySelectorAll("[data-close-dialog]").forEach((button) => button.addEventListener("click", () => button.closest("dialog").close()));
  function applyAccent(choice) {
    accentChoice = accents.includes(choice) ? choice : "citron";
    document.documentElement.dataset.accent = accentChoice;
    writePreference("grokbot.accent", accentChoice);
    document.querySelectorAll("[data-accent-choice]").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.accentChoice === accentChoice)));
  }

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
  document.querySelectorAll("[data-accent-choice]").forEach((button) => button.addEventListener("click", async () => {
    const accent = button.dataset.accentChoice;
    const previous = accentChoice;
    applyAccent(accent);
    try {
      const response = await api("/api/preferences", { accent });
      applyAccent(response.preferences?.accent || accent);
    } catch (error) { applyAccent(previous); notify(error.message); }
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
  $("edit-bot-button").addEventListener("click", () => { const bot = currentBot(); if (bot) openBotDialog(bot); });
  $("workspace-label").addEventListener("click", () => { const bot = currentBot(); if (bot && !bot.deleted) openBotDialog(bot); });
  document.querySelectorAll("[data-new-bot]").forEach((button) => button.addEventListener("click", () => openBotDialog(null)));
  // Les pastilles du bot reprennent celles de l'accent, sans leur effet sur l'application.
  $("bot-colors").replaceChildren(...[...document.querySelectorAll("[data-accent-choice]")].map((swatch) => {
    const button = swatch.cloneNode(true);
    button.dataset.botColor = swatch.dataset.accentChoice;
    button.removeAttribute("data-accent-choice");
    button.type = "button";
    button.addEventListener("click", () => selectBotColor(button.dataset.botColor));
    return button;
  }));
  $("bot-shapes").replaceChildren(...Object.entries(mascot?.shapes || {}).map(([shape, { label }]) => {
    const button = element("button");
    button.type = "button";
    button.dataset.shape = shape;
    const preview = element("span", "bot-avatar");
    preview.append(mascot.create(false, shape));
    button.append(preview, element("span", "", label));
    button.addEventListener("click", () => selectBotShape(shape));
    return button;
  }));
  $("bot-dialog").addEventListener("close", () => { if (!$("bot-dialog").open) clearBotExtensions(); });
  $("bot-form").addEventListener("submit", saveBot);
  $("bot-delete-button").addEventListener("click", () => $("bot-delete-dialog").showModal());
  $("confirm-bot-delete").addEventListener("click", deleteBot);
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
      setComposerText("");
      selectedId = null;
      writePreference("grokbot.selected", "");
      setComposerText(readPreference(draftKey(), ""));
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
    if (event.key === "Escape") { closeMenu(); closeSidebar(); if (pickerKind) { closePicker(); renderProvider(); } }
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
  applyAccent(accentChoice);
  setComposerText(readPreference(draftKey(), ""));
  render();
  refresh().then(() => {
    setComposerText(readPreference(draftKey(), ""));
    restoreDraftDocument();
    updateComposer();
    scrollToLatest();
  }).catch(showConnectionError);
})();
