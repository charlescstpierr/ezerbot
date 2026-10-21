"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let section, version, status, log, search, update, badge, dot;
  let tracking = false;

  const node = (tag, cls, text) => {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const button = (text, cls, action) => {
    const el = node("button", cls, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  };

  function installedLabel(data) {
    const versionText = data?.version || "";
    const commit = data?.installed_commit === "inconnu" || !data?.installed_short
      ? "commit inconnu"
      : data.installed_short;
    return versionText ? `Version installée ${versionText} · ${commit}` : "Version installée indisponible";
  }

  function ensureBadge() {
    const host = document.getElementById("settings-button");
    if (!host || badge) return;
    badge = node("span", "update-badge");
    badge.setAttribute("role", "img");
    badge.setAttribute("aria-label", "Mise à jour disponible");
    badge.hidden = true;
    const icon = host.querySelector(".icon");
    if (icon) host.insertBefore(badge, icon);
    else host.append(badge);
  }

  function paint(data) {
    if (!section || !data) return;
    version.textContent = installedLabel(data);
    const available = Boolean(data.available);
    const running = Boolean(data.running);
    if (dot) dot.hidden = !available;
    ensureBadge();
    if (badge) badge.hidden = !available;
    update.hidden = !available || running;
    search.disabled = running;
    update.disabled = running;
    status.textContent = data.message || "";
    status.dataset.tone = data.phase === "erreur" ? "error" : "";
    log.hidden = !data.log;
    log.textContent = data.log || "";
    tracking = running;
    section.setAttribute("aria-busy", String(running));
  }

  async function run(path) {
    tracking = true;
    search.disabled = true;
    update.disabled = true;
    status.dataset.tone = "";
    try {
      paint(await panels.api(path, {}));
    } catch (error) {
      status.dataset.tone = "error";
      status.textContent = error.message;
      tracking = false;
      search.disabled = false;
    }
  }

  panels.register({
    id: "updates",
    wantsPolling: () => tracking,
    onState(state) { paint(state?.updates); },
    settingsSection(container) {
      section = container;
      section.className = "setting-section updates-section";
      const title = node("div", "updates-title");
      dot = node("span", "update-badge updates-dot");
      dot.hidden = true;
      dot.setAttribute("aria-hidden", "true");
      title.append(node("h3", "", "Mises à jour"), dot);
      version = node("p", "updates-version", "Version installée");
      const intro = node("p", "", "Ezerbot compare cette version à la branche main, puis télécharge, compile et remplace l’application. Le dossier de développement n’est pas modifié. Vos conversations et vos sessions restent sur ce Mac.");
      const actions = node("div", "updates-actions");
      search = button("Rechercher les mises à jour", "secondary-button", () => run("/api/updates/check"));
      update = button("Mettre à jour et redémarrer", "primary-button", () => run("/api/updates/apply"));
      update.hidden = true;
      actions.append(search, update);
      status = node("p", "updates-status");
      status.setAttribute("role", "status");
      log = node("pre", "updates-log");
      log.hidden = true;
      section.append(title, version, intro, actions, status, log);
      ensureBadge();
    },
  });
})();
