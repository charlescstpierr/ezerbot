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
  const effortNames = { none: "None", minimal: "Minimal", low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max", ultra: "Ultra" };
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
  function botById(id) { return state.bots.find((bot) => bot.id === id); }
  // Les bots supprimés restent dans l'état pour que leurs conversations gardent nom et couleur.
  function liveBots() { return state.bots.filter((bot) => !bot.deleted); }
  // Un choix devenu caduc (bot supprimé ailleurs) retombe sur le bot par défaut.
  function chosenBot() { const bot = botById(chosenBotId); return bot && !bot.deleted ? bot : liveBots()[0]; }
  function currentBot() { const conversation = currentConversation(); return conversation ? botById(conversation.bot_id) : chosenBot(); }
  function botName(id) { const bot = botById(id); return bot ? `${bot.name}${bot.deleted ? " (supprimé)" : ""}` : "Bot inconnu"; }
  function provider() { return state.provider || { connected: false, signin: "idle", models: [] }; }
  function streamingMessage(conversation = currentConversation()) { return conversation?.messages.find((message) => message.state === "streaming"); }
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
    state = await api("/api/state");
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
    if (pinned && streamingMessage()) scrollToLatest();
    schedulePoll();
  }

  // The engine writes a reply while it streams; the shell re-reads it until every reply settles.
  function schedulePoll() {
    clearTimeout(pollTimer);
    const live = state.conversations.some((conversation) => streamingMessage(conversation));
    const waiting = provider().signin === "pending" || provider().titling || (provider().connected && !provider().models_ready);
    if (!live && !waiting) return;
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

  function updateComposer() {
    const pendingMessage = currentConversation()?.messages.find((message) => message.role === "user" && pendingStates.has(message.state));
    const pending = Boolean(pendingMessage);
    const tooLong = new TextEncoder().encode(ui.input.value.trim()).length > 16 * 1024;
    const live = Boolean(streamingMessage());
    ui.send.disabled = live ? false : busy || !connected || !ui.input.value.trim() || pending || tooLong;
    ui.send.dataset.mode = live ? "stop" : "send";
    ui.send.title = live ? "Arrêter la réponse" : "Envoyer le message";
    ui.send.setAttribute("aria-label", live ? "Arrêter la réponse" : busy ? "Envoi en cours" : "Envoyer le message");
    ui.send.querySelector("use").setAttribute("href", live ? "#i-stop" : "#i-send");
    ui.input.setInvalid(tooLong);
    ui.hint.dataset.alert = String(Boolean(pending || tooLong));
    ui.hint.textContent = pending ? "Vérifiez le dernier envoi pour poursuivre la conversation." : tooLong ? "Ce message est trop long. Raccourcissez-le pour l’envoyer." : "Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne";
    $("new-chat").disabled = busy;
    ui.menuButton.disabled = busy || !currentConversation();
    mascot?.setContext({ connection: connected ? "online" : connectionStatus,
      activity: submissionActivity, pending: pendingMessage?.state || null, drafting: Boolean(ui.input.value.trim()) });
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
      const bot = botById(conversation.bot_id);
      const date = timestamp(conversation.updated_at).toDateString();
      const group = date === today ? "Aujourd’hui" : date === yesterday ? "Hier" : "Précédemment";
      if (group !== previousGroup) { ui.list.append(element("p", "list-group-title", group)); previousGroup = group; }
      const button = element("button", `conversation-item${conversation.id === selectedId ? " active" : ""}`);
      button.type = "button";
      button.dataset.conversationId = conversation.id;
      button.title = `${conversation.title} · ${botName(conversation.bot_id)}`;
      if (conversation.id === selectedId) button.setAttribute("aria-current", "page");
      button.append(botDot(bot), element("span", "conversation-name", conversation.title || "Nouvelle conversation"));
      if (conversation.messages.some((message) => message.state === "unknown")) button.append(element("span", "pending-indicator"));
      if (conversation.favorite) button.append(icon("star", "icon favorite-indicator"));
      button.addEventListener("click", () => { if (!busy) selectConversation(conversation.id); });
      ui.list.append(button);
    }
  }

  function botDot(bot) {
    const dot = element("span", "bot-dot");
    dot.setAttribute("aria-hidden", "true");
    if (bot) dot.dataset.botColor = bot.color;
    return dot;
  }

  function renderBots() {
    const chosen = chosenBot();
    const chips = liveBots().map((bot) => {
      const chip = element("button", "bot-chip");
      chip.type = "button";
      chip.dataset.botColor = bot.color;
      chip.setAttribute("aria-pressed", String(bot.id === chosen?.id));
      if (bot.description) chip.title = bot.description;
      chip.append(botDot(bot), element("span", "", bot.name));
      chip.addEventListener("click", () => { chosenBotId = bot.id; writePreference("grokbot.bot", bot.id); render(); });
      return chip;
    });
    const add = element("button", "bot-chip add-bot");
    add.type = "button";
    add.append(icon("plus"), element("span", "", "Nouveau bot"));
    add.addEventListener("click", () => openBotDialog(null));
    $("bot-chips").replaceChildren(...chips, add);
    $("welcome-description").textContent = chosen?.description || defaultWelcome;

    $("bot-list").replaceChildren(...liveBots().map((bot) => {
      const row = element("div", "bot-row");
      const copy = element("div");
      copy.append(element("strong", "", bot.name), element("small", "", bot.description || (bot.is_default ? "Le bot par défaut." : "Aucune description.")));
      const edit = element("button", "secondary-button", "Modifier");
      edit.type = "button";
      edit.setAttribute("aria-label", `Modifier ${bot.name}`);
      edit.addEventListener("click", () => openBotDialog(bot));
      row.append(botDot(bot), copy, edit);
      return row;
    }));
  }

  function openBotDialog(bot) {
    closeMenu();
    editingBotId = bot?.id || null;
    if (!bot && !draftBotId) draftBotId = crypto.randomUUID();
    const values = bot || { name: "", color: chosenBot()?.color || "citron", description: "", instructions: "" };
    $("bot-dialog-title").textContent = bot ? `Modifier ${bot.name}` : "Nouveau bot";
    $("bot-save").textContent = bot ? "Enregistrer" : "Créer le bot";
    $("bot-name").value = values.name;
    $("bot-description").value = values.description;
    $("bot-instructions").value = values.instructions;
    selectBotColor(values.color);
    $("bot-delete-button").hidden = !bot || bot.is_default;
    $("bot-dialog").showModal();
    $("bot-name").focus();
  }

  function selectBotColor(color) {
    $("bot-colors").querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.botColor === color)));
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
      if (!existing) { draftBotId = null; chosenBotId = bot.id; writePreference("grokbot.bot", bot.id); }
      $("bot-dialog").close();
      render();
      notify(existing ? "Bot enregistré." : `${bot.name} est prêt.`);
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
      render();
      notify(`${bot.name} a été supprimé.`);
    } catch (error) { notify(error.message); }
    finally { $("confirm-bot-delete").disabled = false; }
  }

  const thread = {
    createMascot: () => mascot?.create(),
    onRetry: (message) => message.state === "queued" ? resumeQueued(message) : reconcile(message.id),
    onRegenerate: (message) => generation("/api/regenerate", message.id.replace(/^reply-/, "")),
    onSignIn: () => signIn(),
    usageUrl,
    async onCopy(text) {
      try { await copyText(text); return true; }
      catch { notify("La copie n’est pas disponible. Sélectionnez le texte pour le copier."); return false; }
    },
  };

  function render() {
    const conversation = currentConversation();
    const hasMessages = Boolean(conversation?.messages.length);
    const name = state.account.name || "Espace personnel";
    $("account-name").textContent = name;
    $("account-avatar").textContent = name.charAt(0).toLocaleUpperCase("fr");
    ui.title.textContent = conversation?.title || "Nouvelle conversation";
    const bot = currentBot();
    $("workspace-label").textContent = conversation ? botName(conversation.bot_id) : "Espace personnel";
    if (bot) document.querySelector(".main-panel").dataset.botColor = bot.color;
    else delete document.querySelector(".main-panel").dataset.botColor;
    document.querySelector("[data-mascot=hero]").setAttribute("aria-label", `Faire sourire ${bot?.name || "Grokbot"}`);
    $("edit-bot-button").disabled = !bot || bot.deleted;
    document.title = conversation ? `${conversation.title} · Grokbot` : "Grokbot";
    ui.welcome.hidden = hasMessages;
    ui.messages.hidden = !hasMessages;
    window.GrokbotChat.renderThread(ui.messages, { messages: conversation?.messages || [], busy, botName: bot?.name || "Grokbot", ...thread });
    $("favorite-button").querySelector("span").textContent = conversation?.favorite ? "Retirer des favoris" : "Ajouter aux favoris";
    renderList();
    renderBots();
    renderProvider();
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
      tools.replaceChildren(document.createTextNode(`${currentBot()?.name || "Grokbot"} `), element("span", "composer-local", "Local"));
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
      : "Les réponses de Grokbot utilisent l’usage inclus dans votre abonnement ChatGPT. Sans connexion, les réponses restent simulées."));
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
    if (response && !response.revoked) notify("Déconnecté sur ce Mac. OpenAI n’a pas confirmé : vous pouvez retirer Grokbot dans les réglages de ChatGPT.");
    if (response) await refresh().catch(() => {});
  }

  async function generation(path, messageId) {
    const conversation = currentConversation();
    if (!conversation) return;
    try {
      const response = await api(path, { conversation_id: conversation.id, message_id: messageId });
      upsertConversation(response.conversation);
      render();
      schedulePoll();
    } catch (error) { notify(error.message); }
  }

  function scrollToLatest() {
    requestAnimationFrame(() => { ui.scroll.scrollTop = ui.scroll.scrollHeight; });
  }

  async function newConversation() {
    if (busy) return;
    busy = true;
    updateComposer();
    try {
      const { conversation } = await api("/api/conversations", { bot_id: chosenBot()?.id });
      upsertConversation(conversation);
      selectConversation(conversation.id, true);
    } catch (error) { notify(error.message); }
    finally { busy = false; updateComposer(); }
  }

  async function sendMessage(event) {
    event.preventDefault();
    const live = streamingMessage();
    if (live) { await generation("/api/stop", live.id.replace(/^reply-/, "")); return; }
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
        const created = await api("/api/conversations", { bot_id: chosenBot()?.id });
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
      schedulePoll();
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
  ui.input = window.GrokbotChat.mountInput($("composer-input"), {
    placeholder: "Qu’avez-vous en tête ?",
    onInput: () => { saveDraft(); updateComposer(); },
    onSubmit: () => { if (!ui.send.disabled && !streamingMessage()) $("composer-form").requestSubmit(); },
  });
  $("composer-form").addEventListener("submit", sendMessage);
  document.querySelectorAll(".suggestion").forEach((button) => button.addEventListener("click", () => {
    ui.input.value = button.dataset.prompt.replace(/\\n/g, "\n");
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
  ui.input.value = readPreference(draftKey(), "");
  render();
  refresh().then(() => {
    ui.input.value = readPreference(draftKey(), "");
    updateComposer();
    scrollToLatest();
  }).catch(showConnectionError);
})();
