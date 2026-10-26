"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, text, className) => { const el = document.createElement(tag); if (text) el.textContent = text; if (className) el.className = className; return el; };
  const kinds = { reply: "Réponses terminées", approval: "Autorisations demandées", form: "Formulaires en attente", secret: "Secrets demandés", routine: "Routines terminées ou en échec", delegation: "Sous-tâches terminées" };
  let current = null, data = null, loading = false, section, message, nativeFocus = null, pendingOpen = null;
  let permission = "unknown", error = "", pollFailure = "", saving = false;
  // A tap on a push while the app was closed opens /?notification=…[&reply=1].
  let launch = null;
  try {
    const query = new URLSearchParams(window.location?.search || "");
    if (query.get("notification")) { launch = { id: query.get("notification"), reply: query.get("reply") === "1" }; window.history?.replaceState(null, "", window.location.pathname); }
  } catch { /* no launch destination */ }
  let listSignature = "", settingsSignature = "";
  const bridge = () => window.webkit?.messageHandlers?.ezerbotNotifications;
  async function native(command, fields = {}) {
    if (!bridge()) return { status: "unavailable" };
    return await bridge().postMessage({ command, ...fields });
  }
  const button = node("button", null, "tool-pill notifications-button");
  button.type = "button";
  // Sous 1100 px seule la cloche reste dans l'en-tête ; le nom accessible garde le libellé.
  const bell = node("span", null, "notifications-icon"); bell.setAttribute("aria-hidden", "true");
  bell.innerHTML = '<svg class="icon"><use href="#i-bell"/></svg>';
  const caption = node("span", "Notifications", "notifications-label");
  button.append(bell, caption);
  button.setAttribute("aria-label", "Notifications");
  button.setAttribute("aria-haspopup", "dialog");
  document.querySelector(".topbar-actions").prepend(button);
  const dialog = node("dialog", null, "dialog notifications-dialog");
  dialog.setAttribute("aria-labelledby", "notifications-title");
  const header = node("div", null, "dialog-header");
  const title = node("h2", "Notifications"); title.id = "notifications-title";
  const close = node("button", "Fermer", "secondary-button"); close.onclick = () => dialog.close();
  header.append(title, close);
  const hint = node("p", "Vos événements non lus, conservés sur ce Mac.", "notification-hint");
  const list = node("div", null, "notification-list");
  const footer = node("div", null, "dialog-footer");
  const all = node("button", "Tout marquer comme lu", "secondary-button");
  all.onclick = async () => { try { await host.api("/api/notifications/read", { ids: data.items.map(item => item.id) }); await poll(); } catch (e) { host.notify(e.message); } };
  footer.append(all); dialog.append(header, hint, list, footer); document.getElementById("panels").append(dialog);
  button.onclick = () => { renderList(); dialog.showModal(); };
  dialog.addEventListener("click", event => { if (event.target === dialog) { const r = dialog.getBoundingClientRect(); if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) dialog.close(); } });
  function renderList() {
    const signature = JSON.stringify([data?.unread, data?.items, data?.settings?.dnd]);
    if (signature === listSignature) return;
    listSignature = signature;
    const unread = data?.unread || 0;
    const badge = node("span", unread > 999 ? "999+" : String(unread), "notification-count");
    badge.hidden = !unread; badge.setAttribute("aria-hidden", "true");
    button.replaceChildren(bell, caption, badge);
    button.classList.toggle("has-unread", unread > 0);
    const paused = data?.settings?.dnd;
    button.title = paused ? "Ne pas déranger : alertes suspendues" : "Consulter les événements non lus";
    button.setAttribute("aria-label", `Notifications, ${unread} non lue${unread > 1 ? "s" : ""}${paused ? ", Ne pas déranger actif" : ""}`);
    list.replaceChildren();
    footer.hidden = !data?.items?.length;
    all.disabled = !data?.items?.length;
    all.textContent = unread > 100 ? "Marquer cette liste comme lue" : "Tout marquer comme lu";
    if (!data?.items?.length) list.append(node("p", "Tout est à jour. Aucune notification non lue.", "notification-empty"));
    for (const item of data?.items || []) {
      const row = node("button", null, "notification-item");
      row.type = "button";
      const heading = node("div", null, "notification-item-heading");
      if (["approval", "form", "secret"].includes(item.kind)) heading.append(node("span", "Action requise", "notification-action-required"));
      heading.append(node("strong", item.title));
      const date = new Date(item.created_at), now = new Date(), yesterday = new Date();
      yesterday.setDate(now.getDate() - 1);
      const hour = date.toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" });
      const when = date.toDateString() === now.toDateString() ? hour : date.toDateString() === yesterday.toDateString() ? `Hier à ${hour}`
        : `${date.toLocaleDateString("fr-CA", { day: "numeric", month: "short", ...(date.getFullYear() !== now.getFullYear() ? { year: "numeric" } : {}) })} · ${hour}`;
      const time = node("time", when); time.dateTime = date.toISOString(); time.title = date.toLocaleString("fr-CA");
      row.append(heading, node("span", item.conversation_title), time); row.onclick = () => open(item.id); list.append(row);
    }
    if (unread > 100) list.append(node("p", "Les 100 événements les plus récents sont affichés. Marquez-les comme lus pour voir les suivants.", "notification-hint"));
  }
  function statusText() {
    if (error) return error;
    if (pollFailure) return pollFailure;
    if (window.EzerbotIPhone?.remote) return "Les types et Ne pas déranger s’appliquent au Mac et à l’iPhone. Activez les notifications de cet appareil dans la section iPhone.";
    if (permission === "denied") return "Notifications refusées par macOS. Autorisez Ezerbot dans Réglages Système › Notifications, puis réessayez.";
    if (data?.settings?.dnd) return "Ne pas déranger est actif : les alertes sont suspendues, les événements restent dans la liste.";
    if (permission === "recording") return "Mode de test : notifications enregistrées, sans alerte système ni demande de permission.";
    if (permission === "disabled") return "Version de développement : les alertes système sont désactivées.";
    if (permission === "unavailable") return "Les alertes système sont disponibles dans l’application macOS. La liste des événements reste disponible ici.";
    return data?.settings?.enabled ? "Alertes actives hors du fil visible et focalisé. Aucun contenu de message ou secret n’est affiché dans macOS." : "Activez les alertes pour les recevoir hors du fil visible. macOS vous demandera votre autorisation au premier essai.";
  }
  function toggle(key, label, detail) {
    const row = node("label", null, "setting-toggle notification-toggle");
    row.classList.toggle("is-muted", key in kinds && (!data?.settings?.enabled || data?.settings?.dnd));
    const copy = node("span"); copy.append(node("strong", label)); if (detail) copy.append(node("small", detail));
    const input = node("input"); input.type = "checkbox"; input.role = "switch"; input.checked = Boolean(data?.settings?.[key]); input.disabled = saving || !data;
    input.dataset.notificationSetting = key;
    const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true");
    input.onchange = async () => {
      const settings = { ...data.settings, [key]: input.checked };
      saving = true; error = ""; renderSettings();
      try {
        if (key === "enabled" && settings.enabled && !window.EzerbotIPhone?.remote) {
          const result = await native("authorize"); permission = result.status;
          if (!["granted", "recording"].includes(permission)) { settings.enabled = false; if (permission === "error") error = "macOS n’a pas pu activer les notifications. Réessayez."; }
        }
        data = await host.api("/api/notifications/settings", settings);
      } catch (e) { error = e.message; }
      finally { saving = false; renderSettings(); renderList(); }
    };
    row.append(copy, input, track); return row;
  }
  function renderSettings() {
    if (!section) return;
    const signature = JSON.stringify([data?.settings, permission, error, pollFailure, saving]);
    if (signature === settingsSignature) return;
    settingsSignature = signature;
    section.replaceChildren(node("h3", "Notifications"));
    const warning = permission === "denied" || Boolean(error || pollFailure);
    message = node("p", null, warning ? "notification-warning" : ""); message.role = warning ? "alert" : "status";
    if (warning) {
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("aria-hidden", "true");
      svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "2");
      const path = document.createElementNS(svg.namespaceURI, "path"); path.setAttribute("d", "M12 3 2 21h20L12 3Z M12 9v4 M12 17v1"); svg.append(path); message.append(svg);
    }
    message.append(node("span", statusText())); section.append(message);
    section.append(toggle("enabled", window.EzerbotIPhone?.remote ? "Recevoir les alertes" : "Recevoir les alertes macOS et iPhone"));
    section.append(toggle("dnd", "Ne pas déranger", "Suspend les alertes ; les événements restent dans la liste des non lus."));
    for (const [key, label] of Object.entries(kinds)) section.append(toggle(key, label));
  }
  // Répondre: a sheet on the conversation, field ready. One message id per conversation
  // and text until it is confirmed, so a second tap on Envoyer, a retry after a lost
  // answer, or a reopened sheet never posts the same words twice.
  let reply = null, offer = null;
  const unconfirmed = new Map();
  const MAC_UNREACHABLE = "Le Mac est injoignable. Ce message n’est pas marqué envoyé et n’est pas mis en file ; s’il est déjà arrivé, il apparaîtra dans le fil. Vérifiez que le Mac est allumé avec Ezerbot ouvert, puis réessayez avec le même texte : il ne partira pas deux fois.";
  function botName(conversationId) {
    const state = host.state, conversation = state?.conversations?.find(item => item.id === conversationId);
    return state?.bots?.find(bot => bot.id === conversation?.bot_id)?.name || "";
  }
  function replyStatus(sheet, text, failed = false) {
    sheet.status.textContent = text; sheet.status.className = failed ? "notification-warning" : "notification-hint"; sheet.status.role = failed ? "alert" : "status";
  }
  function dismissOffer() { offer?.remove?.(); offer = null; }
  // iOS shows no action button on a web notification: a plain tap lands on the
  // conversation, where this control opens the same sheet.
  function offerReply(conversationId) {
    dismissOffer();
    const name = botName(conversationId);
    const bar = node("div", null, "reply-offer"); bar.role = "status";
    const open = node("button", name ? `Répondre à ${name}` : "Répondre", "primary-button"); open.type = "button";
    open.onclick = () => { dismissOffer(); openReply(conversationId); };
    bar.append(open); offer = bar; bar.conversationId = conversationId;
    document.getElementById("panels").append(bar);
    setTimeout(() => { if (offer === bar) dismissOffer(); }, 30000);
  }
  function openReply(conversationId) {
    reply?.dialog.remove?.();
    const name = botName(conversationId);
    const dialog = node("dialog", null, "dialog reply-sheet"); dialog.setAttribute("aria-labelledby", "reply-title");
    const heading = node("div", null, "dialog-header");
    const titleNode = node("h2", name ? `Répondre à ${name}` : "Répondre"); titleNode.id = "reply-title";
    const dismiss = node("button", "Fermer", "secondary-button"); dismiss.type = "button"; dismiss.onclick = () => dialog.close();
    heading.append(titleNode, dismiss);
    const form = node("form", null, "reply-form");
    const area = node("textarea", null, "text-input reply-input"); area.setAttribute("aria-label", "Votre réponse"); area.placeholder = "Votre réponse"; area.rows = 4; area.maxLength = 20000; area.enterKeyHint = "send";
    const status = node("p", null, "notification-hint"); status.role = "status";
    const send = node("button", "Envoyer", "primary-button"); send.type = "submit";
    form.append(area, status, send); dialog.append(heading, form);
    const sheet = { dialog, status, sending: false };
    reply = sheet;
    form.onsubmit = async event => {
      event.preventDefault();
      const body = area.value.trim(), key = `${conversationId}\n${body}`;
      if (!body || sheet.sending) return;
      if (!unconfirmed.has(key)) unconfirmed.set(key, crypto.randomUUID());
      sheet.sending = true; send.disabled = true; area.readOnly = true; replyStatus(sheet, "Envoi en cours…");
      try {
        await host.api("/api/send", { conversation_id: conversationId, message_id: unconfirmed.get(key), body });
        unconfirmed.delete(key);
        await host.refresh().catch(() => {});
        dialog.close();
      } catch (e) {
        // A refusal from the Mac says why; a missing answer leaves the outcome unconfirmed.
        if (e.status) unconfirmed.delete(key);
        replyStatus(sheet, e.status ? e.message : MAC_UNREACHABLE, true); send.disabled = false; area.readOnly = false;
        if (!e.status) host.refresh().catch(() => {});
      } finally { sheet.sending = false; }
    };
    document.getElementById("panels").append(dialog);
    dialog.showModal(); area.focus();
  }
  function presence() {
    return { focused: (nativeFocus ?? document.hasFocus()) && document.visibilityState === "visible", visible: Boolean(current) && !document.querySelector("dialog[open]"), conversation_id: current?.id || null };
  }
  async function open(id, options = {}) {
    try {
      const target = await host.api("/api/notifications/open", { id });
      document.querySelectorAll("dialog[open]").forEach(el => el.close());
      pendingOpen = target;
      await host.refresh();
      focusTarget();
      if (options.reply) openReply(target.conversation_id);
      else if (options.push && id.startsWith("reply:")) offerReply(target.conversation_id);
    } catch (e) { host.notify(e.message); }
  }
  function focusTarget() {
    if (!pendingOpen || !host.openConversation(pendingOpen.conversation_id)) return;
    const target = pendingOpen; pendingOpen = null;
    // A scheduler failure before run creation has no transcript card to invent.
    if (!target.card_id) return;
    const runId = target.id.startsWith("reply:") ? target.id.slice(6)
      : target.id.startsWith("routine:") && target.card_id.startsWith("reply-") ? target.card_id.slice(6) : null;
    // React commits after the vanilla shell. Retry briefly, never steal scroll on every poll.
    let attempts = 0;
    function reveal() {
      if (current?.id !== target.conversation_id) return;
      // Some task snapshots intentionally render nothing. Do not focus their
      // empty slots instead of the real task/error card belonging to the run.
      const nodes = [...document.querySelectorAll("[data-notification-card], [data-message-id], [data-run-id]")].filter(el => el.getClientRects().length > 0);
      const found = nodes.find(el => el.dataset.notificationCard === target.card_id || el.dataset.messageId === target.card_id)
        || (runId && nodes.findLast(el => el.dataset.runId === runId));
      if (!found && ++attempts < 20) { setTimeout(reveal, 50); return; }
      if (found) {
        const viewport = document.getElementById("chat-scroll");
        const tall = viewport && found.getBoundingClientRect().height + 24 > viewport.clientHeight;
        found.scrollIntoView({ block: tall ? "start" : "center", behavior: "instant" });
        found.tabIndex = -1; found.focus({ preventScroll: true }); found.classList.add("notification-target");
        setTimeout(() => found.classList.remove("notification-target"), 3000);
      }
    }
    requestAnimationFrame(reveal);
    // WebKit suspends animation frames while macOS is locked. Keep the exact
    // destination ready even then; the frame callback restores it on activation.
    setTimeout(reveal, 80);
  }
  async function poll() {
    if (loading || saving) return;
    loading = true;
    try {
      const p0 = presence();
      await native("presence", { conversation_id: p0.conversation_id, visible: p0.visible });
      data = window.EzerbotIPhone?.remote ? await host.api("/api/notifications") : await host.api("/api/notifications/poll", p0);
      pollFailure = "";
      renderList();
      if (!section?.querySelector("input:focus")) renderSettings();
      for (const item of data.deliveries || []) {
        try {
          const result = await native("deliver", { event: item });
          if (["denied", "error"].includes(result.status)) { permission = result.status; error = result.status === "error" ? "Une alerte n’a pas pu être envoyée. L’événement reste dans la liste." : ""; renderSettings(); }
        } catch { error = "Le service de notifications est indisponible. Vos événements restent dans la liste."; renderSettings(); }
      }
      // Reading only events present in this snapshot avoids consuming a new, unseen event.
      const p = presence();
      const visible = p.focused && p.visible ? data.items.filter(item => item.conversation_id === p.conversation_id).map(item => item.id) : [];
      if (visible.length) { data = await host.api("/api/notifications/read", { ids: visible }); renderList(); }
      focusTarget();
      if (launch) { const { id, reply: wantsReply } = launch; launch = null; open(id, { reply: wantsReply, push: true }); }
    } catch (e) { pollFailure = e.message; renderSettings(); }
    finally { loading = false; }
  }
  window.EzerbotNotifications = {
    activate: (id, options) => open(id, options),
    focus: value => {
      nativeFocus = Boolean(value);
      if (nativeFocus) native("status").then(result => { permission = result.status; renderSettings(); }).catch(() => {});
      poll();
    },
  };
  host.register({
    id: "notifications",
    openSidebar() { button.click(); },
    settingsSection(container) { section = container; section.className = "setting-section notifications-settings"; renderSettings(); },
    onState(_, ctx) { current = ctx.conversation; if (offer && current?.id !== offer.conversationId) dismissOffer(); },
  });
  for (const name of ["focus", "blur", "visibilitychange"]) window.addEventListener(name, () => poll());
  document.addEventListener("visibilitychange", poll);
  native("status").then(result => { permission = result.status; renderSettings(); }).catch(() => {});
  // Independent of model streaming: routines and delegated work may finish while idle.
  setInterval(poll, 1000);
  poll();
})();
