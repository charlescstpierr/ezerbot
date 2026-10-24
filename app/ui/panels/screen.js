"use strict";
(() => {
  // Native session administration stays on the Mac; remote tools still use
  // the same approval cards and may display authenticated transcript captures.
  if (window.EzerbotIPhone?.remote) return;
  const host = window.GrokbotPanels;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  const button = (text, action, cls = "secondary-button") => { const b = node("button", text, cls); b.type = "button"; b.onclick = action; return b; };
  let data = {}, context = {}, section, signature = "", error = "", saving = false, expiryTimer;
  // Le Mode YOLO ouvre l'écran à ses bots ; la session, elle, reste à accorder ici.
  let lastYolo;
  const yoloBot = (id) => host.state.yolo?.bots?.find(b => b.bot_id === id)?.effective === "yolo";
  const screenOpen = (id) => Boolean(id) && (yoloBot(id) || Boolean(data.bots?.some(b => b.bot_id === id && b.enabled)));
  function updateEntry() {
    entry.hidden = !data.session && !(context.conversation?.kind === "chat" && screenOpen(context.bot?.id));
    menuItem.hidden = entry.hidden;
    // Sans conversation ouverte, le menu est désactivé : une session en cours garde son entrée dans l'en-tête.
    entry.classList.toggle("screen-button-kept", Boolean(data.session) && !context.conversation);
  }
  const entry = button("Écran du Mac", () => { renderDialog(); dialog.showModal(); showOverlay(); }, "tool-pill screen-button");
  entry.setAttribute("aria-label", "Écran du Mac"); entry.title = "Écran du Mac";
  document.querySelector(".topbar-actions").prepend(entry);
  // Sous 1100 px l'en-tête n'a plus la place du titre : la même entrée passe dans le menu de la conversation.
  const menuItem = node("button", null, "screen-menu-item"); menuItem.type = "button"; menuItem.hidden = true;
  menuItem.innerHTML = '<svg class="icon"><use href="#i-monitor"/></svg>'; menuItem.append(node("span", "Écran du Mac"));
  menuItem.onclick = () => {
    document.getElementById("conversation-menu").hidden = true;
    document.getElementById("conversation-menu-button").setAttribute("aria-expanded", "false");
    entry.onclick();
  };
  document.getElementById("conversation-menu").prepend(menuItem);
  const dialog = node("dialog", null, "dialog screen-dialog"); dialog.setAttribute("aria-labelledby", "screen-title");
  document.getElementById("panels").append(dialog);
  const overlay = node("div", null, "screen-overlay"); overlay.setAttribute("role", "status"); overlay.hidden = true; document.querySelector(".topbar").after(overlay);
  function showOverlay() {
    const floating = Boolean(document.querySelector("dialog[open]"));
    if (overlay.hidePopover && overlay.matches(":popover-open")) overlay.hidePopover();
    overlay.classList.toggle("screen-floating", floating);
    if (floating) overlay.setAttribute("popover", "manual");
    else overlay.removeAttribute("popover");
    overlay.hidden = !data.session;
    preview.classList.toggle("screen-preview-active", Boolean(data.session));
    if (data.session && floating && overlay.showPopover) overlay.showPopover();
  }
  function scheduleExpiry() {
    clearTimeout(expiryTimer);
    if (!data.session) return;
    expiryTimer = setTimeout(() => {
      data = { ...data, session: null };
      renderOverlay(); if (dialog.open) renderDialog();
      host.refresh().catch(() => {});
    }, Math.max(0, data.session.expires_at - Date.now() + 50));
  }
  async function save(path, body) {
    if (saving) return;
    saving = true; error = "";
    try { await host.api(path, body); await host.refresh(); }
    catch (e) { error = e.message; }
    finally { saving = false; signature = ""; renderSettings(); if (dialog.open) renderDialog(); renderOverlay(); }
  }
  // Emergency stop must never be swallowed by the ordinary save guard (e.g.
  // a slow resume or refresh). Revoke is idempotent and has its own request.
  async function stop() {
    if (!data.session) return;
    const id = data.session.id;
    try { await host.api("/api/screen/revoke", { id }); await host.refresh(); host.notify("Contrôle de l’écran arrêté."); }
    catch (e) { error = e.message; }
    finally { signature = ""; renderSettings(); if (dialog.open) renderDialog(); renderOverlay(); }
  }
  async function pane(name) {
    const bridge = window.webkit?.messageHandlers?.ezerbotNotifications;
    if (data.fake) { host.notify("Mode de test : aucun panneau système ni demande de permission."); return; }
    if (!bridge) { host.notify("Ouvrez l’application native Ezerbot pour accéder aux Réglages Système."); return; }
    await bridge.postMessage({ command: "screenSettings", pane: name });
  }
  function permissionButtons(container) {
    const actions = node("div", null, "screen-actions");
    actions.append(button("Ouvrir Accessibilité", () => pane("accessibility")), button("Ouvrir Enregistrement de l’écran", () => pane("recording")), button("Ouvrir Surveillance de l’entrée", () => pane("input")));
    container.append(actions);
  }
  function renderSettings() {
    if (!section) return;
    const key = JSON.stringify([data, host.state.yolo?.bots, host.state.bots?.map(b => [b.id,b.name,b.deleted]), error, saving]);
    if (signature === key) return; signature = key;
    section.className = "setting-section screen-settings"; section.replaceChildren(node("h3", "Écran du Mac"), node("p", host.state.yolo?.mode === "yolo"
      ? "Le Mode YOLO ouvre l’écran aux bots qui le suivent : vous accordez toujours chaque session (un bot, un fil direct, une durée), puis ses gestes partent sans carte. Échap révoque le contrôle partout."
      : "Désactivé par défaut. Chaque session est limitée à un bot, un fil direct et une durée. Chaque action demande votre confirmation ; aucun achat, envoi ou suppression automatique."));
    if (data.fake) section.append(node("p", "Version de développement : écran fictif et événements enregistrés. Aucune capture réelle, saisie réelle ou permission macOS.", "screen-test-note"));
    for (const bot of host.state.bots?.filter(b => !b.deleted) || []) {
      const row = node("div", null, "screen-bot-row"), label = node("label", null, "setting-toggle"), input = node("input"); input.type = "checkbox"; input.setAttribute("role", "switch"); input.setAttribute("aria-label", `Contrôle de l’écran pour ${bot.name}`); input.name = `screen-${bot.id}`; input.checked = screenOpen(bot.id); input.disabled = saving || yoloBot(bot.id); if (yoloBot(bot.id)) input.title = "Ouvert par le Mode YOLO";
      input.onchange = () => save("/api/screen/bot", { bot_id: bot.id, enabled: input.checked });
      const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true");
      label.append(node("span", "Écran"), input, track); row.append(node("strong", bot.name), label); section.append(row);
    }
    section.append(node("p", "Dans un fil existant, ouvrez « Écran du Mac » en haut pour accorder une session. Une entrée de la souris ou du clavier hors d’une fenêtre d’Ezerbot met le bot en pause. Répondre aux cartes ou écrire dans Ezerbot ne met pas en pause. Échap révoque le contrôle partout."));
    if (error) { const alert = node("p", error, "screen-error"); alert.setAttribute("role", "alert"); section.append(alert); }
    if (/permission|Accessibilité|Enregistrement|Surveillance/i.test(error)) permissionButtons(section);
    else { const settings = node("details"); settings.append(node("summary", "Réglages Système…")); permissionButtons(settings); section.append(settings); }
  }
  function renderOverlay() {
    overlay.replaceChildren();
    if (data.session) {
      overlay.append(node("strong", data.session.paused ? "Ezerbot en pause — Vous avez repris la main" : "Ezerbot contrôle l’écran — Échap pour arrêter"));
      if (data.session.paused) overlay.append(button("Reprendre", () => save("/api/screen/resume", { id: data.session.id })));
      overlay.append(button("Arrêter", stop));
    }
    showOverlay();
    entry.classList.toggle("screen-active", Boolean(data.session));
    updateEntry();
  }
  function renderDialog() {
    dialog.replaceChildren();
    const header = node("div", null, "dialog-header"), title = node("h2", "Écran du Mac"); title.id = "screen-title";
    header.append(title, button("Fermer", () => dialog.close())); dialog.append(header);
    if (data.fake) dialog.append(node("p", "Écran fictif · aucune action sur votre Mac", "screen-test-note"));
    if (data.session) {
      const s = data.session, bot = host.state.bots?.find(b => b.id === s.bot_id), chat = host.state.conversations?.find(c => c.id === s.conversation_id);
      dialog.append(node("h3", s.paused ? "En pause — Vous avez la main" : "Ezerbot contrôle l’écran — Échap pour arrêter"), node("p", `${bot?.name || "Bot"} · ${chat?.title || "Conversation"} · jusqu’à ${new Date(s.expires_at).toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" })}`));
      dialog.append(node("p", yoloBot(s.bot_id) ? "Mode YOLO : les gestes de cette session partent sans carte. Vous pouvez arrêter à tout moment ; les effets déjà livrés ne sont pas annulés." : "L’accord n’autorise aucune action sensible sans sa carte. Vous pouvez arrêter à tout moment, même si des actions attendent. Les effets déjà livrés ne sont pas annulés."));
      const actions = node("div", null, "screen-actions");
      if (s.paused) actions.append(button("Reprendre le bot", () => save("/api/screen/resume", { id: s.id }), "primary-button"));
      actions.append(button("Révoquer maintenant", stop, "danger-button"));
      if (data.fake) actions.append(button("Simuler une entrée hors d’Ezerbot", () => save("/api/screen/fake-input", { id: s.id })));
      dialog.append(actions);
    } else {
      const chat = context.conversation, bot = context.bot;
      dialog.append(node("p", "Le modèle pourra voir des captures de l’écran principal et proposer des gestes. Les captures et actions restent dans le fil et sont envoyées au fournisseur d’IA. Fermez vos fenêtres privées avant de commencer."));
      const available = chat?.kind === "chat" && screenOpen(bot?.id);
      if (!available) dialog.append(node("p", "Activez d’abord ce bot dans Paramètres › Écran du Mac, puis ouvrez un fil direct existant (envoyez un message pour créer le fil)."), button("Ouvrir Paramètres", () => { dialog.close(); document.getElementById("settings-button").click(); }));
      else {
        dialog.append(node("h3", `${bot.name} · ${chat.title}`));
        const form = node("form"), label = node("label", "Durée de cet accord", "form-label"), duration = node("select", null, "text-input"); duration.name = "screen-duration";
        for (const minutes of [1,5,15]) { const opt = node("option", `${minutes} minute${minutes > 1 ? "s" : ""}`); opt.value = String(minutes); duration.append(opt); }
        duration.value = "5"; label.append(duration);
        // Le texte de consentement dit si les gestes passent par une carte : il ne vaut que pour ce mode.
        const consentMode = yoloBot(bot.id);
        const consent = node("label", null, "screen-consent"), check = node("input"); check.type = "checkbox"; check.required = true; check.name = "screen-consent";
        consent.append(check, node("span", yoloBot(bot.id) ? "J’autorise ce bot à voir cet écran et à agir, sans carte (Mode YOLO), dans ce fil et pendant cette durée. Échap arrête tout." : "J’autorise ce bot à voir cet écran et à proposer des gestes, chacun soumis à ma confirmation, dans ce fil et pendant cette durée. Échap arrête tout."));
        const allow = node("button", "Accorder cette session", "primary-button"); allow.type = "submit"; allow.disabled = true;
        check.onchange = () => { allow.disabled = saving || !check.checked; };
        form.append(label, consent, allow); form.onsubmit = event => { event.preventDefault(); if (yoloBot(bot.id) !== consentMode) { renderDialog(); return; } if (check.checked) save("/api/screen/grant", { bot_id: bot.id, conversation_id: chat.id, minutes: Number(duration.value) }); }; dialog.append(form);
      }
    }
    if (error) { const alert = node("p", error, "screen-error"); alert.setAttribute("role", "alert"); dialog.append(alert); }
    if (/permission|Accessibilité|Enregistrement|Surveillance/i.test(error)) permissionButtons(dialog);
  }
  window.addEventListener("keydown", event => { if (event.key === "Escape" && data.session) { stop(); } }, true);
  // All DOM input is inside Ezerbot and is deliberately exempt. Debug cannot
  // observe another Mac window: the explicit fixture button above represents
  // that external input. Release classifies the actual destination window in
  // the native listen-only tap; Escape remains unconditional everywhere.
  document.addEventListener("toggle", event => { if (event.target instanceof HTMLDialogElement) showOverlay(); }, true);
  const preview = node("dialog", null, "dialog screen-preview");
  document.getElementById("panels").append(preview);
  window.EzerbotScreen = { pane, preview(image) {
    if (typeof image !== "string" || !image.startsWith("/api/screen/shot?")) return;
    const img = node("img"); img.src = image; img.alt = "Capture de l’écran après l’action";
    const header = node("div", null, "dialog-header"); header.append(node("h2", "Capture de l’écran"), button("Fermer", () => preview.close()));
    preview.replaceChildren(header, img); preview.showModal(); showOverlay();
  } };
  host.register({
    id: "screen",
    settingsSection(container) { section = container; renderSettings(); },
    onState(state, ctx) { const yoloNow = yoloBot(ctx?.bot?.id); const yoloChanged = yoloNow !== lastYolo; lastYolo = yoloNow; const incoming = state.screen || {}; const next = { ...incoming, session: incoming.session?.expires_at > Date.now() ? incoming.session : null }; const changed = JSON.stringify(data) !== JSON.stringify(next); data = next; context = ctx; renderSettings(); updateEntry(); if (changed) { scheduleExpiry(); renderOverlay(); if (dialog.open) renderDialog(); } else if (yoloChanged && dialog.open) renderDialog(); },
    wantsPolling() { return Boolean(data.session); },
  });
})();
