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
  let state = { account: { name: "Espace personnel" }, mode: "simulated", conversations: [], bots: [], threads: {} };
  let selectedId = readPreference("grokbot.selected", null);
  let chosenBotId = readPreference("grokbot.bot", "grokbot");
  // Le bot ouvert dans le dialogue; `null` pour une création, dont l'identifiant reste stable entre deux essais.
  let editingBotId = null;
  let draftBotId = null;
  let botDialogInitial = "";
  const defaultWelcome = $("welcome-description").textContent;
  let favoriteOnly = false;
  let sidebarSection = "conversations";
  let connected = false;
  let connectionStatus = "loading";
  let busy = false;
  let submissionActivity = null;
  const mascot = window.GrokbotMascot;
  let toastTimer;
  let simulateLoss = false;
  let deepLinksBootReady = false;
  let linkDrafts = null;
  let navigationGeneration = 0;
  const deepLinks = window.GrokbotChat.createDeepLinkQueue({
    resolve: async (url) => { await refresh(); return api("/api/deep-links/resolve", { url }); },
    navigate: navigateLink, report: linkError,
    generation: () => navigationGeneration,
  });
  // Installed before the first state request; Cocoa only flushes after ready.
  window.EzerbotDeepLinks = { receive: (url) => deepLinks.receive(url) };
  let themeChoice = readPreference("grokbot.theme", "dark");
  let accentChoice = readPreference("grokbot.accent", "citron");
  const accents = ["citron", "orange", "ambre", "menthe", "cyan", "magenta", "vermillon", "violet", "blanc"];
  let preferencesLoaded = false;
  let pollTimer;
  let pickerKind = null;
  let revealedBotId = null;
  let renameAvatarColor = "citron";
  function usageUrl() {
    return "https://chatgpt.com/codex/settings/usage";
  }
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  const pendingStates = new Set(["queued", "dispatching", "unknown"]);

  const panels = new Map();
  const unavailablePanels = new Set(window.EzerbotShell?.platform === "linux" ? window.EzerbotShell.unavailablePanels : []);
  if (window.EzerbotShell?.platform === "linux") {
    const notice = document.createElement("p");
    notice.id = "linux-native-services";
    notice.textContent = "Indisponibles sous Linux : dictée, microphone, appels et mode voix, enregistrement local de réunion, capture et contrôle de l’écran, démonstrations et fenêtre flottante.";
    $("settings-extensions").before(notice);
    for (const paragraph of document.querySelectorAll(".setting-info p")) {
      paragraph.textContent = paragraph.textContent.replace("sur ce Mac", "sur cet ordinateur");
    }
    for (const row of $("settings-shortcuts").querySelectorAll("div")) {
      if (row.textContent.includes("Dicter un message")) row.hidden = true;
      const key = row.querySelector("kbd");
      if (key) key.textContent = key.textContent.replace("⌘", "Ctrl");
    }
  }
  const panelFailed = Symbol("panel-failed");
  const botCleanups = [];
  // Keep the original extras until a queued submission settles, even if state refresh replaces its DTO.
  const pendingSends = new Map();
  const approvalFollow = window.GrokbotChat.createApprovalFollow();
  async function openRatings() {
    const bridge = window.webkit?.messageHandlers?.ezerbotRatings;
    if (bridge) return window.GrokbotChat.createNativeRatingTable(bridge);
    try { return window.GrokbotChat.createRatingTable?.(localStorage) || null; }
    catch { return null; }
  }
  // Le démarrage n’attend pas le Mac : la table native remplace la table vide dès qu’elle est lue.
  const unavailable = () => { throw new Error("Notation indisponible."); };
  let ratings = { list() { return []; }, recordGood: unavailable, recordBad: unavailable, exportDocument() { return { notations: [] }; } };
  const ratingsReady = openRatings().then((table) => { if (table) ratings = table; }, () => {});
  let pendingRating = null;
  let findState = { open: false, query: "", index: 0, matches: [], conversationId: null };
  let followAfterLayout = null;
  let scrollGeneration = 0;
  // Section hooks mount once: settingsSection(container, context), botSection(bot, container, context).
  // botSection may return a cleanup function, called before remount and on dialog close.
  // onState(state, context), onConnection(connected, context), onCardAction(message, action, detail, context); only true claims an action.
  // wantsPolling(context) and sendExtras(context) are synchronous; sendExtras returns extra /api/send fields.
  // composer: mount(container, context), onFiles(files, event, context), blocked(context), capture(context),
  // restore({messageId, extras, captured}, context), onSent({messageId, extras, captured}, context). capture runs with sendExtras when a message is submitted; its value
  // comes back to the same panel as `captured`, so a panel tells what left with this message from what was staged since.
  // restore runs after a refused draft returns and may return a warning to display.
  // onSent runs once the server holds the message, including unknown/queued states, not on confirmation.
  // onFiles returns false to preserve normal paste/drop. Async uploads call composerChanged() when ready.
  const panelContext = () => ({ state, connected, conversation: currentConversation(), bot: currentBot(), draft: ui.input?.value || "" });
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
        for (const panel of panels.values()) panelCall(panel, panel.composer?.onSent, { messageId: message.id, extras: pending.extras, captured: pending.captured.get(panel.id) });
      }
      if (message.state !== "queued") pendingSends.delete(message.id);
    }
  }
  function sendExtras() {
    const extras = { mentions: ui.input?.mentions || [] };
    for (const panel of panels.values()) {
      const fields = panelCall(panel, panel.sendExtras);
      if (fields && typeof fields === "object" && !fields.then && !Array.isArray(fields)) {
        const attachments = [...(extras.attachments || []), ...(fields.attachments || [])];
        Object.assign(extras, fields);
        if (attachments.length) extras.attachments = attachments;
      }
    }
    return extras;
  }
  function captureComposer() {
    const captured = new Map();
    for (const panel of panels.values()) if (panel.composer?.capture) captured.set(panel.id, panelCall(panel, panel.composer.capture));
    return captured;
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
    if (action === "provider.signin") {
      await signIn();
      return true;
    }
    for (const panel of panels.values()) {
      if (!panel.onCardAction) continue;
      try {
        if (await panel.onCardAction(message, action, detail, panelContext()) === true) return true;
      } catch (error) {
        // Memory receipts show actionable refusals inline (for example while a
        // related reply is running), rather than losing the reason in a toast.
        if (message.kind === "memory_auto" || message.kind === "memory_share" || message.kind === "topic_memory_offer") throw error;
        // A refused line stays on its task card, next to the field it came from.
        if (action === "local.task.input") throw error;
        panelError(panel.id, error); notify(error.message); return false;
      }
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
      if (unavailablePanels.has(panel.id)) return false;
      if (window.EzerbotIPhone?.remote && !["iphone", "notifications", "forms", "attachments", "multibot", "engine", "delegation", "local"].includes(panel.id)) return false;
      panels.set(panel.id, panel);
      if (!window.EzerbotIPhone?.remote || ["iphone", "notifications", "forms"].includes(panel.id)) {
        mountSection(panel, panel.settingsSection, $("settings-extensions"));
      }
      if ($("bot-dialog").open) mountBotSection(panel, botById(editingBotId) || null);
      panelCall(panel, panel.composer?.mount, $("composer-attachments"));
      panelCall(panel, panel.onState, state);
      updateComposer();
      syncSidebarRail();
      schedulePoll();
      return true;
    },
    openConversation(id, messageId) {
      if (busy || !state.conversations.some((conversation) => conversation.id === id)) return false;
      selectConversation(id, !messageId);
      if (messageId) {
        let attempts = 0;
        const reveal = () => {
          const target = [...ui.messages.querySelectorAll("[data-message-id]")].find((el) => el.dataset.messageId === messageId);
          if (target) {
            if (target.dataset.cardKind === "approval" && target.dataset.requestStatus === "pending") window.GrokbotChat.revealPendingApproval(target, ui.scroll);
            else target.scrollIntoView({ block: "center" });
            target.tabIndex = -1; target.focus({ preventScroll: true });
            target.classList.add("routine-run-target"); setTimeout(() => target.classList.remove("routine-run-target"), 2500);
          }
          else if (++attempts < 20) setTimeout(reveal, 50);
        };
        // selectConversation schedules an initial jump to the latest message.
        // Reveal the selected run only after that frame and React's layout.
        requestAnimationFrame(() => requestAnimationFrame(reveal));
      }
      return true;
    },
    async focusConversation(id) {
      if (busy) return false;
      // Native panel navigation shares deep-link editor/draft protection and
      // closes overlays before revealing the complete conversation.
      await navigateLink({ kind: "conversation", id });
      return true;
    },
    // Quitte le fil affiché pour une page qui n’est pas un fil (la page d’un sujet).
    leaveConversation() {
      if (busy) return false;
      selectConversation(null);
      return true;
    },
    draft: () => ui.input?.value || "",
    draftDocument: () => ui.input?.document,
    appendDraft(text) { ui.input?.appendText(text); saveDraft(); updateComposer(); },
    get state() { return state; },
    get connected() { return connected; },
    get busy() { return busy; },
    api, refresh, notify, copyText, createBotMascot, botSummary,
    followVoiceReply(conversationId) {
      if (window.EzerbotVoiceMode?.active() && currentConversation()?.id === conversationId) render(true);
    },
    async sendVoice(text, conversationId) {
      const conversation = currentConversation();
      if (!connected || busy || conversation?.id !== conversationId || conversation.kind === "task") throw new Error("Ouvrez une conversation disponible pour envoyer la transcription.");
      if (!text.trim()) throw new Error("Aucune parole reconnue. Réessayez.");
      const messageId = crypto.randomUUID();
      busy = true; submissionActivity = "send";
      approvalFollow.arm(conversation.id, conversation.messages, true);
      conversation.messages.push({id:messageId,role:"user",body:text,state:"queued",created_at:new Date().toISOString()});
      render(true);
      try {
        const response = await api("/api/send", {conversation_id:conversationId,message_id:messageId,body:text,voice:true});
        upsertConversation(response.conversation); render(true); schedulePoll();
        return messageId;
      } catch (error) {
        try { await refresh(); } catch { showConnectionError(error); }
        throw new Error("Envoi vocal non confirmé. Vérifiez le fil avant de réessayer. " + error.message);
      } finally { busy = false; submissionActivity = null; updateComposer(); schedulePoll(); }
    },
    composerChanged() { updateComposer(); schedulePoll(); },
    selectBot(id, text) {
      const bot = botById(id);
      if (busy || !bot || bot.deleted) return false;
      if (stagedOnEmptyThread()) {
        notify(`Retirez les pièces jointes pour changer de bot, ou envoyez-les à ${currentBot()?.name || "ce bot"}.`);
        return false;
      }
      chosenBotId = id;
      writePreference("grokbot.bot", id);
      notifyNavigation();
      leaveEmptyThread();
      if (currentConversation()) selectConversation(null, true);
      if (typeof text === "string") { setComposerText(text); saveDraft(); }
      render();
      return true;
    },
  };

  function previousLinkDrafts() {
    if (linkDrafts === null) {
      try {
        const saved = JSON.parse(readPreference("ezerbot.link-previous-draft", "[]"));
        linkDrafts = Array.isArray(saved) ? saved.filter((draft) => typeof draft?.text === "string").slice(-16) : [];
      } catch { linkDrafts = []; }
    }
    return linkDrafts;
  }

  function saveLinkDrafts() {
    if (previousLinkDrafts().length) writePreference("ezerbot.link-previous-draft", JSON.stringify(linkDrafts));
    else removePreference("ezerbot.link-previous-draft");
  }

  function linkError(message) {
    $("link-error-message").textContent = message;
    if (!$("link-error-dialog").open) $("link-error-dialog").showModal();
  }

  function focusLinkTarget(target) {
    if (!target) throw new Error("Cette section n’est pas disponible dans cette version d’Ezerbot.");
    target.tabIndex = -1; target.scrollIntoView({ block: "start" }); target.focus({ preventScroll: true });
    target.classList.add("deep-link-target");
    setTimeout(() => target.classList.remove("deep-link-target"), 2500);
  }

  async function navigateLink(destination) {
    // Never discard an unsaved editor or silently substitute another target.
    const botEdited = $("bot-dialog").open && (JSON.stringify(botFormValues()) !== botDialogInitial
      || Boolean($("bot-extensions").querySelector(".memory-editor:not([hidden])")));
    if (botEdited || document.querySelector("#rename-dialog[open], .skills-dialog[open], .routines-dialog[open], dialog.gb-file-save:not(.gb-file-zoom)[open]")) {
      throw new Error("Fermez l’éditeur en cours, puis rouvrez ce lien. Vos saisies sont conservées.");
    }
    for (const dialog of document.querySelectorAll("dialog[open]")) dialog.close();
    const { kind, id, agent } = destination;
    if (kind === "conversation") {
      if (!state.conversations.some((item) => item.id === id)) throw new Error("Cette conversation est introuvable ou inaccessible sur ce Mac.");
      selectConversation(id, true);
    } else if (kind === "bot") {
      const bot = botById(id);
      if (!bot || bot.deleted) throw new Error("Ce bot est introuvable ou inaccessible sur ce Mac.");
      openBotDialog(bot);
    } else if (kind === "settings") {
      $("settings-dialog").showModal();
      for (const panel of panels.values()) panelCall(panel, panel.onSettingsLink, id);
      const target = ({ account: $("provider-section"), appearance: $("settings-appearance"), bots: $("settings-bots"), shortcuts: $("settings-shortcuts") })[id]
        || $("settings-extensions").querySelector(`[data-panel="${id}"]`);
      focusLinkTarget(target);
    } else if (kind === "routine" || kind === "skill") {
      const bot = botById(agent);
      if (!bot || bot.deleted) throw new Error("Ce bot est introuvable ou inaccessible sur ce Mac.");
      if (kind === "skill") openBotDialog(bot);
      const panel = panels.get(kind === "skill" ? "skills" : "routines");
      if (!panel?.openLink || await panel.openLink({ id, agent }) === false) throw new Error("La cible de ce lien est introuvable ou inaccessible sur ce Mac.");
    } else if (kind === "bot_template") {
      if (await panels.get("templates")?.openLink?.(destination) !== true) throw new Error("Ce modèle de bot est introuvable ou son lien est abîmé.");
    } else if (kind === "compose") {
      if (agent && (!botById(agent) || botById(agent).deleted)) throw new Error("Ce bot est introuvable ou inaccessible sur ce Mac.");
      if (stagedOnEmptyThread() || sendExtras().attachments?.length) throw new Error("Retirez les pièces jointes en cours ou envoyez-les vous-même avant de rouvrir ce lien. Rien n’a été envoyé.");
      const newDraftText = selectedId ? readPreference(draftKey(null), "") : ui.input.value;
      if (previousLinkDrafts().length >= 16 && newDraftText) throw new Error("Reprenez d’abord un brouillon précédent avant d’ouvrir un autre lien contenant du texte.");
      selectConversation(null);
      if (ui.input.value) {
        previousLinkDrafts().push({ text: ui.input.value, document: ui.input.document, bot: chosenBotId });
        saveLinkDrafts();
      }
      chosenBotId = agent || chosenBot()?.id;
      writePreference("grokbot.bot", chosenBotId);
      setComposerText(destination.text); saveDraft(); render(); ui.input.focus();
      notify("Texte placé dans le brouillon. Rien n’a été envoyé.");
    }
  }

  async function copyLink(path, fields) {
    closeMenu();
    const query = Object.entries(fields).sort(([a], [b]) => a.localeCompare(b)).map(([key, value]) => `${key}=${encodeURIComponent(value)}`).join("&");
    try { await copyText(`ezerbot://app/v1/${path}?${query}`); notify("Lien copié. Il n’accorde aucun droit d’accès."); }
    catch { notify("Le lien n’a pas pu être copié."); }
  }

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
    const anchor = [...document.querySelectorAll("dialog[open]")].at(-1) || document.querySelector(".composer-dock");
    const box = anchor.getBoundingClientRect();
    const footer = anchor.querySelector(".dialog-footer")?.getBoundingClientRect();
    const bottom = anchor.localName === "dialog"
      ? footer && footer.top >= box.top && footer.top < box.bottom ? innerHeight - footer.top + 8 : innerHeight - box.bottom + 16
      : innerHeight - box.top + 8;
    // Keep the live status inside the active modal (not inert), then enter its
    // top layer. Older WebKit also renders it here without the popover API.
    if (ui.toast.isConnected) ui.toast.hidePopover?.();
    anchor.append(ui.toast);
    ui.toast.style.setProperty("--toast-left", `${box.left + box.width / 2}px`);
    ui.toast.style.setProperty("--toast-bottom", `${bottom}px`);
    ui.toast.textContent = message;
    ui.toast.hidden = false;
    ui.toast.showPopover?.();
    toastTimer = setTimeout(() => {
      if (ui.toast.isConnected) ui.toast.hidePopover?.();
      ui.toast.hidden = true;
      document.querySelector(".composer-dock").append(ui.toast);
    }, 4000);
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
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 15000);
    try {
      let response;
      try { response = await fetch(path, { ...options, signal: controller.signal }); }
      catch (error) {
        showConnectionError(error);
        throw new Error("La connexion au moteur local est interrompue.");
      }
      let data;
      try { data = await response.json(); }
      catch (error) { showConnectionError(error); throw new Error("Le moteur local a renvoyé une réponse illisible."); }
      if (!response.ok) {
        const error = new Error(data.error || "L’action n’a pas pu être terminée.");
        error.status = response.status;
        if (remoteAccessDisabled(response.status, data)) {
          error.accessDisabled = true;
          showConnectionError(error);
        }
        throw error;
      }
      return data;
    } finally { clearTimeout(timeout); }
  }

  let refreshWork;
  function refresh() {
    if (!refreshWork) refreshWork = loadState().finally(() => { refreshWork = null; });
    return refreshWork;
  }
  async function loadState() {
    const previous = currentConversation();
    const wasLive = Boolean(streamingMessage(previous));
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
    for (const panel of panels.values()) panelCall(panel, panel.onConnection, true);
    if (selectedId && !state.conversations.some((item) => item.id === selectedId)) selectedId = null;
    writePreference("grokbot.selected", selectedId || "");
    const pinned = ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90;
    const current = currentConversation();
    // A terminal error (or a one-chunk success) may never have a streaming DOM
    // frame. Follow the transition after React commits, not just live deltas.
    render(pinned && (Boolean(streamingMessage()) || (current?.id === previous?.id && (wasLive || current?.messages.at(-1)?.id !== previousTail))));
    schedulePoll();
  }

  // The engine writes a reply while it streams; the shell re-reads it until every reply settles.
  function schedulePoll() {
    clearTimeout(pollTimer);
    const live = state.conversations.some((conversation) => streamingMessage(conversation));
    const waiting = provider().signin === "pending" || provider().connection_test?.state === "running" || provider().titling || (provider().connected && !provider().models_ready);
    const panelWaiting = [...panels.values()].some((panel) => panelCall(panel, panel.wantsPolling) === true);
    // A failed refresh must never disarm recovery. An idle heartbeat also
    // notices a wrapper restart or a run started by another window.

    pollTimer = setTimeout(async () => {
      if (busy) { schedulePoll(); return; }
      try {
        if (live || waiting || !connected) await refresh();
        else {
          // Idle liveness must not serialize/repaint the entire workspace.
          const ping = await api("/api/ping");
          if (ping.revision !== state.state_revision) await refresh();
          else {
            for (const panel of panels.values()) if (panelCall(panel, panel.wantsPolling) === true) panelCall(panel, panel.onState, state);
            schedulePoll();
          }
        }
      } catch (error) {
        if (error?.accessDisabled) showConnectionError(error);
        schedulePoll();
      }
    }, !connected ? 1500 : live ? 200 : waiting || panelWaiting ? 800 : 5000);
  }

  function remoteAccessDisabled(status, data) {
    const message = String(data?.error || "").replaceAll("'", "’");
    return Boolean(window.EzerbotIPhone?.remote && status === 403 && message === "L’accès iPhone est désactivé sur le Mac.");
  }

  function showConnectionError(error) {
    connected = false;
    connectionStatus = error?.accessDisabled ? "disabled" : "offline";
    ui.bannerText.textContent = error?.accessDisabled
      ? "L’accès iPhone est désactivé sur le Mac. Réactivez-le dans Paramètres › iPhone. Reconnexion automatique…"
      : window.EzerbotIPhone?.remote
      ? "Mac injoignable. Vérifiez que le Mac est allumé, Ezerbot ouvert et Tailscale connecté. Reconnexion automatique…"
      : "Le moteur local ne répond plus. Vos conversations enregistrées sont conservées.";
    ui.banner.hidden = false;
    render();
    for (const panel of panels.values()) panelCall(panel, panel.onConnection, false);
    schedulePoll();
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
      const text = readPreference(draftKey(), null);
      // A rich document without its text twin is an orphan from an earlier transfer.
      if (text === null) { removePreference(`${draftKey()}.document`); return; }
      const document = JSON.parse(readPreference(`${draftKey()}.document`, "null"));
      if (document && text === ui.input.value) ui.input.document = document;
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
    const blockingPanel = [...panels.values()].find((panel) => Boolean(panelCall(panel, panel.composer?.blocked)));
    const attachmentsBlocked = Boolean(blockingPanel);
    const blockedReason = blockingPanel && panelCall(blockingPanel, blockingPanel.composer?.blockedReason);
    // The task banner is the single cancellation control. The composer only sends follow-ups.
    const control = window.GrokbotChat.composerConnectionState({ offline: connectionStatus === "offline", busy, group,
      live: !task && Boolean(streamingMessage()) && (!group || (!ui.input.value.trim() && !hasAttachments)) });
    const live = control.live;
    const noMembers = group && !currentConversation().member_ids?.length;
    ui.send.disabled = control.offline || taskFinished || (live ? ((group || task) && (control.busy || !connected)) : control.busy || !connected || noMembers || (!ui.input.value.trim() && !hasAttachments) || pending || tooLong || attachmentsBlocked);
    ui.send.dataset.mode = control.mode;
    ui.send.title = control.title;
    ui.send.setAttribute("aria-label", control.label);
    ui.send.querySelector("use").setAttribute("href", control.icon);
    ui.input.setInvalid(tooLong);
    ui.hint.dataset.alert = String(Boolean(pending || tooLong || blockedReason));
    ui.hint.textContent = taskFinished ? "Cette tâche est terminée ; poursuivez dans la conversation principale." : noMembers ? "Ajoutez un bot au groupe pour poursuivre." : pending ? "Vérifiez le dernier envoi pour poursuivre la conversation." : tooLong ? "Ce message est trop long. Raccourcissez-le pour l’envoyer." : typeof blockedReason === "string" ? blockedReason : task ? "Suivi réservé à cette sous-tâche · Entrée pour envoyer" : group && live ? "Écrivez pour envoyer un nouveau message, ou arrêtez les tours." : "Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne";
    $("new-chat").disabled = busy;
    ui.menuButton.disabled = busy || !currentConversation();
    deepLinks.setReady(deepLinksBootReady && connected && !busy);
    $("restore-link-draft").hidden = Boolean(selectedId) || previousLinkDrafts().length === 0;
    const learn = $("learn-skill-button");
    learn.hidden = ["task", "routine"].includes(currentConversation()?.kind);
    learn.disabled = busy || !connected || pending || noMembers || Boolean(streamingMessage())
      || !currentConversation()?.messages.some((message) => message.role === "user" && ["accepted", "echoed"].includes(message.state));
    mascot?.setContext({ connection: connected ? "online" : connectionStatus,
      activity: taskFinished || control.offline ? null : submissionActivity, pending: taskFinished || control.offline ? null : pendingMessage?.state || null, drafting: !taskFinished && !attachmentsBlocked && Boolean(ui.input.value.trim()) });
  }

  // Changer d’écran sans ouvrir de fil (un autre bot, l’accueil) ferme aussi la page d’un sujet.
  function notifyNavigation() {
    sidebarSection = "conversations";
    for (const panel of panels.values()) panelCall(panel, panel.onNavigate);
    syncSidebarRail();
  }

  function syncSidebarRail() {
    const active = document.querySelector(".routines-dialog[open]") ? "routines"
      : document.querySelector(".notifications-dialog[open]") ? "notifications"
      : $("settings-dialog").open ? "settings"
      : document.querySelector(".main-panel").dataset.view === "topic" ? "topics" : sidebarSection;
    for (const button of document.querySelectorAll("[data-sidebar-destination]")) {
      const id = button.dataset.sidebarDestination;
      button.setAttribute("aria-pressed", String(id === active));
      if (id === "topics" || id === "routines") button.hidden = !panels.has(id);
    }
  }

  function selectConversation(id, focus = false) {
    navigationGeneration++;
    notifyNavigation();
    approvalFollow.cancel();
    saveDraft();
    selectedId = id;
    writePreference("grokbot.selected", id || "");
    setComposerText(readPreference(draftKey(), ""));
    restoreDraftDocument();
    closeMenu();
    closeSidebar();
    render(true);
    if (focus) ui.input.focus();
  }

  // All staged attachments, including voice memos, belong to this bot/thread.
  function stagedOnEmptyThread() {
    return Boolean(!currentConversation()?.messages.length && $("composer-attachments").querySelector(".attachment-chip, .voice-memo-chip"));
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
      // Un fil rangé dans un sujet vit dans la section SUJETS, pas dans CONVERSATIONS.
      .filter((item) => !(panels.has("topics") && state.topics?.membership?.[item.id]))
      .filter((item) => !favoriteOnly || item.favorite)
      .filter((item) => !query || item.title.toLocaleLowerCase("fr").includes(query) || (item.description || "").toLocaleLowerCase("fr").includes(query) || item.messages.some((message) => window.GrokbotChat.searchableMessageText(message).toLocaleLowerCase("fr").includes(query)))
      .sort((a, b) => timestamp(b.updated_at) - timestamp(a.updated_at));
    if (!items.length) {
      const text = query ? "Aucune conversation trouvée." : favoriteOnly ? "Ajoutez une conversation aux favoris pour la retrouver ici." : "Vos conversations apparaîtront ici.";
      ui.list.append(element("p", "sidebar-empty", text));
      return;
    }
    let previousGroup;
    const today = new Date().toDateString();
    const yesterday = new Date(Date.now() - 86400000).toDateString();
    const weekStart = new Date();
    weekStart.setHours(0, 0, 0, 0);
    weekStart.setDate(weekStart.getDate() - (weekStart.getDay() + 6) % 7);
    for (const conversation of items) {
      const bot = botById(conversation.bot_id);
      const updated = timestamp(conversation.updated_at);
      const date = updated.toDateString();
      const group = date === today ? "Aujourd’hui" : date === yesterday ? "Hier" : updated >= weekStart ? "Cette semaine" : "Précédemment";
      if (group !== previousGroup) { ui.list.append(element("p", "list-group-title", group)); previousGroup = group; }
      const button = element("button", `conversation-item${conversation.id === selectedId ? " active" : ""}`);
      button.type = "button";
      button.dataset.conversationId = conversation.id;
      button.title = conversation.kind === "group"
        ? `${conversation.title}${conversation.description ? ` — ${conversation.description}` : ""} · Groupe : ${groupMembers(conversation).map((b) => b.name).join(", ")}`
        : `${conversation.title} · ${botName(conversation.bot_id)}`;
      if (conversation.id === selectedId) button.setAttribute("aria-current", "page");
      button.append(conversation.kind === "group" ? groupMark(conversation) : botAvatar(bot));
      if (conversation.kind === "group") {
        const copy = element("span", "conversation-copy");
        copy.append(element("span", "conversation-name", conversation.title || "Nouvelle conversation"));
        if (conversation.description) copy.append(element("small", "conversation-description", conversation.description));
        button.append(copy);
      } else button.append(element("span", "conversation-name", conversation.title || "Nouvelle conversation"));
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
  function groupMark(conversation) {
    const mark = element("span", "group-mark");
    if (conversation.avatar?.color) mark.dataset.botColor = conversation.avatar.color;
    mark.textContent = conversation.avatar?.mark || Array.from((conversation.title || "G").trim() || "G")[0] || "G";
    mark.setAttribute("aria-hidden", "true");
    return mark;
  }
  function groupAvatars(conversation) {
    const stack = element("span", "group-avatar-stack");
    stack.setAttribute("aria-hidden", "true");
    const members = groupMembers(conversation);
    for (const bot of members.slice(0, 3)) stack.append(botAvatar(bot));
    if (members.length > 3) stack.append(element("span", "group-avatar-more", `+${members.length - 3}`));
    if (!members.length) stack.append(botAvatar(null));
    return stack;
  }

  // Le choix vaut pour le fil vide affiché ; depuis un autre fil, il ouvre un nouveau fil avec ce bot.
  function chooseBot(bot) {
    if (busy) return;
    if (stagedOnEmptyThread()) {
      notify(`Retirez les pièces jointes pour changer de bot, ou envoyez-les à ${currentBot()?.name || "ce bot"}.`);
      return;
    }
    closeBotMenu();
    chosenBotId = bot.id;
    writePreference("grokbot.bot", bot.id);
    notifyNavigation();
    const conversation = currentConversation();
    if (conversation?.messages.length || conversation?.kind === "group") { selectConversation(null, true); return; }
    // Preserve both drafts when leaving a persisted empty thread from an old version.
    leaveEmptyThread();
    render();
  }

  function revealChosenBot() {
    const list = $("sidebar-bot-list");
    const pressed = list.querySelector("[aria-pressed=true]");
    if (pressed && (pressed.offsetTop < list.scrollTop || pressed.offsetTop + pressed.offsetHeight > list.scrollTop + list.clientHeight)) list.scrollTop = pressed.offsetTop;
  }

  // Un fondu bas signale qu'il reste des bots à faire défiler.
  function fadeBotList() {
    const list = $("sidebar-bot-list");
    list.classList.toggle("has-more", list.scrollTop + list.clientHeight < list.scrollHeight - 1);
  }

  function composerCanFocus() {
    const editor = $("composer-input")?.querySelector(".gb-editor")?.editor;
    return editor?.isEditable !== false;
  }

  function focusComposerOrTitle() {
    if (composerCanFocus()) {
      ui.input?.focus();
      return;
    }
    const title = $("conversation-title");
    if (!title) return;
    title.tabIndex = -1;
    title.focus();
  }

  function closeBotMenu(options = {}) {
    const menu = $("bot-menu");
    const wasOpen = !menu.hidden;
    const hadFocus = menu.contains(document.activeElement);
    // Move focus before hiding, so removing the focused item cannot leave focus on body.
    // A disabled trigger cannot take focus: return it to the editor, or to the title when the editor is inert.
    if (wasOpen && (hadFocus || options.restoreFocus)) {
      if ($("composer-bot").disabled) focusComposerOrTitle();
      else $("composer-bot").focus();
    }
    menu.hidden = true;
    menu.replaceChildren();
    $("composer-bot").setAttribute("aria-expanded", "false");
  }

  function renderBotMenu() {
    const menu = $("bot-menu");
    if (menu.hidden) return;
    // Un rafraîchissement remplace les boutons : garder l'option focalisée et le défilement.
    const focusedKey = menu.contains(document.activeElement) ? document.activeElement.dataset.menuKey : null;
    const scrollTop = menu.querySelector(".bot-menu-list")?.scrollTop || 0;
    const chosen = currentBot();
    const options = liveBots().map((bot) => {
      const button = element("button");
      button.type = "button";
      button.dataset.menuKey = `bot:${bot.id}`;
      button.setAttribute("role", "menuitemradio");
      button.setAttribute("aria-checked", String(bot.id === chosen?.id));
      const copy = element("span", "bot-menu-copy");
      copy.append(element("span", "bot-menu-name", bot.name), element("small", "", bot.description || (bot.is_default ? "Le bot par défaut." : "Bot personnel")));
      button.append(botAvatar(bot), copy, icon("check"));
      button.addEventListener("click", () => chooseBot(bot));
      return button;
    });
    const action = (glyph, label, run) => {
      const button = element("button", "bot-menu-action");
      button.type = "button";
      button.dataset.menuKey = `action:${glyph}`;
      button.setAttribute("role", "menuitem");
      button.append(icon(glyph), element("span", "", label));
      button.addEventListener("click", () => { closeBotMenu(); run(); });
      return button;
    };
    const actions = chosen && !chosen.deleted ? [action("edit", `Modifier ${chosen.name}`, () => openBotDialog(chosen))] : [];
    const list = element("div", "bot-menu-list");
    list.append(...options);
    menu.replaceChildren(element("h4", "", "Avec qui ?"), list, element("div", "menu-divider"), ...actions, action("plus", "Nouveau bot", () => openBotDialog(null)));
    list.scrollTop = scrollTop;
    if (focusedKey) [...menu.querySelectorAll("button")].find((button) => button.dataset.menuKey === focusedKey)?.focus({ preventScroll: true });
    $("composer-bot").setAttribute("aria-expanded", "true");
  }

  function renderBots() {
    const chosen = currentBot();
    const conversation = currentConversation();
    const bots = liveBots().map((bot) => {
      const button = element("button", "sidebar-bot");
      button.type = "button";
      button.dataset.botColor = bot.color;
      button.setAttribute("aria-pressed", String(bot.id === chosen?.id));
      button.title = bot.description || bot.name;
      button.append(botAvatar(bot), element("span", "", bot.name));
      button.addEventListener("click", () => chooseBot(bot));
      return button;
    });
    const list = $("sidebar-bot-list");
    list.replaceChildren(...bots);
    // Ramène le bot choisi dans la vue quand il change, sans contrarier un défilement manuel.
    if (chosen?.id !== revealedBotId) { revealedBotId = chosen?.id; revealChosenBot(); }
    fadeBotList();
    // La barre d'écriture montre le bot du fil ; il ne se change que sur un fil direct encore vide.
    $("composer-bot-name").textContent = chosen?.name || "Ezerbot";
    $("composer-bot").disabled = Boolean(conversation?.messages.length) || ["group", "task", "routine"].includes(conversation?.kind);
    if ($("composer-bot").disabled) closeBotMenu();
    renderBotMenu();
    $("welcome-description").textContent = currentConversation()?.kind === "group"
      ? "Un fil partagé pour croiser vos idées. Sans mention, le coordinateur répond."
      : chosen?.description || defaultWelcome;

    // A state refresh rebuilds the rows; it must not close the menu the user is using.
    const openMenu = $("bot-list").querySelector(".bot-link-menu[open]");
    const focusedTag = openMenu?.contains(document.activeElement) ? document.activeElement.localName : null;
    $("bot-list").replaceChildren(...liveBots().map((bot) => {
      const row = element("div", "bot-row");
      const copy = element("div");
      copy.append(element("strong", "", bot.name), element("small", "", bot.description || (bot.is_default ? "Le bot par défaut." : "Aucune description.")));
      const edit = element("button", "secondary-button", "Modifier");
      edit.type = "button";
      edit.setAttribute("aria-label", `Modifier ${bot.name}`);
      edit.addEventListener("click", () => openBotDialog(bot));
      const menu = element("details", "bot-link-menu");
      const summary = element("summary", "icon-button");
      summary.setAttribute("aria-label", `Actions de ${bot.name}`); summary.append(icon("more"));
      const actions = element("div", "popover");
      const link = element("button", "", "Copier le lien"); link.type = "button";
      link.addEventListener("click", () => { menu.open = false; void copyLink("agent", { id: bot.id }); });
      actions.append(link); menu.append(summary, actions);
      let restoringMenu = bot.id === openMenu?.dataset.botId;
      menu.addEventListener("toggle", () => {
        if (menu.open && !restoringMenu) actions.scrollIntoView({ block: "nearest", inline: "nearest" });
        restoringMenu = false;
      });
      menu.dataset.botId = bot.id; menu.open = restoringMenu;
      row.append(botAvatar(bot), copy, edit, menu);
      return row;
    }));
    if (focusedTag) $("bot-list").querySelector(`.bot-link-menu[open] ${focusedTag}`)?.focus({ preventScroll: true });
  }

  function openBotDialog(bot) {
    navigationGeneration++;
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
    $("bot-copy-link").hidden = !bot;
    renderBotExtensions(bot);
    botDialogInitial = JSON.stringify(botFormValues());
    $("bot-dialog").showModal();
    $("bot-name").focus();
  }

  function openSkillsPanel() {
    const candidate = currentBot();
    const bot = candidate && !candidate.deleted ? candidate : chosenBot();
    if (!bot || bot.deleted) return;
    if (!$("bot-dialog").open || editingBotId !== bot.id) openBotDialog(bot);
    document.querySelector('[data-panel="skills"] h3')?.focus();
    panels.get("skills")?.noteShortcut?.(bot.id);
  }

  function botFormValues() {
    return {
      name: $("bot-name").value.trim(),
      color: $("bot-colors").querySelector("[aria-pressed=true]")?.dataset.botColor || "citron",
      shape: $("bot-shapes").querySelector("[aria-pressed=true]")?.dataset.shape || "galet",
      description: $("bot-description").value.trim(),
      instructions: $("bot-instructions").value.trim(),
    };
  }

  function selectBotColor(color) {
    $("bot-colors").querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.botColor === color)));
    $("bot-shapes").dataset.botColor = color;
    renderBotPreview();
  }

  function selectBotShape(shape) {
    $("bot-shapes").querySelectorAll("button").forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.shape === shape)));
    renderBotPreview();
  }

  // L'aperçu en tête du formulaire suit chaque frappe, forme et couleur.
  function renderBotPreview() {
    const { name, color, shape, description } = botFormValues();
    $("bot-preview").dataset.botColor = color;
    const figure = $("bot-preview-mascot").firstElementChild;
    if (figure) mascot.setShape(figure, shape);
    $("bot-preview-name").textContent = name || "Votre nouveau bot";
    $("bot-preview-name").classList.toggle("is-empty", !name);
    $("bot-preview-description").textContent = description || "Sa présentation apparaîtra ici.";
  }

  function upsertBot(bot) {
    const index = state.bots.findIndex((item) => item.id === bot.id);
    if (index === -1) state.bots.push(bot);
    else state.bots[index] = bot;
  }

  async function saveBot(event) {
    event.preventDefault();
    const values = botFormValues();
    if (!values.name) { notify("Donnez un nom à votre bot."); $("bot-name").focus(); return; }
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
        ? `${bot.name} est prêt. Retirez les pièces jointes pour le choisir, ou envoyez-les à ${currentBot()?.name || "ce bot"}.`
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
    onReact: (message, emoji) => reactToMessage(message, emoji),
    onThread: (message) => { void openThread(message); },
    onResend: (message) => resendMessage(message),
    onDelete: (message) => deleteFailedMessage(message),
    onRegenerate: (message) => message.error_kind === "local_connection" ? refresh().catch((error) => notify(error.message)) : generation("/api/regenerate", message.id.replace(/^reply-/, "")),
    onSignIn: () => { void signIn(); },
    onCardAction: cardAction,
    onOpenPage: (id) => { const pages = panels.get("pages"); if (pages) panelCall(pages, pages.openPage, id); },
    get usageUrl() { return usageUrl(); },
    // « Ouvrir la source » : un lien Google dans le navigateur, un fichier de la bibliothèque dans la fiche du bot.
    async onOpenSource(source, message) {
      const opening = window.GrokbotChat.sourceOpening(source, { connectors: state.mcp?.catalogue });
      if (opening.problem) return { ok: false, message: opening.problem };
      if (opening.url) {
        // Un vrai clic sur un lien : la coque native ouvre alors l'adresse dans le navigateur.
        const link = element("a");
        link.href = opening.url; link.target = "_blank"; link.rel = "noopener noreferrer";
        document.body.append(link); link.click(); link.remove();
        return { ok: true };
      }
      const bot = botById(message?.payload?.bot_id) || currentBot();
      if (!bot || bot.deleted) return { ok: false, message: "Ce bot n’existe plus, sa bibliothèque non plus." };
      const library = await api(`/api/library?bot_id=${encodeURIComponent(bot.id)}`);
      if (!(library.files || []).some((file) => file.id === opening.library)) return { ok: false, message: "Ce fichier n’est plus dans la bibliothèque." };
      openBotDialog(bot);
      return { ok: true };
    },
    // « N souvenirs utilisés » : l'écran Mémoire s'ouvre sur ces souvenirs, avec de quoi revenir à la réponse.
    onOpenMemory(ids, message) {
      const space = window.EzerbotMemorySpace;
      if (!space) return { ok: false, message: "L’écran Mémoire n’est pas disponible ici." };
      return space.openWith(ids, { conversationId: currentConversation()?.id, messageId: message?.id });
    },
    async onCopy(text) {
      try { await copyText(text); return true; }
      catch { notify("La copie n’est pas disponible. Sélectionnez le texte pour le copier."); return false; }
    },
  };

  function render(followLatest = false) {
    const conversation = currentConversation();
    // Keep an explicit request until React's layout effect runs, even if a
    // subsequent busy/status render replaces this render before it commits.
    if (followLatest && conversation) followAfterLayout = conversation.id;
    const generation = scrollGeneration;
    const follow = (conversation?.kind === "group" || conversation?.messages.some((message) => message.kind === "memory_auto")) && ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90;
    const onLayout = () => {
      if (currentConversation()?.id !== conversation?.id || generation !== scrollGeneration) return;
      if (ui.findOpen) return;
      const requested = Boolean(conversation && followAfterLayout === conversation.id);
      if (!scrollToPendingApproval() && (requested || follow)) ui.scroll.scrollTop = ui.scroll.scrollHeight;
      if (requested) followAfterLayout = null;
    };
    const hasMessages = Boolean(conversation?.messages.length);
    ui.findOpen = findState.open;
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
    ui.messages.hidden = !hasMessages;
    const visibleMessages = window.GrokbotChat.connectionMessages(conversation?.messages || [], connected);
    const query = findState.query.trim();
    const searching = findState.open && Boolean(query);
    // Les correspondances d’un autre fil ou d’une recherche fermée ne survivent pas ; le fil validé les recalcule.
    if (!searching || findState.conversationId !== (conversation?.id || null)) findState.matches = [];
    findState.conversationId = conversation?.id || null;
    window.GrokbotChat.renderThread(ui.messages, { messages: visibleMessages, busy: busy && connected, engineConnected: connected, providerConnected: Boolean(provider().connected), botName: bot?.name || "Ezerbot", bots: state.bots, createBotMascot, isGroup: conversation?.kind === "group", isTask: conversation?.kind === "task", onLayout, find: searching ? { query, index: findState.index } : null, onFindMatches, onRateGood: rateGood, onRateBad: rateBad, memoSpeed: state.voice?.bots?.[conversation?.bot_id]?.speed, ...thread });
    paintFind();
    renderThreadBanner(conversation);
    $("favorite-button").querySelector("span").textContent = conversation?.favorite ? "Retirer des favoris" : "Ajouter aux favoris";
    const topicEntry = $("topic-button");
    if (topicEntry) topicEntry.hidden = !panels.has("topics") || !["chat", "group"].includes(conversation?.kind);
    renderList();
    renderBots();
    renderProvider();
    for (const panel of panels.values()) panelCall(panel, panel.onState, state);
    updateComposer();
    syncSidebarRail();
  }

  function pill(kind, label, value, enabled = true) {
    const button = element("button", "tool-pill");
    button.type = "button";
    button.dataset.picker = kind;
    button.title = `${label} : ${value}`;
    button.setAttribute("aria-label", button.title);
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
    if (!pickerKind) { picker.replaceChildren(); return; }
    const { models = [] } = provider();
    const { model, effort, fast } = state.preferences || {};
    const selected = models.find((item) => item.slug === model);
    const title = { model: "Modèle", effort: "Effort de réflexion", speed: "Vitesse" }[pickerKind];
    const options = pickerKind === "model" ? models.map((item) => option(item.name, item.description, item.slug === model, { model: item.slug }))
      : pickerKind === "effort" ? (selected?.efforts || []).map((level) => option(level.effort, level.description, level.effort === effort, { effort: level.effort }))
      : [option("Standard", "", !fast, { fast: false }), ...(selected?.tiers || []).slice(0, 1).map((tier) => option(tier.name, tier.description, Boolean(fast), { fast: true }))];
    const foot = element("div", "picker-foot");
    const link = element("a", "usage-link", "Gérer l’utilisation");
    link.href = usageUrl(); link.target = "_blank"; link.rel = "noreferrer";
    const scrollHint = element("small", "picker-scroll-hint", "Liste défilante ↕");
    scrollHint.hidden = true;
    foot.append(scrollHint, element("span", "", "Quota de votre abonnement ChatGPT"), link);
    if (picker.dataset.kind !== pickerKind) picker.scrollTop = 0;
    picker.dataset.kind = pickerKind;
    picker.replaceChildren(element("h4", "", title), ...options, foot);
    scrollHint.hidden = !(picker.scrollHeight > picker.clientHeight);
  }

  function renderProvider() {
    const current = provider();
    const { model, effort, fast } = state.preferences || {};
    const selected = (current.models || []).find((item) => item.slug === model);
    const probe = current.connection_test || {};
    const signedIn = Boolean(current.signed_in);
    $("plan-banner").hidden = signedIn;
    $("plan-banner").querySelector("span").textContent = "Connectez votre compte ChatGPT. Un seul abonnement, aucune clé API.";
    document.querySelectorAll("[data-provider-signin]").forEach((button) => {
      if (button.closest?.(".gb-generated-image-card")) return;
      button.disabled = current.signin === "pending";
      button.textContent = "Se connecter";
    });

    const tools = $("composer-tools");
    if (current.connected) {
      tools.replaceChildren(
        pill("model", "Modèle", selected?.name || (current.models_ready ? "Aucun modèle" : "Chargement…"), Boolean(current.models?.length)),
        pill("effort", "Effort", selected?.efforts.length ? (effort || selected.default_effort || "Par défaut") : "Par défaut", Boolean(selected?.efforts.length)),
        // Speed is offered only by models that list a service tier.
        ...(selected?.tiers.length ? [pill("speed", "Vitesse", fast ? selected.tiers[0].name : "Standard")] : []),
      );
      if (pickerKind === "speed" && !selected?.tiers.length) pickerKind = null;
    } else {
      closePicker();
      // Sans compte, rien à signaler : l'app tourne en local sans le dire partout.
      tools.replaceChildren(...(signedIn ? [element("span", "composer-local", probe.state === "running" ? "Vérification" : "Non connecté")] : []));
    }
    renderPicker();

    const note = $("mode-note");
    if (current.connected) {
      const link = element("a", "", "Gérer l’utilisation");
      link.href = usageUrl(); link.target = "_blank"; link.rel = "noreferrer";
      note.replaceChildren(document.createTextNode("Quota de votre abonnement ChatGPT · "), link);
    } else if (signedIn) {
      note.replaceChildren(document.createTextNode(probe.state === "running"
        ? "Vérification de la connexion en cours. Votre question part sans attendre."
        : "La vérification a échoué. Vous pouvez envoyer, ou réessayer le test."));
    } else {
      note.replaceChildren(element("span", "simulation-dot"), document.createTextNode(current.simulation ? "Mode développeur · réponses simulées" : "Connexion ChatGPT requise"));
    }

    const card = element("div", "provider-card");
    const copy = element("div");
    const actions = element("div", "provider-actions");
    const heading = element("h3", "", current.connected ? "Connecté à ChatGPT"
      : signedIn ? (probe.state === "running" ? "Vérification de la connexion OpenAI" : "OpenAI n’a pas confirmé la connexion")
      : "Utiliser votre compte ChatGPT");
    if (current.connected) heading.prepend(element("span", "status-dot"));
    copy.append(heading, element("p", "", current.connected ? (current.email ? `Compte ChatGPT : ${current.email}` : "Adresse e-mail du compte ChatGPT indisponible.")
      : signedIn ? (probe.state === "running"
        ? "La vérification se fait en arrière-plan. L’envoi n’est pas bloqué : votre question part tout de suite."
        : "OpenAI n’a pas confirmé la vérification. Envoyez votre question, ou réessayez le test.")
      : current.signin === "pending" ? "Terminez la connexion dans votre navigateur. Cette fenêtre se met à jour toute seule."
      : "Utilisez votre abonnement ChatGPT, sans clé API ni autre abonnement. OpenAI autorise Ezerbot dans votre navigateur."));
    const planNames = { plus: "Plus", pro: "Pro", free: "Gratuit", team: "Équipe", business: "Business", enterprise: "Entreprise", edu: "Éducation" };
    if (current.connected) copy.append(element("p", "", `Forfait : ${planNames[current.plan] || current.plan || "non communiqué par OpenAI"}`));
    if (signedIn) copy.append(element("p", "", "Cette connexion est propre à Ezerbot ; elle utilise le quota Codex de ce même abonnement. La déconnexion l’efface sur ce Mac uniquement."));
    if (current.signin === "failed" && current.error) copy.append(element("p", "provider-error", current.error));
    const action = (label, className, run) => { const button = element("button", className, label); button.type = "button"; button.addEventListener("click", run); return button; };
    const test = action(probe.state === "running" ? "Test en cours…" : "Tester la connexion", "secondary-button", () => providerAction("/api/provider/test"));
    test.disabled = !signedIn || probe.state === "running";
    test.title = !signedIn ? "Connectez votre compte ChatGPT pour tester la connexion." : "Envoyer une petite requête de vérification à OpenAI";
    if (current.connected) {
      actions.append(test);
      const link = element("a", "secondary-button", "Gérer l’utilisation");
      link.href = usageUrl(); link.target = "_blank"; link.rel = "noreferrer";
      actions.append(link, action("Se déconnecter", "secondary-button", signOut));
    } else if (signedIn) {
      actions.append(test, action("Se déconnecter", "secondary-button", signOut));
    } else if (current.retained) {
      copy.append(element("p", "provider-error", "Connexion suspendue sur ce Mac. Des identifiants restent sur le disque : réessayez de les effacer avant de fermer Ezerbot."));
      actions.append(action("Réessayer la déconnexion", "secondary-button", signOut));
    } else if (current.signin === "pending") {
      actions.append(action("Annuler", "secondary-button", () => providerAction("/api/provider/cancel")));
    } else {
      actions.append(action("Se connecter", "chatgpt-button", signIn));
    }
    if (!signedIn) actions.append(test);
    if (probe.message && probe.state !== "running" && signedIn) {
      const result = element("p", probe.state === "failed" ? "provider-error" : probe.state === "ok" ? "provider-success" : "", probe.message);
      if (probe.state === "ok") result.prepend(element("span", "status-dot"));
      result.setAttribute("role", "status"); copy.append(result);
    }
    card.append(copy, actions);
    $("provider-section").replaceChildren(card);

    const dialog = $("plan-dialog");
    dialog.querySelector("a").href = usageUrl();
    $("plan-title").textContent = "Vous utilisez votre abonnement ChatGPT";
    dialog.querySelector(".delete-description").textContent = "Aucun abonnement Ezerbot supplémentaire. Les réponses utilisent le quota Codex de votre abonnement ChatGPT, partagé avec les autres applications qui utilisent ce quota. OpenAI fixe les limites ; les jauges affichées viennent directement d’OpenAI.";
    if (current.connected && !current.welcomed && !dialog.open) dialog.showModal();
  }

  async function providerAction(path, body = {}) {
    try {
      const response = await api(path, body);
      state.provider = response.provider;
      renderProvider();
      schedulePoll();
      return response;
    } catch (error) { notify(error.message); return null; }
  }

  async function signIn() {
    const response = await providerAction("/api/provider/signin");
    // Debug fake OAuth only; production opens the system browser in Rust.
    if (response?.authorization_url?.startsWith("http://127.0.0.1:")) window.open(response.authorization_url, "_blank", "noopener");
    return response;
  }

  async function signOut() {
    const response = await providerAction("/api/provider/signout");
    if (response && !response.revoked) notify("Déconnecté sur ce Mac. OpenAI n’a pas confirmé : vous pouvez retirer Ezerbot dans les réglages de ChatGPT.");
    await refresh().catch(() => {});
  }

  async function generation(path, messageId) {
    const conversation = currentConversation();
    if (!conversation) return;
    const pinned = ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90;
    try {
      if (path === "/api/stop") await window.EzerbotVoiceMode?.stopSpeech();
      const response = await api(path, { conversation_id: conversation.id, message_id: messageId });
      upsertConversation(response.conversation);
      render(currentConversation()?.id === conversation.id && pinned && ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90);
      schedulePoll();
    } catch (error) { notify(error.message); }
  }

  function scrollToPendingApproval() {
    const conversation = currentConversation();
    const id = approvalFollow.observe(conversation?.id, conversation?.messages || []);
    if (!id) return false;
    const card = [...ui.messages.querySelectorAll('[data-card-kind="approval"]')].find((node) => node.dataset.messageId === id);
    if (card) window.GrokbotChat.revealPendingApproval(card, ui.scroll);
    return true; // React may not have committed the card yet; onLayout retries.
  }

  function scrollToLatest() {
    render(true); // onLayout runs after the thread/cards actually exist.
  }

  function cancelScrollFollow() {
    followAfterLayout = null;
    scrollGeneration += 1;
    approvalFollow.cancel();
  }

  function observeViewportResize() {
    const scroll = ui.scroll;
    const pinned = () => scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 90;
    const viewport = { width: scroll.clientWidth, height: scroll.clientHeight, pinned: pinned() };
    scroll.addEventListener("scroll", () => {
      // A resize may dispatch scroll before ResizeObserver. Keep the position
      // from the old viewport until the observer has consumed that resize.
      if (scroll.clientWidth === viewport.width && scroll.clientHeight === viewport.height) viewport.pinned = pinned();
    }, { passive: true });
    new ResizeObserver(() => {
      const changed = scroll.clientWidth !== viewport.width || scroll.clientHeight !== viewport.height;
      const follow = viewport.pinned;
      viewport.width = scroll.clientWidth; viewport.height = scroll.clientHeight; viewport.pinned = pinned();
      if (changed && follow) scrollToLatest();
    }).observe(scroll);
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

  async function learnConversation() {
    if ($("learn-skill-button").disabled) return;
    const conversation = currentConversation();
    if (!conversation) return;
    closeMenu();
    approvalFollow.arm(conversation.id, conversation.messages, ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90);
    busy = true; submissionActivity = "send"; updateComposer();
    const messageId = crypto.randomUUID();
    // Use the ordinary durable send/context path. No separate summarizer, no
    // direct skill mutation, and no consumption of the user's unsent draft/files.
    const body = window.GrokbotChat.teachPrompt;
    try {
      const response = await api("/api/send", { conversation_id: conversation.id, message_id: messageId, body });
      upsertConversation(response.conversation); render(true); schedulePoll();
    } catch (error) {
      try { await refresh(); } catch { showConnectionError(error); }
      notify("Demande d’apprentissage non confirmée. Vérifiez le fil avant de réessayer.");
    } finally { busy = false; submissionActivity = null; updateComposer(); schedulePoll(); }
  }

  async function sendMessage(event) {
    event.preventDefault();
    const live = streamingMessage();
    if (live && ui.send.dataset.mode === "stop") { await generation("/api/stop", currentConversation()?.kind === "group" ? "groupe" : live.id.replace(/^reply-/, "")); return; }
    if (ui.send.disabled) return;
    const submittedScrollGeneration = scrollGeneration;
    const submittedDraft = ui.input.value;
    const submittedDocument = ui.input.document;
    const body = submittedDraft.trim();
    const extras = structuredClone(sendExtras());
    const captured = captureComposer();
    let conversation = currentConversation();
    approvalFollow.arm(conversation?.id ?? null, conversation?.messages || [], ui.scroll.scrollHeight - ui.scroll.scrollTop - ui.scroll.clientHeight < 90);
    busy = true;
    submissionActivity = "send";
    updateComposer();
    let messageId;
    let delivered = false;
    const previousDraftKey = draftKey();
    const restoreDraft = async () => {
      const remainder = ui.input.value;
      const remainderDocument = ui.input.document;
      setComposerText(submittedDraft && remainder ? `${submittedDraft}\n${remainder}` : submittedDraft || remainder);
      ui.input.document = submittedDraft && remainder
        ? { type: "doc", content: [...submittedDocument.content, ...remainderDocument.content] }
        : submittedDraft ? submittedDocument : remainderDocument;
      saveDraft();
      for (const panel of panels.values()) {
        try {
          const warning = await panel.composer?.restore?.({ messageId, extras, captured: captured.get(panel.id) }, panelContext());
          if (warning) notify(warning);
        } catch {
          notify("La citation n’a pas pu être restaurée. Vérifiez-la avant de réessayer.");
        }
      }
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
        approvalFollow.bind(conversation.id);
      }
      messageId = crypto.randomUUID();
      pendingSends.set(messageId, { extras, captured, consumed: false });
      conversation.messages.push({ ...extras, ...(conversation.kind === "task" ? { payload: { source: "follow_up" } } : {}), id: messageId, role: "user", body, state: "queued", created_at: new Date().toISOString() });
      removePreference(previousDraftKey);
      removePreference(`${previousDraftKey}.document`);
      saveDraft(); // Keep any text typed while conversation creation was in flight.
      render(scrollGeneration === submittedScrollGeneration);
      const response = await sendRequest(conversation, messageId, body, extras, simulateLoss);
      simulateLoss = false;
      $("simulate-loss").checked = false;
      upsertConversation(response.conversation);
      delivered = response.conversation.messages.some((message) => message.id === messageId && ["accepted", "echoed"].includes(message.state));
      render(scrollGeneration === submittedScrollGeneration);
      schedulePoll();
    } catch (error) {
      if (messageId) {
        try {
          await refresh();
          const savedMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          notify(error.message);
          if (!savedMessage) { pendingSends.delete(messageId); await restoreDraft(); }
        } catch {
          const optimisticMessage = currentConversation()?.messages.find((message) => message.id === messageId);
          const confirmed = optimisticMessage && ["accepted", "echoed"].includes(optimisticMessage.state);
          if (!confirmed) {
            if (error.accessDisabled) {
              const owner = state.conversations.find((item) => item.messages.some((message) => message.id === messageId));
              if (owner) owner.messages = owner.messages.filter((message) => message.id !== messageId);
              pendingSends.delete(messageId);
            } else if (optimisticMessage) optimisticMessage.state = "unknown";
            await restoreDraft();
          }
          render();
          showConnectionError(error);
        }
      } else { notify(error.message); await restoreDraft(); }
    } finally {
      busy = false;
      submissionActivity = null;
      render();
      if (delivered) mascot?.celebrate();
      schedulePoll();
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

  function attachmentIds(message, extras = {}) {
    return extras.attachments || (message.attachments || []).map((item) => typeof item === "string" ? item : item.id || item.attachment_id).filter(Boolean);
  }

  // Après un rechargement, pendingSends est vide : l'ancre enregistrée avec le message reste celle du premier envoi, que le moteur exige à l'identique.
  function resendExtras(message) {
    const pending = pendingSends.get(message.id)?.extras;
    if (pending) return pending;
    const anchor = message.payload?.page_anchor;
    const attachments = attachmentIds(message);
    return {
      ...(attachments.length ? { attachments } : {}),
      ...(anchor ? { page_anchor: { page_id: anchor.page_id, source_version: anchor.source_version, passage: anchor.passage } } : {}),
    };
  }

  async function reactToMessage(message, emoji) {
    const conversation = currentConversation();
    if (!conversation) return;
    const target = conversation.messages.find((item) => item.id === message.id);
    const previous = target?.payload?.reaction ?? null;
    if (target) {
      const next = previous === emoji ? null : emoji;
      target.payload = { ...(target.payload && typeof target.payload === "object" ? target.payload : {}) };
      if (next) target.payload.reaction = next;
      else delete target.payload.reaction;
    }
    render();
    try {
      const response = await api("/api/messages/react", { conversation_id: conversation.id, message_id: message.id, emoji });
      upsertConversation(response.conversation);
      render();
    } catch (error) {
      if (target && currentConversation()?.id === conversation.id) {
        const current = currentConversation().messages.find((item) => item.id === message.id);
        if (current) {
          current.payload = { ...(current.payload && typeof current.payload === "object" ? current.payload : {}) };
          if (previous) current.payload.reaction = previous;
          else delete current.payload.reaction;
        }
      }
      render();
      notify(error.message);
    }
  }

  // « Répondre en fil » ouvre (ou rouvre) le sous-fil lié à ce message ; le fil principal ne change pas.
  async function openThread(message) {
    const conversation = currentConversation();
    if (!conversation || busy) return;
    try {
      const response = await api("/api/messages/thread", { conversation_id: conversation.id, message_id: message.id });
      upsertConversation(response.conversation);
      state.threads = { ...(state.threads || {}), [response.conversation.id]: response.thread };
      selectConversation(response.conversation.id, true);
    } catch (error) { notify(error.message); }
  }

  function renderThreadBanner(conversation) {
    const banner = $("thread-banner");
    if (!banner) return;
    const link = conversation ? state.threads?.[conversation.id] : null;
    banner.hidden = !link;
    if (!link) return;
    const parent = state.conversations.find((item) => item.id === link.parent_conversation_id);
    const author = link.parent_role === "user" ? "Vous" : botName(parent?.bot_id || conversation.bot_id);
    $("thread-banner-quote").textContent = `${author} : « ${link.parent_excerpt} »`;
    $("thread-banner-open").disabled = busy || !parent?.messages.some((item) => item.id === link.parent_message_id);
  }

  // Revient au fil principal, sur le message qui a ouvert ce sous-fil.
  function showThreadParent() {
    const link = state.threads?.[selectedId];
    if (link) window.GrokbotPanels.openConversation(link.parent_conversation_id, link.parent_message_id);
  }

  async function resendMessage(message) {
    const conversation = currentConversation();
    if (!conversation || busy) return;
    busy = true;
    submissionActivity = "send";
    let delivered = false;
    render();
    try {
      let response;
      try {
        response = await api("/api/messages/resend", { conversation_id: conversation.id, message_id: message.id });
      } catch (error) {
        if (error.status !== 404) throw error;
        const extras = resendExtras(message);
        response = await sendRequest(conversation, message.id, message.body, { ...extras, attachments: attachmentIds(message, extras) });
      }
      upsertConversation(response.conversation);
      delivered = response.conversation.messages.some((item) => item.id === message.id && ["accepted", "echoed"].includes(item.state));
      render();
      scrollToLatest();
    } catch (error) { notify(error.message); }
    finally { busy = false; submissionActivity = null; render(); if (delivered) mascot?.celebrate(); }
  }

  async function deleteFailedMessage(message) {
    const conversation = currentConversation();
    if (!conversation) return;
    const snapshot = conversation.messages;
    conversation.messages = snapshot.filter((item) => item.id !== message.id);
    render();
    try {
      const response = await api("/api/messages/delete", { conversation_id: conversation.id, message_id: message.id });
      upsertConversation(response.conversation);
      render();
    } catch (error) {
      if (currentConversation()?.id === conversation.id) currentConversation().messages = snapshot;
      render();
      notify(error.message);
    }
  }

  async function resumeQueued(message) {
    const conversation = currentConversation();
    if (busy || !conversation) return;
    busy = true;
    submissionActivity = "send";
    let delivered = false;
    render();
    try {
      const response = await sendRequest(conversation, message.id, message.body, resendExtras(message));
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
    const dark = document.documentElement.dataset.theme === "dark";
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#14161a" : "#ffffff");
    document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]')?.setAttribute("content", dark ? "black-translucent" : "default");
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

  function downloadJson(value, filename) {
    const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = element("a");
    anchor.href = url;
    anchor.download = filename;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function exportConversation() {
    await ratingsReady;
    const conversation = currentConversation();
    if (!conversation) return;
    const notations = ratings.list().filter((row) => row.conversation_id === conversation.id);
    const exported = { ...conversation, messages: conversation.messages.map(window.GrokbotChat.exportMessage), notations };
    downloadJson(exported, `ezerbot-${conversation.id}.json`);
    closeMenu();
  }

  async function exportRatings() {
    await ratingsReady;
    downloadJson(ratings.exportDocument(), "ezerbot-notations.json");
    closeMenu();
  }

  // Appelé par le fil à chaque validation React : le compteur suit le DOM affiché.
  function onFindMatches(matches) {
    findState.matches = matches;
    if (findState.index >= matches.length) findState.index = 0;
    paintFind();
  }

  function paintFind() {
    const bar = $("find-bar");
    if (!bar) return;
    const matches = findState.matches;
    bar.hidden = !findState.open;
    const count = $("find-count");
    const label = findState.open && findState.query.trim() && window.GrokbotChat.findCountLabel
      ? window.GrokbotChat.findCountLabel(matches.length, matches.length ? findState.index : 0)
      : "";
    if (count) {
      count.textContent = label;
      if (label) count.setAttribute("aria-label", label);
      else count.removeAttribute("aria-label");
    }
    if ($("find-next")) $("find-next").disabled = matches.length === 0;
    if ($("find-previous")) $("find-previous").disabled = matches.length === 0;
  }

  function openFind() {
    if (!$("find-bar") || !window.GrokbotChat.findInRendered) return;
    findState.open = true;
    render();
    $("find-input")?.focus();
    $("find-input")?.select();
  }

  function closeFind() {
    if (!findState.open) return false;
    findState.open = false;
    findState.query = "";
    findState.index = 0;
    if ($("find-input")) $("find-input").value = "";
    render();
    $("chat-scroll")?.focus();
    return true;
  }

  function stepFind(direction) {
    const matches = findState.matches;
    if (!window.GrokbotChat.cycleFind || !matches.length) return;
    findState.index = window.GrokbotChat.cycleFind(findState.index, matches.length, direction);
    render();
  }

  async function rateGood(message) {
    const conversation = currentConversation();
    if (!conversation || !message?.id) return;
    await ratingsReady;
    try { await ratings.recordGood({ conversationId: conversation.id, messageId: message.id, runId: message.payload?.run_id || null }); }
    catch { notify("Impossible d’enregistrer la notation."); return; }
    notify("Bon résultat enregistré.");
  }

  function rateBad(message) {
    const conversation = currentConversation();
    const dialog = $("rating-dialog");
    if (!conversation || !message?.id || !dialog?.showModal) return;
    pendingRating = { conversationId: conversation.id, messageId: message.id, runId: message.payload?.run_id || null };
    $("rating-form")?.reset?.();
    dialog.showModal();
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
  // Cancel only for explicit reading/navigation gestures, not for scroll events
  // caused by streaming, layout changes or our own scrollToLatest callbacks.
  for (const event of ["wheel", "touchmove"]) ui.scroll.addEventListener(event, cancelScrollFollow, { passive: true });
  ui.scroll.addEventListener("pointerdown", (event) => { if (event.target === ui.scroll) cancelScrollFollow(); });
  ui.scroll.addEventListener("keydown", (event) => {
    if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)
        && !event.target.closest('input, textarea, select, button, summary, [contenteditable="true"]')) cancelScrollFollow();
  });
  observeViewportResize();
  $("composer-form").addEventListener("submit", sendMessage);
  $("learn-skill-button").addEventListener("click", () => void learnConversation());
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
    for (const menu of document.querySelectorAll(".bot-link-menu[open]")) if (!menu.contains(event.target)) menu.open = false;
    const opener = event.target.closest("[data-picker]");
    if (opener) { pickerKind = pickerKind === opener.dataset.picker ? null : opener.dataset.picker; renderProvider(); }
    else if (pickerKind && !$("picker").contains(event.target)) { closePicker(); renderProvider(); }
    if (!$("bot-menu").hidden && !$("bot-menu").contains(event.target) && !$("composer-bot").contains(event.target)) closeBotMenu();
    if (event.target.closest("[data-provider-signin]")) signIn();
  });
  window.addEventListener("resize", () => { if (pickerKind) renderPicker(); });
  $("plan-acknowledge").addEventListener("click", async () => { $("plan-dialog").close(); await providerAction("/api/provider/welcomed"); });
  $("sidebar-toggle").addEventListener("click", () => {
    const open = document.body.classList.toggle("sidebar-open");
    $("sidebar-toggle").setAttribute("aria-expanded", String(open));
    if (open && !window.EzerbotIPhone?.remote) ui.search.focus();
    else if (open) $("sidebar-close").focus({ preventScroll: true });
  });
  $("sidebar-close").addEventListener("click", closeSidebar);
  $("sidebar-scrim").addEventListener("click", closeSidebar);
  $("settings-button").addEventListener("click", () => { closeSidebar(); $("settings-dialog").showModal(); });
  const sidebarActions = {
    conversations() {
      if (busy) return;
      notifyNavigation();
      render();
      ui.list.scrollIntoView({ block: "start" });
      ui.list.focus({ preventScroll: true });
    },
    topics() {
      if (busy || !panels.has("topics")) return;
      notifyNavigation();
      render();
      sidebarSection = "topics";
      $("sidebar-topics").scrollIntoView({ block: "start" });
      $("new-topic").focus({ preventScroll: true });
    },
    routines() {
      const panel = panels.get("routines");
      if (panel) panelCall(panel, panel.openSidebar);
    },
    notifications() {
      const panel = panels.get("notifications");
      if (panel) panelCall(panel, panel.openSidebar);
    },
  };
  for (const button of document.querySelectorAll("[data-sidebar-destination]")) {
    button.addEventListener("click", () => {
      sidebarActions[button.dataset.sidebarDestination]?.();
      syncSidebarRail();
    });
  }
  document.addEventListener("click", syncSidebarRail);
  document.addEventListener("toggle", syncSidebarRail, true);
  document.addEventListener("close", syncSidebarRail, true);
  if (typeof MutationObserver !== "undefined") {
    new MutationObserver(syncSidebarRail).observe(document.querySelector(".main-panel"), { attributes: true, attributeFilter: ["data-view"] });
  }
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
  function selectRenameColor(accent) {
    renameAvatarColor = accents.includes(accent) ? accent : "citron";
    document.querySelectorAll("#rename-avatar-colors button").forEach((button) => {
      button.setAttribute("aria-pressed", String(button.dataset.avatarColor === renameAvatarColor));
    });
  }
  function ensureRenameColors(selected) {
    const box = $("rename-avatar-colors");
    if (!box) return;
    if (!box.querySelector("button")) {
      const colors = [
        ["citron", "Jaune citron", "#E6FF3D"],
        ["orange", "Orange vif", "#FF5C1F"],
        ["ambre", "Ambre", "#FFB224"],
        ["menthe", "Vert menthe", "#35E0A1"],
        ["cyan", "Cyan", "#22D3EE"],
        ["magenta", "Rose magenta", "#FF3D8B"],
        ["vermillon", "Rouge vermillon", "#FF4530"],
        ["violet", "Violet", "#9B7BFF"],
        ["blanc", "Sans couleur", "#F5F5F5"],
      ];
      box.replaceChildren(...colors.map(([accent, label, swatch]) => {
        const button = element("button");
        button.type = "button";
        button.dataset.avatarColor = accent;
        button.dataset.botColor = accent;
        button.setAttribute("aria-label", label);
        button.setAttribute("aria-pressed", "false");
        const dot = element("i");
        dot.style.setProperty("--swatch", swatch);
        dot.setAttribute("aria-hidden", "true");
        button.append(dot, element("span", "", label));
        button.addEventListener("click", () => selectRenameColor(accent));
        return button;
      }));
    }
    selectRenameColor(selected);
  }
  $("rename-button").addEventListener("click", () => {
    closeMenu();
    const conversation = currentConversation();
    const group = conversation?.kind === "group";
    $("rename-input").value = conversation?.title || "";
    if ($("rename-label")) $("rename-label").textContent = group ? "Nom" : "Nom de la conversation";
    if ($("rename-title")) $("rename-title").textContent = group ? "Renommer le groupe" : "Renommer la conversation";
    for (const node of document.querySelectorAll(".group-rename")) node.hidden = !group;
    if (group && $("rename-description")) {
      $("rename-description").value = conversation.description || "";
      $("rename-mark").value = conversation.avatar?.mark || "";
      ensureRenameColors(conversation.avatar?.color || "citron");
    }
    $("rename-dialog").showModal();
    $("rename-input").select();
  });
  $("rename-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const title = $("rename-input").value.trim();
    if (!title) { $("rename-input").focus(); return; }
    const changes = { title };
    const conversation = currentConversation();
    if (conversation?.kind === "group" && $("rename-description")) {
      const description = $("rename-description").value.trim();
      if (!description) { $("rename-description").focus(); return; }
      changes.description = description;
      const mark = ($("rename-mark").value || "").trim() || Array.from(title)[0] || "G";
      changes.avatar = { color: renameAvatarColor, mark };
    }
    if (await updateConversation(changes)) $("rename-dialog").close();
  });
  $("favorite-button").addEventListener("click", async () => {
    closeMenu();
    const conversation = currentConversation();
    if (conversation) await updateConversation({ favorite: !conversation.favorite });
  });
  $("topic-button")?.addEventListener("click", () => {
    closeMenu();
    const conversation = currentConversation(), topics = panels.get("topics");
    if (conversation && topics) panelCall(topics, topics.assign, conversation);
  });
  $("copy-link-button").addEventListener("click", () => { const conversation = currentConversation(); if (conversation) void copyLink("thread", { id: conversation.id }); });
  $("bot-copy-link").addEventListener("click", () => { if (editingBotId) void copyLink("agent", { id: editingBotId }); });
  $("restore-link-draft").addEventListener("click", () => {
    try {
      const previous = previousLinkDrafts().pop();
      if (!previous) return;
      setComposerText(previous.text); if (previous.document) ui.input.document = previous.document;
      if (botById(previous.bot) && !botById(previous.bot).deleted) {
        chosenBotId = previous.bot; writePreference("grokbot.bot", chosenBotId);
      }
      saveLinkDrafts(); saveDraft(); render(); ui.input.focus();
    } catch { notify("Le brouillon précédent ne peut pas être restauré."); }
  });
  $("export-button").addEventListener("click", exportConversation);
  $("export-ratings-button")?.addEventListener("click", exportRatings);
  $("find-input")?.addEventListener("input", () => {
    findState.query = $("find-input").value;
    findState.index = 0;
    findState.open = true;
    render();
  });
  $("find-input")?.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    stepFind(event.shiftKey ? -1 : 1);
  });
  $("find-next")?.addEventListener("click", () => stepFind(1));
  $("find-previous")?.addEventListener("click", () => stepFind(-1));
  $("find-close")?.addEventListener("click", () => closeFind());
  $("thread-banner-open")?.addEventListener("click", showThreadParent);
  $("rating-form")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!pendingRating) return;
    await ratingsReady;
    const selected = [...$("rating-form").querySelectorAll('input[name="motif"]')].find((input) => input.checked);
    try { await ratings.recordBad({
      conversationId: pendingRating.conversationId,
      messageId: pendingRating.messageId,
      runId: pendingRating.runId,
      motif: selected?.value || "",
      details: $("rating-details")?.value || "",
    }); }
    catch { notify("Impossible d’enregistrer la notation."); return; }
    pendingRating = null;
    $("rating-dialog")?.close();
    notify("Notation enregistrée.");
  });
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
  if (mascot) $("bot-preview-mascot").append(mascot.create(true));
  for (const id of ["bot-name", "bot-description"]) $(id).addEventListener("input", renderBotPreview);
  $("sidebar-bot-list").addEventListener("scroll", fadeBotList, { passive: true });
  // Le nombre de tuiles par rangée suit la largeur de la barre latérale.
  window.addEventListener("resize", () => { revealChosenBot(); fadeBotList(); });
  $("composer-bot").addEventListener("click", () => {
    if (!$("bot-menu").hidden) { closeBotMenu(); return; }
    if (pickerKind) { closePicker(); renderProvider(); }
    $("bot-menu").hidden = false;
    renderBotMenu();
    const checked = $("bot-menu").querySelector("[aria-checked=true]");
    checked?.scrollIntoView({ block: "nearest" });
    checked?.focus({ preventScroll: true });
  });
  $("bot-menu").addEventListener("keydown", (event) => {
    const items = [...$("bot-menu").querySelectorAll("button")];
    const index = items.indexOf(document.activeElement);
    const last = items.length - 1;
    const target = { ArrowDown: index >= last ? 0 : index + 1, ArrowUp: index <= 0 ? last : index - 1, Home: 0, End: last }[event.key];
    if (target === undefined || !items.length) return;
    event.preventDefault();
    items[target].focus({ preventScroll: true });
    items[target].scrollIntoView({ block: "nearest", inline: "nearest" });
  });
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
    if (event.key === "Escape") {
      if (findState.open && !event.target?.closest?.("dialog")) { closeFind(); return; }
      const returnToBot = !$("bot-menu").hidden;
      closeMenu(); closeSidebar(); closeBotMenu({ restoreFocus: returnToBot });
      if (pickerKind) { closePicker(); renderProvider(); }
    }
    if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
    const dialogOpen = Boolean(document.querySelector("dialog[open]"));
    if (event.key.toLowerCase() === "n" && !dialogOpen) { event.preventDefault(); newConversation(); }
    if (event.key.toLowerCase() === "f" && !event.shiftKey && !dialogOpen && $("find-bar")) { event.preventDefault(); openFind(); }
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
  document.addEventListener("keydown", (event) => {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || !event.shiftKey || event.key.toLowerCase() !== "w") return;
    if (document.querySelector("dialog[open]")) return;
    event.preventDefault();
    event.stopPropagation();
    openSkillsPanel();
  }, true);

  applyTheme(themeChoice);
  applyAccent(accentChoice);
  setComposerText(readPreference(draftKey(), ""));
  render();
  refresh().then(() => {
    setComposerText(readPreference(draftKey(), ""));
    restoreDraftDocument();
    updateComposer();
    scrollToLatest();
  }).catch(showConnectionError).finally(() => {
    // Failed boot stays unready; a successful later refresh releases the queue.
    deepLinksBootReady = true;
    deepLinks.setReady(connected && !busy);
    window.webkit?.messageHandlers?.ezerbotDeepLinks?.postMessage({ command: "ready" }).catch(() => {});
  });
})();
