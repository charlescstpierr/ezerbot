"use strict";
(() => {
  const P = window.GrokbotPanels;
  const bridge = (body) => window.webkit?.messageHandlers?.ezerbotFloating?.postMessage(body);
  let select, status, save, lastContext, configured, chosen;
  let contextWork = Promise.resolve();
  const names = [
    ["cmd+option+space", "⌘ ⌥ Espace"], ["ctrl+option+space", "⌃ ⌥ Espace"],
    ["cmd+shift+space", "⌘ ⇧ Espace"], ["cmd+option+j", "⌘ ⌥ J"], ["disabled", "Désactivé"],
  ];
  function text(tag, content, className) {
    const element = document.createElement(tag); element.textContent = content;
    if (className) element.className = className;
    return element;
  }
  async function configure(shortcut) {
    const result = await bridge({ command: "configure", shortcut });
    if (result?.status === "conflict") throw new Error("Ce raccourci est déjà utilisé par une autre application. Choisissez-en un autre.");
    if (result?.status === "error") throw new Error("Le raccourci n’a pas pu être activé. L’ouverture par le menu reste disponible.");
    status.textContent = result?.status === "recording" ? "Mode test : aucun raccourci global enregistré." : result?.status === "disabled" ? "Raccourci global désactivé. Ouvrez le panneau depuis le menu Ezerbot." : result ? "Raccourci enregistré sur ce Mac, sans permission de surveillance du clavier." : "L’ouverture globale est disponible dans l’application Mac. Ici, le panneau s’ouvre dans le navigateur.";
    configured = shortcut;
  }
  window.EzerbotFloatingMain = {
    async openConversation(id) {
      await P.refresh();
      if (!await P.focusConversation(id)) throw new Error("La conversation n’est plus disponible ou un envoi est en cours.");
      return true;
    },
  };
  P.register({
    id: "floating",
    settingsSection(container) {
      container.className = "setting-section"; container.id = "settings-floating";
      container.append(text("h3", "Panneau flottant"), text("p", "Votre bot et un composeur compact, même lorsque la fenêtre principale est masquée. Déplacez le panneau par sa barre de titre ; Échap le ferme."));
      const label = text("label", "Raccourci global", "form-label"); label.htmlFor = "floating-shortcut";
      select = document.createElement("select"); select.id = "floating-shortcut"; select.className = "floating-shortcut";
      for (const [value, title] of names) { const option = text("option", title); option.value = value; select.append(option); }
      select.addEventListener("change", () => { chosen = select.value; });
      const actions = text("div", "", "floating-settings-actions");
      save = text("button", "Enregistrer le raccourci", "secondary-button"); save.type = "button";
      save.addEventListener("click", async () => {
        save.disabled = true; select.disabled = true;
        const shortcut = select.value;
        const previous = P.state.floating?.shortcut || "cmd+option+j";
        try {
          await configure(shortcut);
          const environment = status.textContent;
          await P.api("/api/floating/settings", { shortcut });
          chosen = null; await P.refresh();
          status.textContent = `Raccourci enregistré : ${names.find(([value]) => value === shortcut)?.[1] || shortcut}. ${environment}`;
        } catch (error) {
          try { await configure(previous); } catch { /* Keep the actionable error below. */ }
          status.textContent = error.message;
        } finally { save.disabled = false; select.disabled = false; }
      });
      const open = text("button", "Ouvrir le panneau", "primary-button"); open.type = "button";
      open.addEventListener("click", async () => {
        try {
          await contextWork;
          const result = bridge({ command: "show" });
          if (result) await result;
          else window.open("/floating", "ezerbot-floating", "width=420,height=560");
        } catch (error) { status.textContent = error.message; }
      });
      status = text("p", "", "floating-settings-status"); status.setAttribute("role", "status");
      actions.append(save, open); container.append(label, select, actions, status);
    },
    onState(state, context) {
      if (!context.connected || !state.floating?.shortcut) return;
      if (!chosen) select.value = state.floating.shortcut;
      if (configured !== state.floating.shortcut) {
        configured = state.floating.shortcut;
        configure(configured).catch((error) => { status.textContent = error.message; });
      }
      const conversation = context.conversation;
      const eligible = !conversation || ["chat", "group"].includes(conversation.kind);
      if (!eligible) return; // A task/routine's composer must not escape its scope.
      const selection = { conversation_id: conversation?.id || null, bot_id: context.bot?.id || "grokbot" };
      const key = JSON.stringify(selection);
      if (lastContext !== key) {
        lastContext = key;
        contextWork = contextWork.catch(() => {}).then(() => P.api("/api/floating/context", selection));
        contextWork.catch(() => { if (lastContext === key) lastContext = null; });
      }
    },
  });
})();
