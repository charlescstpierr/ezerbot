"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, text, className) => { const el = document.createElement(tag); if (text) el.textContent = text; if (className) el.className = className; return el; };
  const kinds = { reply: "Réponses terminées", approval: "Autorisations demandées", form: "Formulaires en attente", secret: "Secrets demandés", routine: "Routines terminées ou en échec", delegation: "Sous-tâches terminées" };
  let current = null, data = null, loading = false, section, message, nativeFocus = null, pendingOpen = null;
  let permission = "unknown", error = "", saving = false;
  let listSignature = "", settingsSignature = "";
  const bridge = () => window.webkit?.messageHandlers?.ezerbotNotifications;
  async function native(command, fields = {}) {
    if (!bridge()) return { status: "unavailable" };
    return await bridge().postMessage({ command, ...fields });
  }
  const button = node("button", "Notifications", "tool-pill notifications-button");
  button.type = "button";
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
    button.replaceChildren(node("span", "Notifications"), badge);
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
        if (key === "enabled" && settings.enabled) {
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
    const signature = JSON.stringify([data?.settings, permission, error, saving]);
    if (signature === settingsSignature) return;
    settingsSignature = signature;
    section.replaceChildren(node("h3", "Notifications"));
    const warning = permission === "denied" || Boolean(error);
    message = node("p", null, warning ? "notification-warning" : ""); message.role = warning ? "alert" : "status";
    if (warning) {
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("aria-hidden", "true");
      svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor"); svg.setAttribute("stroke-width", "2");
      const path = document.createElementNS(svg.namespaceURI, "path"); path.setAttribute("d", "M12 3 2 21h20L12 3Z M12 9v4 M12 17v1"); svg.append(path); message.append(svg);
    }
    message.append(node("span", statusText())); section.append(message);
    section.append(toggle("enabled", "Recevoir les alertes macOS"));
    section.append(toggle("dnd", "Ne pas déranger", "Suspend les alertes ; les événements restent dans la liste des non lus."));
    for (const [key, label] of Object.entries(kinds)) section.append(toggle(key, label));
  }
  function presence() {
    return { focused: (nativeFocus ?? document.hasFocus()) && document.visibilityState === "visible", visible: Boolean(current) && !document.querySelector("dialog[open]"), conversation_id: current?.id || null };
  }
  async function open(id) {
    try {
      const target = await host.api("/api/notifications/open", { id });
      document.querySelectorAll("dialog[open]").forEach(el => el.close());
      pendingOpen = target;
      await host.refresh();
      focusTarget();
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
      data = await host.api("/api/notifications/poll", p0);
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
    } catch (e) { error = e.message; renderSettings(); }
    finally { loading = false; }
  }
  window.EzerbotNotifications = {
    activate: id => open(id),
    focus: value => {
      nativeFocus = Boolean(value);
      if (nativeFocus) native("status").then(result => { permission = result.status; renderSettings(); }).catch(() => {});
      poll();
    },
  };
  host.register({
    id: "notifications",
    settingsSection(container) { section = container; section.className = "setting-section notifications-settings"; renderSettings(); },
    onState(_, ctx) { current = ctx.conversation; },
  });
  for (const name of ["focus", "blur", "visibilitychange"]) window.addEventListener(name, () => poll());
  document.addEventListener("visibilitychange", poll);
  native("status").then(result => { permission = result.status; renderSettings(); }).catch(() => {});
  // Independent of model streaming: routines and delegated work may finish while idle.
  setInterval(poll, 1000);
  poll();
})();
