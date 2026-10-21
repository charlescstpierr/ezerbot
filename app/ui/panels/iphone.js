"use strict";
(() => {
  const host = window.GrokbotPanels;
  const remote = window.EzerbotIPhone?.remote;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  let section, data, tailscale, code, expires, confirmation, context;
  let message = "", messageArea = "devices", codeNotice = "", busy = false, signature = "", failed = false, pushEnabled = null;
  let codeDevices = new Set();
  let pendingNotification = remote ? new URL(location.href).searchParams.get("notification") : null;
  const badge = node("button", "iPhone connecté", "tool-pill iphone-presence"); badge.hidden = true;
  if (!remote) {
    document.querySelector(".topbar-actions").prepend(badge);
    badge.onclick = () => { document.getElementById("settings-dialog").showModal(); section.scrollIntoView({ block: "start" }); };
  }
  function button(text, action, cls = "secondary-button", area = "devices") {
    const b = node("button", text, cls); b.type = "button"; b.disabled = busy;
    b.onclick = () => run(action, area); return b;
  }
  async function run(action, area = "devices") {
    if (busy) return;
    busy = true; message = ""; messageArea = area; failed = false; render();
    try { await action(); } catch (e) { message = e.message; failed = true; }
    finally { busy = false; signature = ""; render(); }
  }
  // In-page confirmation: window.confirm() silently returns false in the Mac
  // WKWebView without a confirm-panel delegate. No effect runs before this card.
  function askButton(text, title, description, label, action, area, danger = false) {
    const b = button(text, () => {}, danger ? "danger-button" : "secondary-button");
    b.onclick = () => {
      if (busy) return;
      confirmation = { title, description, label, action, area, danger, origin: text };
      message = ""; signature = ""; render();
      section.querySelector(".iphone-confirm-cancel")?.focus({ preventScroll: true });
      section.querySelector(".iphone-confirmation")?.scrollIntoView({ block: "nearest" });
    };
    return b;
  }
  function cancelConfirmation() {
    const origin = confirmation?.origin; confirmation = null; signature = ""; render();
    [...section.querySelectorAll("button")].find(b => b.textContent === origin)?.focus({ preventScroll: true });
  }
  function appendConfirmation(target, area) {
    if (confirmation?.area !== area) return;
    const current = confirmation;
    const card = node("section", null, "iphone-confirmation"); card.setAttribute("role", "group"); card.setAttribute("aria-label", current.title);
    const actions = node("div", null, "iphone-actions");
    const cancel = button("Annuler", () => {}, "secondary-button iphone-confirm-cancel"); cancel.onclick = cancelConfirmation;
    actions.append(cancel, button(current.label, async () => {
      confirmation = null;
      await current.action();
    }, current.danger ? "danger-button" : "primary-button", area === "tailscale" ? area : "devices"));
    card.append(node("h4", current.title), node("p", current.description), actions); target.append(card);
  }
  function appendNotice(target, area) {
    if (!message || messageArea !== area) return;
    const text = node("p", message, failed ? "iphone-warning" : "iphone-notice");
    text.role = failed ? "alert" : "status"; target.append(text);
  }
  function expireCode() {
    if (!code) return;
    if (data?.devices?.some(d => !codeDevices.has(d.id))) {
      code = null; expires = null; codeNotice = "Code utilisé — vérifiez l’appareil ci-dessous.";
    } else if (Date.now() >= expires) {
      code = null; expires = null; codeNotice = "Code expiré — créez-en un nouveau.";
    }
  }
  async function update() {
    if (remote) return;
    expireCode(); render();
    if (!host.connected || busy) return;
    try { data = await host.api("/api/iphone/status"); expireCode(); render(); }
    catch { /* main shell owns the offline banner */ }
  }
  async function set(path, body) { data = await host.api(path, body); await host.refresh(); }
  async function serve(start) {
    tailscale = await host.api("/api/iphone/serve", { start });
    if (tailscale.error) throw new Error(tailscale.error);
    message = start ? "Publication privée activée. Ouvrez l’adresse HTTPS sur l’iPhone." : "Service HTTPS Tailscale arrêté.";
  }
  function render() {
    if (!section) return;
    expireCode();
    const connected = data?.enabled && data?.devices?.filter(d => d.connected && d.status === "active");
    badge.hidden = !connected?.length;
    badge.textContent = connected?.length > 1 ? `${connected.length} iPhone connectés` : "iPhone connecté";
    badge.setAttribute("aria-label", badge.textContent); badge.title = badge.textContent;
    // Presence polling must not replace the focused confirmation controls.
    const next = JSON.stringify(confirmation ? [confirmation.title, busy] : [data, tailscale, code, codeNotice, message, busy, pushEnabled]);
    if (next === signature) return; signature = next;
    section.replaceChildren(node("h3", "iPhone"));
    if (remote) { renderPhone(); return; }
    section.append(node("p", "Le Mac reste le moteur. Accès privé et chiffré par Tailscale, sans exposition publique. Chaque appareil doit être confirmé ici."));
    const label = node("label", null, "setting-toggle"); const input = node("input");
    input.type = "checkbox"; input.checked = !!data?.enabled; input.disabled = busy || !data; input.role = "switch";
    input.onchange = () => run(async () => { code = null; expires = null; codeNotice = ""; await set("/api/iphone/enabled", { enabled: input.checked }); });
    const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true");
    label.append(node("span", "Activer l’accès depuis l’iPhone"), input, track); section.append(label);
    if (data?.enabled) {
      if (data.listener_error) section.append(node("p", data.listener_error, "iphone-warning"));
      const steps = node("ol", null, "iphone-steps");
      for (const text of [
        "Installez Tailscale sur le Mac et l’iPhone et connectez-les au même compte. Activez MagicDNS et HTTPS dans votre tailnet si Tailscale le demande.",
        "Sur le Mac, vérifiez Tailscale ci-dessous. Publiez uniquement le port privé d’Ezerbot (jamais Tailscale Funnel).",
        "Ouvrez l’adresse HTTPS ci-dessous dans Safari sur l’iPhone. Choisissez Partager › Sur l’écran d’accueil › Ajouter, puis ouvrez Ezerbot depuis son icône.",
        "Dans l’app installée : créez un code sur le Mac, saisissez-le sur l’iPhone, puis confirmez la demande ici. Si Safari et l’app ne partagent pas leur session, associez l’app séparément. La dictée utilise le micro du clavier iOS.",
      ]) steps.append(node("li", text));
      section.append(steps, button("Vérifier Tailscale", async () => { tailscale = await host.api("/api/iphone/tailscale"); }, "secondary-button", "tailscale"));
      if (tailscale) {
        section.append(node("p", tailscale.error || `${tailscale.running ? "Tailscale connecté" : "Tailscale arrêté"} · ${tailscale.tailnet || "Tailnet inconnu"}`, tailscale.error ? "iphone-warning" : ""));
        if (tailscale.url) section.append(node("p", tailscale.url, "iphone-address"));
      }
      section.append(node("pre", `tailscale serve --bg http://127.0.0.1:${data.port}\n# Pour arrêter uniquement le service HTTPS :\ntailscale serve --https=443 off`, "iphone-commands"));
      section.append(node("p", "Ces boutons modifient le service HTTPS 443 de Tailscale sur ce Mac. Si vous l’utilisez déjà pour une autre app, faites la configuration manuellement pour éviter de le remplacer. Désactiver l’accès bloque toutes les requêtes ; le port reste uniquement en boucle locale jusqu’à la fermeture d’Ezerbot. Cela ne modifie pas Tailscale."));
      const actions = node("div", null, "iphone-actions");
      actions.append(
        askButton("Publier Ezerbot avec Tailscale", "Publier Ezerbot sur le HTTPS 443 privé ?", "Cela peut remplacer un service Tailscale existant sur ce Mac. Aucun accès public ne sera activé.", "Publier", () => serve(true), "tailscale"),
        askButton("Arrêter le service HTTPS Tailscale", "Arrêter le service HTTPS 443 ?", "Le service HTTPS privé de ce Mac sera arrêté, y compris les autres apps qui l’utilisent.", "Arrêter", () => serve(false), "tailscale", true),
      );
      section.append(actions); appendConfirmation(section, "tailscale"); appendNotice(section, "tailscale");
      section.append(button("Créer un code d’association", async () => {
        const value = await host.api("/api/iphone/code", {}); code = value.code; expires = value.expires; codeNotice = "";
        codeDevices = new Set((data?.devices || []).map(d => d.id));
      }));
      if (code) {
        section.append(node("p", code.match(/.{1,4}/g).join(" "), "iphone-code"), node("p", `Code à usage unique · expire à ${new Date(expires).toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" })}. Ne le partagez pas.`));
      } else if (codeNotice) section.append(node("p", codeNotice, "iphone-code-status"));
    }
    section.append(node("h4", "Appareils associés"));
    if (!data?.devices?.length) section.append(node("p", "Aucun appareil associé."));
    for (const device of data?.devices || []) {
      const row = node("div", null, "iphone-device"); const copy = node("div"); copy.append(node("strong", device.name));
      let activity = "Dernière activité : jamais";
      if (device.status === "pending") activity = "Demande reçue · confirmez uniquement l’appareil que vous venez d’associer";
      else if (device.status === "confirmed") activity = "Confirmation envoyée · en attente de l’iPhone";
      else if (device.connected && data.enabled) activity = "Connecté maintenant";
      else if (device.last_seen) activity = `Dernière activité : ${new Date(device.last_seen).toLocaleString("fr-CA", { dateStyle: "medium", timeStyle: "short" })}`;
      copy.append(node("small", activity));
      if (device.push_error) copy.append(node("small", device.push_error, "iphone-warning")); row.append(copy);
      const actions = node("div", null, "iphone-actions");
      if (device.status === "pending") {
        actions.append(button("Confirmer", () => set("/api/iphone/confirm", { id: device.id, allow: true }), "primary-button"), button("Refuser", () => set("/api/iphone/confirm", { id: device.id, allow: false })));
      } else {
        actions.append(askButton("Révoquer", `Révoquer « ${device.name} » ?`, "Ses accès et notifications seront immédiatement retirés. Il faudra l’associer de nouveau pour les rétablir.", "Révoquer", async () => { await set("/api/iphone/revoke", { id: device.id }); message = "Appareil révoqué."; }, `device:${device.id}`, true));
      }
      row.append(actions); appendConfirmation(row, `device:${device.id}`); section.append(row);
    }
    appendNotice(section, "devices");
  }
  function renderPhone() {
    section.append(node("p", "Le Mac doit rester allumé, Ezerbot ouvert et Tailscale connecté sur les deux appareils. Les approbations et les autorisations sont identiques à celles du Mac."));
    section.append(node("p", "Pour installer : Safari › Partager › Sur l’écran d’accueil › Ajouter. Pour dicter : utilisez le micro du clavier iOS."));
    const status = node("p", pushEnabled === null ? "Vérification des notifications de cet iPhone…" : pushEnabled ? "Notifications activées sur cet iPhone" : "Notifications désactivées sur cet iPhone", pushEnabled ? "iphone-notice" : "");
    status.role = "status"; section.append(status);
    if (pushEnabled === null) section.append(button("Vérifier les notifications", () => presence(true), "secondary-button", "phone"));
    else if (pushEnabled) section.append(button("Désactiver les notifications sur cet iPhone", disablePush, "secondary-button", "phone"));
    else section.append(button("Activer les notifications sur cet iPhone", enablePush, "secondary-button", "phone"));
    section.append(node("p", "iOS 16.4 ou plus récent : ouvrez l’app depuis l’écran d’accueil. Les réglages par type et Ne pas déranger ci-dessous s’appliquent aussi à cet iPhone. Aucun push pour le fil ouvert."));
    appendNotice(section, "phone");
  }
  async function disablePush() {
    await host.api("/api/iphone/push", { subscription: null }); pushEnabled = false;
    try { const registration = await navigator.serviceWorker?.getRegistration(); const sub = await registration?.pushManager.getSubscription(); await sub?.unsubscribe(); }
    catch { throw new Error("Les envois du Mac sont désactivés. iOS n’a pas pu retirer l’abonnement local ; réessayez après avoir rouvert l’app."); }
    message = "Notifications désactivées sur cet appareil.";
  }
  async function enablePush() {
    if (!matchMedia("(display-mode: standalone)").matches && !navigator.standalone) throw new Error("Ajoutez Ezerbot à l’écran d’accueil, puis ouvrez-le depuis son icône pour autoriser les notifications.");
    if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) throw new Error("Ce navigateur ne prend pas en charge les notifications Web Push.");
    let permission;
    try { permission = await Notification.requestPermission(); }
    catch { throw new Error("iOS n’a pas pu demander l’autorisation. Ouvrez Ezerbot depuis l’écran d’accueil et réessayez."); }
    if (permission !== "granted") throw new Error("Notifications non autorisées. Vous pouvez les autoriser dans les réglages iOS de cette app.");
    let registration, timeout;
    try { registration = await Promise.race([navigator.serviceWorker.ready, new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error("L’installation de l’app n’est pas prête. Vérifiez la connexion au Mac, fermez puis rouvrez Ezerbot.")), 8000); })]); }
    finally { clearTimeout(timeout); }
    const { key } = await host.api("/api/iphone/push-key");
    const bytes = Uint8Array.from(atob(key.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - key.length % 4) % 4)), c => c.charCodeAt(0));
    let subscription;
    try { subscription = await registration.pushManager.getSubscription() || await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: bytes }); }
    catch { throw new Error("L’abonnement aux notifications a échoué. Vérifiez l’autorisation iOS et votre connexion, puis réessayez."); }
    await host.api("/api/iphone/push", { subscription: subscription.toJSON() }); pushEnabled = true;
    const notifications = await host.api("/api/notifications");
    if (!notifications.settings.enabled) await host.api("/api/notifications/settings", { ...notifications.settings, enabled: true });
    message = "Notifications autorisées sur cet iPhone. Vérifiez Ne pas déranger et les types souhaités ci-dessous.";
  }
  async function presence(force = false) {
    if (!remote || !host.connected) return;
    try {
      const value = await host.api("/api/iphone/presence", { conversation_id: context?.conversation?.id || null, visible: document.visibilityState === "visible" && !document.querySelector("dialog[open]") });
      if ((force === true || !busy) && typeof value.push === "boolean" && value.push !== pushEnabled) { pushEnabled = value.push; render(); }
    } catch { /* shell shows recovery */ }
  }
  host.register({
    id: "iphone",
    settingsSection(container) {
      section = container; section.id = "settings-iphone"; section.className = "setting-section iphone-settings";
      section.addEventListener("keydown", event => { if (event.key === "Escape" && confirmation && !busy) { event.preventDefault(); event.stopPropagation(); cancelConfirmation(); } });
      render();
    },
    onState(state, ctx) {
      context = ctx; if (!remote && state.iphone) data = state.iphone; render(); presence();
      if (pendingNotification && ctx.connected) {
        const id = pendingNotification; pendingNotification = null; history.replaceState(null, "", "/");
        queueMicrotask(() => window.EzerbotNotifications?.activate(id));
      }
    },
  });
  setInterval(remote ? presence : update, remote ? 5000 : 2000);
  document.addEventListener("visibilitychange", presence);
})();
