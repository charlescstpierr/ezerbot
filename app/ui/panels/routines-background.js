"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, text, cls) => { const el = document.createElement(tag); if (text) el.textContent = text; if (cls) el.className = cls; return el; };
  let section, saving = false, error = "", signature = "";
  function render(state = host.state) {
    if (!section) return;
    const routines = section.parentElement?.querySelector('[data-panel="routines"]');
    if (routines && routines.nextElementSibling !== section) routines.after(section);
    const data = state?.routines_background || {};
    const next = JSON.stringify([data, saving, error]);
    if (next === signature) return;
    signature = next;
    section.replaceChildren(node("h3", "Routines en arrière-plan"));
    section.append(node("p", "Sur ce Mac connecté à votre session, un agent personnel vérifie les échéances chaque minute. Le moteur démarre seulement si une routine est due, puis s’arrête. Les règles de rattrapage, les autorisations et l’historique restent les mêmes."));
    const row = node("label", null, "setting-toggle notification-toggle");
    const copy = node("span"); copy.append(node("strong", "Exécuter les routines quand Ezerbot est fermé"), node("small", "L’installation et le retrait demandent votre accord dans la conversation Approbations."));
    const input = node("input"); input.type = "checkbox"; input.role = "switch"; input.dataset.routinesBackground = "true";
    input.checked = Boolean(data.enabled); input.disabled = saving || !data.available;
    const track = node("span", null, "toggle-track"); track.setAttribute("aria-hidden", "true");
    input.onchange = async () => {
      const enabled = input.checked;
      saving = true; error = ""; render();
      try {
        const result = await host.api("/api/routines/background", { enabled });
        await host.refresh();
        if (result.conversation_id) {
          document.getElementById("settings-dialog").close();
          host.openConversation(result.conversation_id, result.approval_message_id);
          host.notify("Confirmez l’action dans la carte d’approbation.");
        }
      } catch (e) { error = e.message || "La configuration de l’agent a échoué."; }
      finally { saving = false; render(); }
    };
    row.append(copy, input, track); section.append(row);
    const status = node("p", `Agent : ${data.installed ? "installé" : "non installé"}${data.test_mode && data.available ? " · simulation de test, aucun service macOS modifié" : ""}.`);
    status.role = "status"; status.className = "routines-agent-status"; section.append(status);
    section.append(node("p", `Dernière exécution : ${data.last_run ? new Date(data.last_run).toLocaleString("fr-CA", { dateStyle: "medium", timeStyle: "short" }) : "aucune"}${data.outcome ? ` · ${data.outcome}` : ""}.`));
    if (data.enabled && !data.installed && !data.error) section.append(node("p", "L’agent est absent. Désactivez puis réactivez ce réglage pour le réinstaller.", "notification-warning"));
    if (!data.available) section.append(node("p", data.unavailable_reason || "Installation indisponible."));
    const detail = error || data.error;
    if (detail) { const warning = node("p", detail, "notification-warning"); warning.role = "alert"; section.append(warning); }
    section.append(node("p", "Les résultats et les échecs se retrouvent dans Routines et Notifications. Les alertes macOS suivent vos réglages Notifications. À son premier passage après le réveil, l’agent rattrape les créneaux dus pendant l’extinction ou la veille du Mac, dans l’ordre, même si Ezerbot est resté fermé. Une demande d’autorisation attend votre retour."));
  }
  host.register({ id: "routines-background", settingsSection(container) { section = container; section.className = "setting-section routines-background-settings"; render(); }, onState(state) { render(state); } });
})();
