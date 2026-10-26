"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let section, version, attempt, failure;
  let corner, cornerAction, cornerLabel, cornerError, cornerErrorText;
  let tracking = false;
  let latest = null;
  let dismissedError = "";

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
    attente: "Après l’enregistrement",
  };
  const attemptLabel = {
    aucune: "Dernière tentative : aucune",
    en_cours: "Dernière tentative : en cours",
    echec: "Dernière tentative : échec",
    reussie: "Dernière tentative : réussie",
    attente: "Dernière tentative : après l’enregistrement",
    a_jour: "Dernière tentative : déjà à jour",
  };
  // null : le serveur peut garder un hold d’avant le rechargement ; le premier état est toujours envoyé.
  let holdSent = null;

  function captureActive() {
    return Boolean(panels.meetingActive?.() || panels.voiceActive?.() || window.EzerbotVoiceMode?.active?.() || window.EzerbotVoiceCall?.active?.());
  }

  function syncHold() {
    const active = captureActive();
    if (active === holdSent) return;
    holdSent = active;
    panels.api("/api/updates/hold", { active }).catch(() => { holdSent = null; });
  }

  function versionLabel(data) {
    const versionText = data?.version || "";
    const commit = data?.installed_commit === "inconnu" || !data?.installed_short
      ? "commit inconnu"
      : data.installed_short;
    return versionText ? `Version actuelle ${versionText} · ${commit}` : "Version actuelle indisponible";
  }

  function progressing(data) {
    return Boolean(data?.running) && Object.prototype.hasOwnProperty.call(progressLabel, data.phase);
  }

  function ensureCorner() {
    if (corner) return;
    const host = document.getElementById("sidebar-updates");
    if (!host) return;
    corner = node("div", "update-corner");
    corner.hidden = true;
    cornerAction = button("", "update-corner-action", () => {
      if (cornerAction.disabled) return undefined;
      dismissedError = "";
      return run("/api/updates/apply");
    });
    const mark = node("span", "update-corner-mark");
    mark.setAttribute("aria-hidden", "true");
    cornerLabel = node("span", "update-corner-label", "Mettre à jour");
    cornerAction.append(mark, cornerLabel);
    cornerError = node("p", "update-corner-error");
    cornerError.hidden = true;
    cornerError.setAttribute("role", "alert");
    cornerErrorText = node("span", "update-corner-error-text");
    const dismiss = button("×", "update-corner-dismiss", () => {
      dismissedError = cornerErrorText.textContent;
      paint(latest);
    });
    dismiss.setAttribute("aria-label", "Fermer");
    cornerError.append(cornerErrorText, dismiss);
    corner.append(cornerAction, cornerError);
    host.append(corner);
  }

  function paint(data) {
    if (!data) return;
    latest = data;
    const available = Boolean(data.available);
    const running = Boolean(data.running);
    const waiting = data.phase === "attente" || (data.phase === "redemarrage" && captureActive());
    const busy = progressing(data) || waiting;
    const errorText = data.phase === "erreur" ? (data.message || "") : "";
    const showError = Boolean(errorText) && errorText !== dismissedError;
    ensureCorner();
    syncHold();
    if (corner) {
      corner.hidden = !(available || busy || showError);
      cornerAction.hidden = showError || !(available || busy);
      cornerAction.disabled = busy;
      flag(corner, "is-running", busy && !waiting && !showError);
      const label = waiting ? progressLabel.attente : busy ? progressLabel[data.phase] : "Mettre à jour";
      cornerLabel.textContent = label;
      cornerAction.setAttribute("aria-label", label);
      if (waiting && data.message) cornerAction.setAttribute("title", data.message);
      else cornerAction.setAttribute("title", "");
      cornerError.hidden = !showError;
      cornerErrorText.textContent = showError ? errorText : "";
    }
    if (!section) {
      tracking = running;
      return;
    }
    version.textContent = versionLabel(data);
    const attemptCode = attemptLabel[data.last_attempt] ? data.last_attempt : "aucune";
    attempt.textContent = attemptLabel[attemptCode];
    const storedError = data.last_error || (data.phase === "erreur" ? data.message : "");
    failure.hidden = !storedError;
    failure.textContent = storedError || "";
    tracking = running;
    section.setAttribute("aria-busy", String(running));
  }

  async function run(path) {
    tracking = true;
    dismissedError = "";
    if (cornerAction) cornerAction.disabled = true;
    try {
      paint(await panels.api(path, {}));
    } catch (error) {
      tracking = false;
      paint({
        ...(latest || {}),
        running: false,
        phase: "erreur",
        message: error.message,
        last_attempt: "echec",
        last_error: error.message,
      });
    }
  }

  panels.register({
    id: "updates",
    wantsPolling: () => tracking,
    onState(state) { paint(state?.updates); },
    settingsSection(container) {
      section = container;
      section.id = "settings-updates";
      section.className = "setting-section updates-section";
      const dialog = document.getElementById("settings-dialog");
      const footer = dialog?.querySelector(".dialog-footer");
      if (footer) footer.before(section);
      version = node("p", "updates-version", "Version actuelle");
      attempt = node("p", "updates-attempt", attemptLabel.aucune);
      failure = node("p", "updates-error");
      failure.hidden = true;
      failure.setAttribute("role", "status");
      section.append(node("h3", "", "Mises à jour"), version, attempt, failure);
      ensureCorner();
    },
  });
  if (typeof window.addEventListener === "function") {
    window.addEventListener("grokbot:meeting-change", syncHold);
    window.addEventListener("grokbot:voice-change", syncHold);
    window.addEventListener("grokbot:voice-mode-change", syncHold);
    window.addEventListener("grokbot:voice-call-change", syncHold);
  }
})();
