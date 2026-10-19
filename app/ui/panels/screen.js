"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  const button = (text, action, cls = "secondary-button") => { const b = node("button", text, cls); b.type = "button"; b.onclick = action; return b; };
  let data = {}, context = {}, section, signature = "", error = "", saving = false;
  const entry = button("Écran du Mac", () => { renderDialog(); dialog.showModal(); showOverlay(); }, "tool-pill screen-button");
  document.querySelector(".topbar-actions").prepend(entry);
  const dialog = node("dialog", null, "dialog screen-dialog"); dialog.setAttribute("aria-labelledby", "screen-title");
  document.getElementById("panels").append(dialog);
  const overlay = node("div", null, "screen-overlay"); overlay.setAttribute("role", "status"); overlay.setAttribute("popover", "manual"); document.body.append(overlay);
  function showOverlay() {
    if (data.session) { if (overlay.showPopover && !overlay.matches(":popover-open")) overlay.showPopover(); }
    else if (overlay.hidePopover && overlay.matches(":popover-open")) overlay.hidePopover();
    overlay.hidden = !data.session;
  }
  async function save(path, body) {
    if (saving) return;
    saving = true; error = "";
    try { await host.api(path, body); await host.refresh(); }
    catch (e) { error = e.message; }
    finally { saving = false; signature = ""; renderSettings(); if (dialog.open) renderDialog(); renderOverlay(); }
  }
  async function stop() { if (data.session) await save("/api/screen/revoke", { id: data.session.id }); }
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
    const key = JSON.stringify([data, host.state.bots?.map(b => [b.id,b.name,b.deleted]), error, saving]);
    if (signature === key) return; signature = key;
    section.className = "setting-section screen-settings"; section.replaceChildren(node("h3", "Écran du Mac"), node("p", "Désactivé par défaut. Chaque session est limitée à un bot, un fil direct et une durée. Chaque action demande votre confirmation ; aucun achat, envoi ou suppression automatique."));
    if (data.fake) section.append(node("p", "Version de développement : écran fictif et événements enregistrés. Aucune capture réelle, saisie réelle ou permission macOS.", "screen-test-note"));
    for (const bot of host.state.bots?.filter(b => !b.deleted) || []) {
      const label = node("label", null, "setting-toggle screen-bot-row"), input = node("input"); input.type = "checkbox"; input.setAttribute("role", "switch"); input.setAttribute("aria-label", `Contrôle de l’écran pour ${bot.name}`); input.name = `screen-${bot.id}`; input.checked = Boolean(data.bots?.find(b => b.bot_id === bot.id)?.enabled); input.disabled = saving;
      input.onchange = () => save("/api/screen/bot", { bot_id: bot.id, enabled: input.checked });
      const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true");
      label.append(node("span", `Autoriser ${bot.name} à demander le contrôle`), input, track); section.append(label);
    }
    section.append(node("p", "Dans un fil existant, ouvrez « Écran du Mac » en haut pour accorder une session. Toute activité réelle de la souris ou du clavier met le bot en pause. Échap révoque le contrôle."));
    if (error) { const alert = node("p", error, "screen-error"); alert.setAttribute("role", "alert"); section.append(alert); }
    permissionButtons(section);
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
  }
  function renderDialog() {
    dialog.replaceChildren();
    const header = node("div", null, "dialog-header"), title = node("h2", "Écran du Mac"); title.id = "screen-title";
    header.append(title, button("Fermer", () => dialog.close())); dialog.append(header);
    if (data.fake) dialog.append(node("p", "Écran fictif · aucune action sur votre Mac", "screen-test-note"));
    if (data.session) {
      const s = data.session, bot = host.state.bots?.find(b => b.id === s.bot_id), chat = host.state.conversations?.find(c => c.id === s.conversation_id);
      dialog.append(node("h3", s.paused ? "En pause — Vous avez la main" : "Ezerbot contrôle l’écran — Échap pour arrêter"), node("p", `${bot?.name || "Bot"} · ${chat?.title || "Conversation"} · jusqu’à ${new Date(s.expires_at).toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}`));
      dialog.append(node("p", "L’accord n’autorise aucune action sensible sans sa carte. Vous pouvez arrêter à tout moment, même si des actions attendent. Les effets déjà livrés ne sont pas annulés."));
      const actions = node("div", null, "screen-actions");
      if (s.paused) actions.append(button("Reprendre le bot", () => save("/api/screen/resume", { id: s.id }), "primary-button"));
      actions.append(button("Révoquer maintenant", stop, "danger-button"));
      if (data.fake) actions.append(button("Simuler une reprise humaine", () => save("/api/screen/fake-input", { id: s.id })));
      dialog.append(actions);
    } else {
      const chat = context.conversation, bot = context.bot;
      dialog.append(node("p", "Le modèle pourra voir des captures de l’écran principal et proposer des gestes. Les captures et actions restent dans le fil et sont envoyées au fournisseur d’IA. Fermez vos fenêtres privées avant de commencer."));
      const available = chat?.kind === "chat" && data.bots?.some(b => b.bot_id === bot?.id && b.enabled);
      if (!available) dialog.append(node("p", "Activez d’abord ce bot dans Paramètres › Écran du Mac, puis ouvrez un fil direct existant (envoyez un message pour créer le fil)."));
      else {
        dialog.append(node("h3", `${bot.name} · ${chat.title}`));
        const form = node("form"), label = node("label", "Durée de cet accord", "form-label"), duration = node("select", null, "text-input"); duration.name = "screen-duration";
        for (const minutes of [1,5,15]) { const opt = node("option", `${minutes} minute${minutes > 1 ? "s" : ""}`); opt.value = String(minutes); duration.append(opt); }
        duration.value = "5"; label.append(duration);
        const consent = node("label", null, "screen-consent"), check = node("input"); check.type = "checkbox"; check.required = true; check.name = "screen-consent";
        consent.append(check, node("span", "J’autorise les captures de cet écran pour ce bot et ce fil, pendant cette durée. Je peux arrêter avec Échap à tout moment."));
        const allow = node("button", "Accorder cette session", "primary-button"); allow.type = "submit"; allow.disabled = saving;
        form.append(label, consent, allow); form.onsubmit = event => { event.preventDefault(); save("/api/screen/grant", { bot_id: bot.id, conversation_id: chat.id, minutes: Number(duration.value) }); }; dialog.append(form);
      }
    }
    if (error) { const alert = node("p", error, "screen-error"); alert.setAttribute("role", "alert"); dialog.append(alert); }
    permissionButtons(dialog);
  }
  window.addEventListener("keydown", event => { if (event.key === "Escape" && data.session) { event.preventDefault(); stop(); } }, true);
  // Debug-only simulated takeover. Native release input monitoring is entirely
  // outside the WebView, so changing tabs/apps cannot evade it.
  let pausing = false;
  async function fakeHuman(event) {
    if (!event.isTrusted || !data.fake || !data.session || data.session.paused || pausing || event.key === "Escape") return;
    pausing = true;
    try { await host.api("/api/screen/fake-input", { id: data.session.id }); await host.refresh(); } finally { pausing = false; }
  }
  for (const name of ["pointerdown", "pointermove", "keydown", "wheel"]) window.addEventListener(name, fakeHuman, { passive: true });
  document.addEventListener("toggle", event => { if (event.target instanceof HTMLDialogElement && data.session) { if (overlay.hidePopover && overlay.matches(":popover-open")) overlay.hidePopover(); showOverlay(); } }, true);
  const preview = node("dialog", null, "dialog screen-preview");
  document.getElementById("panels").append(preview);
  window.EzerbotScreen = { pane, preview(image) {
    if (typeof image !== "string" || !image.startsWith("data:image/png;base64,")) return;
    const img = node("img"); img.src = image; img.alt = "Capture de l’écran après l’action";
    const header = node("div", null, "dialog-header"); header.append(node("h2", "Capture de l’écran"), button("Fermer", () => preview.close()));
    preview.replaceChildren(header, img); preview.showModal(); showOverlay();
  } };
  host.register({
    id: "screen",
    settingsSection(container) { section = container; renderSettings(); },
    onState(state, ctx) { const changed = JSON.stringify(data) !== JSON.stringify(state.screen || {}); data = state.screen || {}; context = ctx; renderSettings(); if (changed) { renderOverlay(); if (dialog.open) renderDialog(); } },
    wantsPolling() { return Boolean(data.session); },
  });
})();
