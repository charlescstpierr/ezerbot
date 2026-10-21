"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let section, version, status, log, search, update, corner, cornerLabel;
  let tracking = false;
  let latest = null;

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
  const flag = (el, name, on) => {
    const names = el.className.split(/\s+/).filter((item) => item && item !== name);
    if (on) names.push(name);
    el.className = names.join(" ");
  };

  const progressLabel = {
    telechargement: "Téléchargement…",
    compilation: "Compilation…",
    installation: "Installation…",
    redemarrage: "Redémarrage…",
  };

  function installedLabel(data) {
    const versionText = data?.version || "";
    const commit = data?.installed_commit === "inconnu" || !data?.installed_short
      ? "commit inconnu"
      : data.installed_short;
    return versionText ? `Version installée ${versionText} · ${commit}` : "Version installée indisponible";
  }

  function progressing(data) {
    return Boolean(data?.running) && Object.prototype.hasOwnProperty.call(progressLabel, data.phase);
  }

  function ensureCorner() {
    if (corner) return;
    const account = document.getElementById("settings-button");
    const host = account?.parentElement;
    if (!host) return;
    corner = button("", "update-corner", () => run("/api/updates/apply"));
    corner.hidden = true;
    const mark = node("span", "update-corner-mark");
    mark.setAttribute("aria-hidden", "true");
    cornerLabel = node("span", "update-corner-label", "Mettre à jour");
    corner.append(mark, cornerLabel);
    host.insertBefore(corner, account);
  }

  function paint(data) {
    if (!data) return;
    latest = data;
    const available = Boolean(data.available);
    const running = Boolean(data.running);
    const busy = progressing(data);
    ensureCorner();
    if (corner) {
      corner.hidden = !(available || busy);
      corner.disabled = busy;
      flag(corner, "is-running", busy);
      const label = busy ? progressLabel[data.phase] : "Mettre à jour";
      cornerLabel.textContent = label;
      corner.setAttribute("aria-label", label);
      if (data.phase === "erreur" && data.message) corner.setAttribute("title", data.message);
      else corner.setAttribute("title", "");
    }
    if (!section) {
      tracking = running;
      return;
    }
    version.textContent = installedLabel(data);
    update.hidden = !available || busy;
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
    if (search) search.disabled = true;
    if (update) update.disabled = true;
    if (corner) corner.disabled = true;
    if (status) status.dataset.tone = "";
    try {
      paint(await panels.api(path, {}));
    } catch (error) {
      if (status) {
        status.dataset.tone = "error";
        status.textContent = error.message;
      }
      tracking = false;
      if (search) search.disabled = false;
      paint({ ...(latest || {}), running: false, phase: "erreur", message: error.message, log: latest?.log || "" });
    }
  }

  panels.register({
    id: "updates",
    wantsPolling: () => tracking,
    onState(state) { paint(state?.updates); },
    settingsSection(container) {
      section = container;
      section.className = "setting-section updates-section";
      version = node("p", "updates-version", "Version installée");
      const intro = node("p", "", "Ezerbot compare cette version à la branche main, puis télécharge, compile et remplace l’application. Le dossier de développement n’est pas modifié. Vos conversations et vos sessions restent sur ce Mac.");
      const actions = node("div", "updates-actions");
      search = button("Rechercher les mises à jour", "secondary-button", () => run("/api/updates/check"));
      update = button("Mettre à jour et redémarrer", "secondary-button", () => run("/api/updates/apply"));
      update.hidden = true;
      actions.append(search, update);
      status = node("p", "updates-status");
      status.setAttribute("role", "status");
      log = node("pre", "updates-log");
      log.hidden = true;
      section.append(node("h3", "", "Mises à jour"), version, intro, actions, status, log);
      ensureCorner();
    },
  });
})();
