"use strict";
(() => {
  const host = window.GrokbotPanels;
  const remote = window.EzerbotIPhone?.remote;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  let section, data, tailscale, code, expires, message = "", busy = false, signature = "", failed = false, context;
  let pendingNotification = remote ? new URL(location.href).searchParams.get("notification") : null;
  const badge = node("button", "iPhone connecté", "tool-pill iphone-presence"); badge.hidden = true;
  if (!remote) { document.querySelector(".topbar-actions").prepend(badge); badge.onclick = () => { document.getElementById("settings-dialog").showModal(); section.scrollIntoView({ block: "start" }); }; }
  function button(text, action, cls = "secondary-button") { const b = node("button", text, cls); b.type = "button"; b.disabled = busy; b.onclick = () => run(action); return b; }
  async function run(action) { if (busy) return; busy = true; message = ""; failed = false; render(); try { await action(); } catch (e) { message = e.message; failed = true; } finally { busy = false; signature = ""; render(); } }
  async function update() {
    if (remote || !host.connected || busy) return;
    try { data = await host.api("/api/iphone/status"); render(); } catch { /* main shell owns the offline banner */ }
  }
  async function set(path, body) { data = await host.api(path, body); await host.refresh(); }
  function render() {
    if (!section) return;
    const connected = data?.enabled && data?.devices?.filter(d => d.connected && d.status === "active");
    badge.hidden = !connected?.length; badge.textContent = connected?.length > 1 ? `${connected.length} iPhone connectés` : "iPhone connecté";
    const next = JSON.stringify([data, tailscale, code, message, busy]); if (next === signature) return; signature = next;
    section.replaceChildren(node("h3", "iPhone"));
    if (remote) { renderPhone(); return; }
    section.append(node("p", "Le Mac reste le moteur. Accès privé et chiffré par Tailscale, sans exposition publique. Chaque appareil doit être confirmé ici."));
    const label = node("label", null, "setting-toggle"); const copy = node("span", "Activer l’accès depuis l’iPhone"); const input = node("input"); input.type = "checkbox"; input.checked = !!data?.enabled; input.disabled = busy || !data; input.role = "switch";
    input.onchange = () => run(() => set("/api/iphone/enabled", { enabled: input.checked })); const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true"); label.append(copy, input, track); section.append(label);
    if (data?.enabled) {
      if (data.listener_error) section.append(node("p", data.listener_error, "iphone-warning"));
      const steps = node("ol", null, "iphone-steps");
      for (const text of ["Installez Tailscale sur le Mac et l’iPhone et connectez-les au même compte. Activez MagicDNS et HTTPS dans votre tailnet si Tailscale le demande.", "Sur le Mac, vérifiez Tailscale ci-dessous. Publiez uniquement le port privé d’Ezerbot (jamais Tailscale Funnel).", "Ouvrez l’adresse HTTPS ci-dessous dans Safari sur l’iPhone. Choisissez Partager › Sur l’écran d’accueil › Ajouter, puis ouvrez Ezerbot depuis son icône.", "Dans l’app installée : créez un code sur le Mac, saisissez-le sur l’iPhone, puis confirmez la demande ici. Si Safari et l’app ne partagent pas leur session, associez l’app séparément. La dictée utilise le micro du clavier iOS."]) steps.append(node("li", text));
      section.append(steps);
      section.append(button("Vérifier Tailscale", async () => { tailscale = await host.api("/api/iphone/tailscale"); }));
      if (tailscale) {
        section.append(node("p", tailscale.error || `${tailscale.running ? "Tailscale connecté" : "Tailscale arrêté"} · ${tailscale.tailnet || "Tailnet inconnu"}`));
        if (tailscale.url) { const address = node("p", tailscale.url, "iphone-address"); section.append(address); }
      }
      const commands = node("pre", `tailscale serve --bg http://127.0.0.1:${data.port}\n# Pour arrêter uniquement le service HTTPS :\ntailscale serve --https=443 off`, "iphone-commands"); section.append(commands);
      section.append(node("p", "Ces boutons modifient le service HTTPS 443 de Tailscale sur ce Mac. Si vous l’utilisez déjà pour une autre app, faites la configuration manuellement pour éviter de le remplacer. Désactiver l’accès Ezerbot ferme son port, mais ne modifie pas Tailscale."));
      const actions = node("div", null, "iphone-actions");
      actions.append(button("Publier Ezerbot avec Tailscale", async () => { if (!confirm("Publier Ezerbot sur le HTTPS 443 privé de ce Mac ? Cela peut remplacer un service Tailscale existant. Aucun accès public ne sera activé.")) return; tailscale = await host.api("/api/iphone/serve", { start: true }); if (!tailscale.error) message = "Publication privée activée. Ouvrez l’adresse HTTPS sur l’iPhone."; }), button("Arrêter le service HTTPS Tailscale", async () => { if (!confirm("Arrêter le service HTTPS 443 de Tailscale sur ce Mac ?")) return; tailscale = await host.api("/api/iphone/serve", { start: false }); message = "Service HTTPS Tailscale arrêté."; }));
      section.append(actions, button("Créer un code d’association", async () => { const value = await host.api("/api/iphone/code", {}); code = value.code; expires = value.expires; }));
      if (code) { section.append(node("p", code, "iphone-code"), node("p", `Code à usage unique · expire à ${new Date(expires).toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" })}. Ne le partagez pas.`)); }
    }
    section.append(node("h4", "Appareils associés"));
    if (!data?.devices?.length) section.append(node("p", "Aucun appareil associé."));
    for (const device of data?.devices || []) {
      const row = node("div", null, "iphone-device"); const copy = node("div"); copy.append(node("strong", device.name));
      copy.append(node("small", device.status === "pending" ? "Demande reçue · confirmez uniquement l’appareil que vous venez d’associer" : `${device.connected && data.enabled ? "Connecté" : "Hors ligne"} · Dernière activité : ${device.last_seen ? new Date(device.last_seen).toLocaleString("fr-CA") : "jamais"}`));
      if (device.push_error) copy.append(node("small", device.push_error, "iphone-warning")); row.append(copy);
      const actions = node("div", null, "iphone-actions");
      if (device.status === "pending") actions.append(button("Confirmer", () => set("/api/iphone/confirm", { id: device.id, allow: true }), "primary-button"), button("Refuser", () => set("/api/iphone/confirm", { id: device.id, allow: false })));
      else actions.append(button("Révoquer", async () => { if (confirm(`Révoquer « ${device.name} » ? Ses accès et notifications seront immédiatement retirés.`)) await set("/api/iphone/revoke", { id: device.id }); }, "danger-button"));
      row.append(actions); section.append(row);
    }
    if (message) { const text = node("p", message, failed ? "iphone-warning" : "iphone-notice"); text.role = failed ? "alert" : "status"; section.append(text); }
  }
  function renderPhone() {
    section.append(node("p", "Le Mac doit rester allumé, Ezerbot ouvert et Tailscale connecté sur les deux appareils. Les approbations et les autorisations sont identiques à celles du Mac."));
    section.append(node("p", "Pour installer : Safari › Partager › Sur l’écran d’accueil › Ajouter. Pour dicter : utilisez le micro du clavier iOS."));
    section.append(button("Activer les notifications sur cet iPhone", enablePush), button("Désactiver les notifications sur cet iPhone", async () => { await host.api("/api/iphone/push", { subscription: null }); try { const registration = await navigator.serviceWorker?.getRegistration(); const sub = await registration?.pushManager.getSubscription(); await sub?.unsubscribe(); }
      catch { throw new Error("Les envois du Mac sont désactivés. iOS n’a pas pu retirer l’abonnement local ; réessayez après avoir rouvert l’app."); }
      message = "Notifications désactivées sur cet appareil."; }));
    section.append(node("p", "iOS 16.4 ou plus récent : ouvrez l’app depuis l’écran d’accueil. Les réglages par type et Ne pas déranger ci-dessous s’appliquent aussi à cet iPhone. Aucun push pour le fil ouvert."));
    if (message) { const text = node("p", message); text.role = "status"; section.append(text); }
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
    await host.api("/api/iphone/push", { subscription: subscription.toJSON() });
    const notifications = await host.api("/api/notifications");
    if (!notifications.settings.enabled) await host.api("/api/notifications/settings", { ...notifications.settings, enabled: true });
    message = "Notifications autorisées sur cet iPhone. Vérifiez Ne pas déranger et les types souhaités ci-dessous.";
  }
  async function presence() {
    if (!remote || !host.connected) return;
    try { await host.api("/api/iphone/presence", { conversation_id: context?.conversation?.id || null, visible: document.visibilityState === "visible" && !document.querySelector("dialog[open]") }); } catch { /* shell shows recovery */ }
  }
  host.register({
    id: "iphone", settingsSection(container) { section = container; section.id = "settings-iphone"; section.className = "setting-section iphone-settings"; render(); },
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
