"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let settings, key, errorNode, errorMessage = "", folderDraft = "";
  const attentionSeen = new Set();
  let latestContext, resizeAnchor, runningAnchor, resizePending = false;
  const dropAnchor = () => { resizeAnchor = null; runningAnchor = null; resizePending = false; };
  const expireRunning = () => { if (runningAnchor && Date.now() >= runningAnchor.until) dropAnchor(); };
  const localNames = new Set(["run_shell", "read_file", "list_directory", "write_file", "document_action"]);
  function attention(context) {
    if (!context?.conversation) return;
    const messages = context.conversation.messages || [];
    let lastUser = -1;
    for (let i = messages.length - 1; i >= 0; i -= 1) if (messages[i].role === "user") { lastUser = i; break; }
    const turn = messages.slice(lastUser + 1).reverse();
    // Host settings decisions share the approval surface and must also remain
    // reachable after resize; their arguments are not local tool commands.
    const pending = turn.find(m => m.kind === "approval" && m.payload?.status?.status === "pending" && (localNames.has(m.payload?.kind?.tool) || ["routines_background_configure", "routines_webhook_publish"].includes(m.payload?.kind?.tool) || m.payload?.connector?.origin === "settings" || m.payload?.recall?.origin === "settings"));
    const tool = turn.find(m => m.kind === "tool" && localNames.has(m.payload?.call?.name));
    const failure = tool && (tool.payload?.outcome?.status === "failed" || ["échec", "annulé", "délai dépassé"].includes(tool.payload?.outcome?.output?.status));
    const running = tool && typeof tool.payload?.run_id === "string" && !tool.payload.outcome && tool.payload.run_status === "running"
      && turn.some(m => m.kind === "approval" && m.payload?.run_id === tool.payload.run_id
        && m.payload?.kind?.tool === tool.payload.call.name && m.payload?.status?.status === "resolved");
    const message = pending || (failure || running ? tool : null);
    if (!message) return;
    const phase = pending ? "pending" : running ? "running" : tool.payload.outcome.status === "failed" ? "failed" : tool.payload.outcome.output.status;
    return { message, phase, token: `${context.conversation.id}:${message.id}:${phase}:${message.payload?.outcome?.reason || ""}` };
  }
  function focusAttention(context, resized = false, immediate = false) {
    expireRunning();
    const wanted = attention(context);
    if (!wanted || typeof window.requestAnimationFrame !== "function") return;
    if (resized ? resizeAnchor !== wanted.token : attentionSeen.has(wanted.token)) return;
    const { message, phase, token } = wanted;
    // Run after shell anchoring and the matching React commit, not an existing
    // call-only placeholder. Retry briefly, never across a changed turn/modal.
    const settle = remaining => {
      if (attention(latestContext)?.token !== token || (resized ? resizeAnchor !== token : attentionSeen.has(token)) || document.querySelector("dialog[open]")) return;
      const target = [...document.querySelectorAll("[data-message-id]")].find(e => e.dataset.messageId === message.id);
      if (!target || (phase !== "pending" && target.dataset.localState !== phase)) {
        if (remaining > 0) window.requestAnimationFrame(() => settle(remaining - 1));
        return;
      }
      attentionSeen.add(token);
      if (attentionSeen.size > 500) attentionSeen.delete(attentionSeen.values().next().value);
      runningAnchor = phase === "running" ? { token, target, until: runningAnchor?.token === token ? runningAnchor.until : Date.now() + 1000 } : null;
      resizeAnchor = token;
      resizePending = false;
      userScroll = null;
      if (phase === "pending") window.GrokbotChat.revealPendingApproval(target, scroll);
      else {
        const stop = phase === "running" ? target.querySelector?.(".gb-local-stop") : null;
        if (stop && target.getBoundingClientRect().height > scroll?.clientHeight) stop.scrollIntoView({ block: "end" });
        else target.scrollIntoView({ block: "start" });
      }
    };
    if (immediate) settle(0);
    else window.requestAnimationFrame(() => window.requestAnimationFrame(() => settle(30)));
  }
  const scroll = document.getElementById?.("chat-scroll");
  let userScroll = null, inputSequence = 0, pointerHeld = false;
  if (scroll && typeof window.ResizeObserver === "function") {
    let width = scroll.clientWidth, height = scroll.clientHeight;
    new window.ResizeObserver(() => {
      expireRunning();
      if (width === scroll.clientWidth && height === scroll.clientHeight) return;
      width = scroll.clientWidth; height = scroll.clientHeight;
      if (resizeAnchor && attention(latestContext)?.token === resizeAnchor) {
        resizePending = true;
        focusAttention(latestContext, true);
      }
    }).observe(scroll);
    const intent = event => {
      if (event.type === "pointermove" && !event.buttons) return;
      if (event.type === "keydown" && (!new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "]).has(event.key) || event.target?.closest?.("input,textarea,select,[contenteditable=true]"))) return;
      if (event.type === "keydown") { dropAnchor(); userScroll = null; ++inputSequence; return; }
      if (event.type === "pointerdown") pointerHeld = true;
      const sequence = ++inputSequence;
      userScroll = { top: scroll.scrollTop, width: scroll.clientWidth, height: scroll.clientHeight };
      if (!pointerHeld) window.requestAnimationFrame(() => window.requestAnimationFrame(() => { if (inputSequence === sequence) userScroll = null; }));
    };
    for (const type of ["wheel", "touchmove", "pointerdown", "pointermove"]) scroll.addEventListener(type, intent, { passive: true });
    // A removed approval button leaves focus on BODY; keyboard scrolling still
    // targets the last clicked scroller. Observe the real document event path.
    document.addEventListener("keydown", intent, true);
    const releasePointer = () => {
      pointerHeld = false;
      const sequence = ++inputSequence;
      window.requestAnimationFrame(() => window.requestAnimationFrame(() => { if (inputSequence === sequence) userScroll = null; }));
    };
    for (const type of ["pointerup", "pointercancel"]) document.addEventListener(type, releasePointer, true);
    scroll.addEventListener("scroll", () => {
      // Layout clamping and our own scrolls are not a reader leaving the card.
      if (userScroll && width === scroll.clientWidth && height === scroll.clientHeight && userScroll.width === width && userScroll.height === height && Math.abs(scroll.scrollTop - userScroll.top) > 1) {
        dropAnchor(); userScroll = null;
        return;
      }
      // Only the brief post-approval window can repair a late shell follow.
      // Cached running metadata keeps ordinary scroll events O(1).
      expireRunning();
      const active = !userScroll && runningAnchor;
      if (active && active.token === resizeAnchor) {
        const target = active.target;
        const controls = [target?.querySelector(".gb-local-header"), target?.querySelector(".gb-local-stop")];
        const view = scroll.getBoundingClientRect();
        if (controls.every(Boolean) && controls.some(e => { const r = e.getBoundingClientRect(); return r.top < view.top || r.bottom > view.bottom; })) {
          resizePending = true;
          focusAttention(latestContext, true, true);
        }
      }
    });
  }
  const node = (tag, text) => { const e = document.createElement(tag); if (text) e.textContent = text; return e; };
  const showError = (message) => {
    errorMessage = message;
    if (errorNode) { errorNode.textContent = message; errorNode.hidden = !message; if (message) errorNode.scrollIntoView?.({ block: "nearest" }); }
  };
  const save = async (path, body, button) => {
    showError("");
    if (button) button.disabled = true;
    let accepted = false;
    try {
      await panels.api(path, body); accepted = true;
      if (path.endsWith("/folders/add")) folderDraft = "";
      await panels.refresh(); key = null; render(panels.state); showError("");
    } catch (error) {
      key = null; render(panels.state);
      showError(accepted ? "Modification enregistrée, mais son actualisation a échoué. Actualisez avant de réessayer." : error.message || "Modification non confirmée.");
    } finally { if (button) button.disabled = false; }
  };
  function render(state) {
    if (!settings) return;
    const next = JSON.stringify([state.local, state.bots?.map(b => [b.id,b.name,b.deleted])]);
    if (next === key) return;
    key = next;
    settings.className = "setting-section local-settings";
    settings.replaceChildren(node("h3", "Terminal et fichiers sur ce Mac"), node("p", "Désactivés par défaut. Chaque commande et chaque écriture demandent votre accord, sauf règle « Toujours ». Le terminal n’est pas un sandbox : une commande peut accéder à tout votre Mac."));
    if (state.local?.error) { settings.append(node("p", state.local.error)); return; }
    for (const bot of state.bots || []) {
      if (bot.deleted) continue;
      const current = state.local?.bots?.find(b => b.bot_id === bot.id) || { shell: false, files: false };
      const row = node("div"); row.className = "local-bot-row";
      row.append(node("strong", bot.name));
      for (const [field, title] of [["shell", "Terminal"], ["files", "Fichiers"]]) {
        const label = node("label"); label.className = "setting-toggle";
        const input = node("input"); input.type = "checkbox"; input.name = `local-${bot.id}-${field}`; input.checked = Boolean(current[field]);
        input.setAttribute("aria-label", `${title} pour ${bot.name}`); input.setAttribute("role", "switch");
        input.addEventListener("change", () => save("/api/local/bot", { bot_id: bot.id, [field]: input.checked }, input));
        const track = node("span"); track.className = "toggle-track"; track.setAttribute("aria-hidden", "true");
        label.append(node("span", title), input, track); row.append(label);
      }
      settings.append(row);
    }
    settings.append(node("h4", "Dossiers accordés en lecture"), node("p", "Tous les bots dont les fichiers sont activés peuvent lire ces dossiers sans carte. Ailleurs, une carte reste obligatoire, même avec une règle permanente. Retirer un dossier bloque les prochaines lectures automatiques."));
    const folders = state.local?.folders || [];
    if (!folders.length) settings.append(node("p", "Aucun dossier accordé."));
    for (const path of folders) {
      const row = node("div"); row.className = "local-folder-row";
      const code = node("code", path); const remove = node("button", "Retirer"); remove.type = "button";
      remove.addEventListener("click", () => save("/api/local/folders/remove", { path }, remove));
      row.append(code, remove);
      const item = node("div"); item.className = "local-folder-item"; item.append(row);
      if (state.local?.folders_with_app_data?.includes(path)) {
        const warning = node("p", "Les données d’Ezerbot dans ce dossier restent protégées par une carte, même avec ce dossier accordé.");
        warning.className = "local-data-warning"; item.append(warning);
      }
      settings.append(item);
    }
    const form = node("form"); form.className = "local-folder-form";
    const input = node("input"); input.type = "text"; input.name = "local-folder"; input.value = folderDraft; input.placeholder = "/Users/vous/Documents"; input.setAttribute("aria-label", "Chemin absolu du dossier à accorder"); input.required = true;
    input.addEventListener("input", () => { folderDraft = input.value; showError(""); });
    const add = node("button", "Accorder ce dossier"); add.type = "submit";
    form.append(input, add);
    form.addEventListener("submit", event => { event.preventDefault(); folderDraft = input.value.trim(); return save("/api/local/folders/add", { path: folderDraft }, add); });
    errorNode = node("p", errorMessage); errorNode.className = "local-folder-error"; errorNode.setAttribute("role", "alert"); errorNode.hidden = !errorMessage;
    settings.append(form, errorNode);
  }
  document.getElementById?.("settings-dialog")?.addEventListener("close", () => focusAttention(latestContext, resizePending));
  panels.register({
    id: "local", settingsSection(container) { settings = container; render(panels.state); },
    onState(state, context) { render(state); latestContext = context; if (resizeAnchor && attention(context)?.token !== resizeAnchor) { dropAnchor(); } focusAttention(context); },
    wantsPolling(context) {
      return (context?.conversation?.messages || []).some((message) => message.payload?.outcome?.output?.background === true && message.payload?.outcome?.output?.status === "en cours");
    },
    async onCardAction(message, action, detail, context) {
      if (message.kind !== "tool") return false;
      if (action === "local.task.stop") {
        await panels.api("/api/local/tasks/stop", { run_id: message.payload.run_id, call_id: message.payload.call?.call_id });
        await panels.refresh(); return true;
      }
      if (action === "local.task.input") {
        await panels.api("/api/local/tasks/input", { run_id: message.payload.run_id, call_id: message.payload.call?.call_id, line: detail?.line ?? "" });
        await panels.refresh(); return true;
      }
      if (action !== "local.stop") return false;
      await panels.api("/api/stop", { conversation_id: context.conversation.id, message_id: message.payload.run_id });
      await panels.refresh(); return true;
    },
  });
})();
