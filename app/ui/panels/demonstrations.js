"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  const button = (text, action, cls = "secondary-button") => { const el = node("button", text, cls); el.type = "button"; el.onclick = action; return el; };
  let data = {}, context = {}, target = null, busy = false, error = "", explanation = "", keep = false, signature = "", lastSession = null;
  const connectionRequired = "Connectez un fournisseur d’IA avant de montrer une tâche : la démonstration lui sera envoyée pour proposer la skill.";
  const dialog = node("dialog", null, "dialog demonstration-dialog"); dialog.setAttribute("aria-labelledby", "demonstration-title"); document.getElementById("panels").append(dialog);
  const overlay = node("div", null, "demonstration-overlay"); overlay.hidden = true; overlay.setAttribute("role", "status"); document.querySelector(".topbar").after(overlay);
  const menu = button("Montrer au bot", () => {
    document.getElementById("conversation-menu").hidden = true;
    document.getElementById("conversation-menu-button").setAttribute("aria-expanded", "false");
    open(context.bot?.id);
  }, ""); menu.id = "show-bot-button";
  const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg"); icon.classList.add("icon"); icon.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use"); use.setAttribute("href", "#i-monitor"); icon.append(use); menu.prepend(icon);
  document.getElementById("learn-skill-button").after(menu);
  const entry = button("Démonstration", () => open(), "tool-pill"); entry.hidden = true; document.querySelector(".topbar-actions").prepend(entry);
  const summaryDialog = node("dialog", null, "dialog demonstration-dialog"); summaryDialog.setAttribute("aria-label", "Synthèse de la démonstration"); document.getElementById("panels").append(summaryDialog);
  let overlaySignature = "";
  const remaining = s => { const seconds = Math.max(0, Math.ceil((s.expires_at - Date.now()) / 1000)); return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")} restantes`; };
  async function action(path, body) {
    if (busy) return false;
    busy = true; error = ""; render();
    try { await host.api(path, body); await host.refresh(); return true; }
    catch (e) { error = e.message; return false; }
    finally { busy = false; signature = ""; render(); updateOverlay(); }
  }
  function focusControl(preferred = null) {
    if (!dialog.open || summaryDialog.open) return;
    const controls = [...dialog.querySelectorAll("button, textarea, input")].filter(control => !control.disabled);
    const restore = controls.find(control => preferred?.name ? control.name === preferred.name : preferred?.text && control.tagName === "BUTTON" && control.textContent === preferred.text);
    const next = restore || (data.session?.status === "review" && controls.find(control => control.name === "demonstration-explanation")) || controls[0];
    next?.focus({ preventScroll: true });
    if (restore?.setSelectionRange && typeof preferred?.start === "number") restore.setSelectionRange(preferred.start, preferred.end);
  }
  function header(title) { const h = node("div", null, "dialog-header"); const name = node("h2", title); name.id = "demonstration-title"; h.append(name, button("Fermer", () => dialog.close())); return h; }
  function open(bot) {
    target = bot || data.session?.bot_id || context.bot?.id; error = ""; signature = ""; render();
    if (!dialog.open) dialog.showModal(); focusControl(); updateOverlay();
  }
  function openCard() {
    const s = data.session; if (!s) return;
    const request = host.state.forms?.requests?.find(r => r.id === s.request_id);
    dialog.close();
    host.openConversation(request?.route?.target_conversation_id || s.conversation_id, request?.entry_id);
  }
  async function start() {
    if (busy || !target) return;
    const consent = dialog.querySelector("[name=demonstration-consent]"); if (!consent?.checked) return;
    if (!host.state.provider?.connected) { error = connectionRequired; signature = ""; render(); return; }
    busy = true; error = "";
    const bot = target;
    try {
      let chat = context.conversation;
      if (chat?.bot_id !== bot || chat?.kind !== "chat") chat = host.state.conversations?.find(c => c.bot_id === bot && c.kind === "chat");
      if (!chat) chat = (await host.api("/api/conversations", { bot_id: bot })).conversation;
      await host.api("/api/demonstrations/start", { bot_id: bot, conversation_id: chat.id, consent: true });
      explanation = ""; keep = false;
      await host.refresh();
      // Closing another editor would discard unsaved bot/skill changes.
      dialog.close();
      host.openConversation(chat.id);
      // The persistent recording overlay is the status; no stale toast should
      // cover the later approval after a very short demonstration.
    } catch (e) { error = e.message; }
    finally { busy = false; signature = ""; render(); updateOverlay(); }
  }
  async function stop() {
    if (!data.session || busy) return;
    if (await action("/api/demonstrations/stop", { id: data.session.id })) open();
  }
  async function discard() {
    if (!data.session || busy) return;
    if (await action("/api/demonstrations/discard", { id: data.session.id })) { dialog.close(); host.notify("Démonstration ignorée. Enregistrement supprimé."); }
  }
  async function draft() {
    if (!data.session || busy) return;
    if (await action("/api/demonstrations/draft", { id: data.session.id, explanation, keep_images: keep })) { dialog.close(); openCard(); }
  }
  function updateOverlay() {
    const key = JSON.stringify([data.session?.id, data.session?.status, data.fake, busy, context.conversation?.kind]);
    if (key === overlaySignature) return; overlaySignature = key;
    overlay.replaceChildren();
    const s = data.session, recording = s?.status === "recording";
    overlay.hidden = !recording; entry.hidden = !s || recording;
    menu.disabled = busy || (!s && context.conversation?.kind !== "chat");
    if (!recording) return;
    const countdown = node("span", remaining(s)); countdown.dataset.demoCountdown = "true";
    const controls = button("Enregistrement de la démonstration", () => open(), "demonstration-controls"); controls.title = "Ouvrir les contrôles de la démonstration";
    overlay.append(controls, countdown, button("Arrêter", stop));
    if (data.fake) overlay.append(node("small", "Écran fictif · aucune entrée réelle"));
    // This is an ordinary flex item under the main panel's topbar. In a modal,
    // recording controls are also available in its dedicated session dialog.
  }
  function render() {
    const s = data.session;
    const next = JSON.stringify([s, target, error, busy, data.fake, host.state.provider?.connected]); if (signature === next) return; signature = next;
    const active = document.activeElement;
    const preferred = dialog.contains(active) ? { name: active.name, text: active.textContent, start: active.selectionStart, end: active.selectionEnd } : null;
    dialog.replaceChildren(header(s?.capture_error ? "Enregistrement interrompu" : s?.status === "review" ? "Démonstration arrêtée" : s?.status === "recording" ? "Enregistrement de la démonstration" : "Montrer au bot"));
    if (data.fake) dialog.append(node("p", "Mode de test : captures fictives et événements synthétiques. Aucune permission macOS ni entrée réelle.", "screen-test-note"));
    if (!s) {
      const bot = host.state.bots?.find(b => b.id === target);
      dialog.append(node("h3", bot?.name || "Choisissez un bot"), node("p", "Faites la tâche sur votre Mac pendant qu’Ezerbot observe l’écran principal, les clics, les touches de navigation et les changements d’application. Aucun contrôle du Mac n’est accordé."), node("p", "Une capture toutes les 5 secondes, pendant 10 minutes au plus. Aucune saisie de texte n’est enregistrée, même hors des mots de passe ; les captures sont suspendues dans les champs privés ou invérifiables. Fermez vos fenêtres privées avant de commencer."), node("p", "Après Arrêter, ajoutez une explication : jusqu’à 12 captures échantillonnées et la synthèse seront envoyées à votre fournisseur d’IA pour proposer une skill. Les captures sont supprimées après votre décision, sauf conservation explicite. Rien n’est enregistré comme skill sans Enregistrer."));
      if (!host.state.provider?.connected) { const notice = node("p", connectionRequired, "screen-error"); notice.setAttribute("role", "alert"); dialog.append(notice); }
      const label = node("label", null, "screen-consent"), check = node("input"); check.type = "checkbox"; check.name = "demonstration-consent";
      label.append(check, node("span", "J’autorise cet enregistrement et l’envoi au modèle pour préparer une proposition de skill."));
      const begin = button("Commencer la démonstration", start, "primary-button"); begin.disabled = true; check.onchange = () => { begin.disabled = !check.checked || busy || !bot || !host.state.provider?.connected; };
      dialog.append(label, begin);
    } else {
      if (s.notice) { const notice = node("p", s.notice, s.capture_error ? "screen-error" : "demonstration-note"); if (s.capture_error) notice.setAttribute("role", "alert"); dialog.append(notice); }
      if (s.status === "recording") {
        dialog.append(node("p", "Vous avez la main. Le bandeau reste visible pendant toute la démonstration."), button("Arrêter", stop, "primary-button"), button("Annuler et supprimer", discard));
        if (data.fake) dialog.append(button("Simuler une erreur de capture", () => action("/api/demonstrations/fake-error", { id: s.id })));
      } else if (s.status === "review") {
        const label = node("label", "Expliquez brièvement la tâche (facultatif)", "form-label"), input = node("textarea", null, "text-input"); input.rows = 3; input.maxLength = 1000; input.name = "demonstration-explanation"; input.value = explanation; input.oninput = () => { explanation = input.value; }; label.append(input);
        const media = node("label", null, "screen-consent"), check = node("input"); check.type = "checkbox"; check.name = "demonstration-keep"; check.checked = keep; check.onchange = () => { keep = check.checked; }; media.append(check, node("span", "Conserver aussi les captures localement si j’enregistre la skill. Sinon, ne garder que le lien vers la synthèse."));
        dialog.append(label, media, button("Voir la synthèse", () => showSummary(s.id)), button("Créer la proposition de skill", draft, "primary-button"), button("Ignorer et supprimer", discard));
      } else dialog.append(node("p", "Le bot prépare la carte Enregistrer / Ignorer dans la conversation. Enregistrer conserve une synthèse liée à la skill, sans accorder de permission d’exécution."), button(s.request_id ? "Voir la proposition" : "Ouvrir le fil", openCard), button("Annuler et supprimer", discard));
    }
    if (busy) dialog.append(node("p", "Opération en cours…", "skills-note"));
    if (error && (host.state.provider?.connected || error !== connectionRequired)) { const alert = node("p", error, "screen-error"); alert.setAttribute("role", "alert"); dialog.append(alert); }
    for (const control of dialog.querySelectorAll("button, textarea, input")) if (busy) control.disabled = true;
    focusControl(preferred);
  }
  function summaryHeader(title = "Synthèse de la démonstration") {
    const header = node("div", null, "dialog-header"); header.append(node("h2", title), button("Fermer", () => summaryDialog.close())); return header;
  }
  async function showSummary(id) {
    summaryDialog.replaceChildren(summaryHeader(), node("p", "Chargement…"));
    if (!summaryDialog.open) summaryDialog.showModal();
    try {
      const { demonstration: d, images } = await host.api(`/api/demonstrations/summary?id=${encodeURIComponent(id)}`);
      const view = window.GrokbotChat.demonstrationSummaryView(d, { id: data.session?.status === "review" ? data.session.id : null, explanation, keepImages: keep });
      summaryDialog.replaceChildren(summaryHeader(), node("p", view.explanation), node("pre", view.summary, "demonstration-summary"), node("p", view.media));
      for (const image of images || []) { const img = node("img"); img.src = image; img.alt = "Capture conservée de la démonstration"; img.width = 1280; img.height = 800; img.loading = "lazy"; summaryDialog.append(img); }
    } catch (e) { summaryDialog.replaceChildren(summaryHeader("Synthèse indisponible"), node("p", e.message)); }
  }
  // Escape, like Fermer, only closes the controls. Only the explicit destructive
  // buttons may discard the recording; native recording continues visibly.
  const acknowledgedNotices = new Set();
  // Update text only: rebuilding Arrêter each second would lose keyboard focus.
  setInterval(() => { const count = overlay.querySelector("[data-demo-countdown]"); if (count && data.session) count.textContent = remaining(data.session); }, 1000);
  window.EzerbotDemonstration = { open, showSummary };
  host.register({ id: "demonstrations", onState(state, ctx) {
    const previous = lastSession; data = state.demonstrations || {}; context = ctx; lastSession = data.session;
    const notice = data.pending_notice;
    if (notice && !acknowledgedNotices.has(notice.id)) {
      acknowledgedNotices.add(notice.id); host.notify(notice.text);
      host.api("/api/demonstrations/acknowledge", { id: notice.id }).catch(() => {});
    }
    updateOverlay(); if (dialog.open) render();
    if (previous?.status === "recording" && data.session?.status === "review") open();
    if (previous && !data.session && data.notice) { if (dialog.open) dialog.close(); host.notify(data.notice); }
  }, wantsPolling() { return Boolean(data.session); } });
})();
