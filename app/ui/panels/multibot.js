"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, cls, text) => { const n = document.createElement(tag); n.className = cls || ""; if (text !== undefined) n.textContent = text; return n; };
  let context;
  const button = node("button", "tool-pill", "Créer un groupe");
  button.type = "button";
  document.querySelector(".topbar-actions").prepend(button);
  const bar = node("div", "group-bar");
  document.querySelector(".topbar").after(bar);
  const dialog = node("dialog", "dialog group-dialog");
  dialog.setAttribute("aria-label", "Membres de la conversation");
  document.getElementById("panels").append(dialog);
  let selected = new Set();
  let mutating = false;
  let creating = false;
  let focusKey = null;
  function action(text, run, cls = "secondary-button") {
    const b = node("button", cls, text); b.type = "button"; b.disabled = mutating; b.dataset.focusKey = text;
    b.onclick = async () => {
      if (mutating) return;
      focusKey = b.dataset.focusKey;
      mutating = true; b.disabled = true;
      if (dialog.open) renderDialog();
      try { await run(); } catch (error) { host.notify(error.message); }
      finally { mutating = false; if (dialog.open) renderDialog(); }
    };
    return b;
  }
  async function command(path, bot) {
    const conversation = context.conversation;
    await host.api(path, { conversation_id: conversation.id, bot_id: bot, command_id: crypto.randomUUID() });
    await host.refresh();
  }
  function renderDialog() {
    if (dialog.contains(document.activeElement) && document.activeElement.dataset.focusKey) focusKey = document.activeElement.dataset.focusKey;
    const scrollTop = dialog.scrollTop;
    const conversation = context.conversation;
    const bots = context.state.bots.filter((b) => !b.deleted || conversation?.member_ids?.includes(b.id));
    dialog.replaceChildren();
    const header = node("div", "dialog-header");
    header.append(node("h2", "", creating ? "Créer un groupe" : "Membres du fil"), action("Fermer", () => dialog.close()));
    dialog.append(header);
    const section = node("section", "setting-section");
    section.append(node("h3", "", creating ? "Deux à six bots" : "Vos bots · six au maximum"), node("p", "", creating ? "Choisissez de deux à six bots. Ils partageront ce nouveau fil. Les conversations privées restent privées." : "Ajouter un bot lui donne accès à l’historique visible de ce fil, pas aux conversations privées des autres bots."));
    const ids = conversation?.member_ids || [];
    for (const bot of bots) {
      const row = node(creating ? "label" : "div", "group-member");
      row.title = `Bot : ${bot.id}`;
      const avatar = node("span", "group-member-avatar"); avatar.setAttribute("aria-hidden", "true");
      avatar.append(host.createBotMascot(bot)());
      const copy = node("div", "group-member-copy");
      copy.append(node("strong", "", bot.name), node("small", "", host.botSummary(bot)));
      row.append(avatar, copy);
      if (creating) {
        const check = document.createElement("input"); check.className = "group-select"; check.type = "checkbox"; check.checked = selected.has(bot.id);
        check.setAttribute("aria-label", `Choisir ${bot.name} (${bot.id})`);
        check.disabled = mutating || bot.deleted || (!check.checked && selected.size >= 6);
        check.onchange = () => { if (check.checked) selected.add(bot.id); else selected.delete(bot.id); renderDialog(); };
        row.append(check);
      } else if (ids.includes(bot.id)) {
        if (conversation.bot_id === bot.id) row.append(node("span", "group-coordinator", "Coordinateur"));
        else row.append(action("Coordonner", () => command("/api/conversations/coordinator", bot.id)));
        const remove = action("Retirer", () => command("/api/conversations/members/remove", bot.id));
        remove.disabled ||= ids.length === 1;
        if (ids.length === 1) remove.title = "Un groupe garde au moins un bot.";
        row.append(remove);
      } else {
        const add = action("Ajouter", () => command("/api/conversations/members/add", bot.id));
        add.disabled ||= ids.length >= 6 || bot.deleted;
        if (ids.length >= 6) add.title = "Six bots au maximum.";
        row.append(add);
      }
      for (const control of row.querySelectorAll("button, input")) {
        control.dataset.focusKey = `${bot.id}:${control.type === "checkbox" ? "choix" : control.textContent}`;
        if (control.tagName === "BUTTON") control.setAttribute("aria-label", `${control.textContent} ${bot.name} (${bot.id})`);
      }
      section.append(row);
    }
    dialog.append(section);
    const rules = node("section", "setting-section");
    rules.append(node("h3", "", "Prise de parole"), node("p", "", "Sans mention, le coordinateur répond. @ ouvre la liste des membres ; @everyone les adresse tous. Une chaîne s’arrête après 12 tours ou 3 tours par bot. Les demandes entre bots restent dans ce fil."));
    dialog.append(rules);
    const footer = node("div", "dialog-footer");
    footer.append(action(creating ? "Annuler" : "Terminé", () => dialog.close()));
    if (creating) {
      const create = action("Créer le groupe", async () => {
        const response = await host.api("/api/conversations", { bot_ids: [...selected] });
        await host.refresh(); host.openConversation(response.conversation.id); dialog.close();
      }, "primary-button");
      create.disabled ||= selected.size < 2; footer.append(create);
    }
    dialog.append(footer);
    if (creating && selected.size < 2) {
      const help = node("p", "group-help", "Choisissez au moins deux bots."); help.setAttribute("role", "status"); dialog.append(help);
    }
    dialog.scrollTop = scrollTop;
    const focused = [...dialog.querySelectorAll("[data-focus-key]")].find((control) => control.dataset.focusKey === focusKey) || dialog.querySelector("button");
    if (dialog.open && focused && !focused.disabled) focused.focus({ preventScroll: true });
  }
  button.onclick = () => {
    focusKey = null;
    creating = !context.conversation;
    selected = new Set([context.bot?.id].filter(Boolean));
    renderDialog(); dialog.showModal();
  };
  host.register({
    id: "multibot",
    onState(_state, current) {
      context = current;
      const conversation = current.conversation;
      const supported = !conversation || ["chat", "group"].includes(conversation.kind);
      button.hidden = !supported; button.textContent = conversation ? "Membres" : "Créer un groupe";
      bar.replaceChildren(); bar.hidden = conversation?.kind !== "group";
      if (!bar.hidden) {
        const names = (conversation.member_ids || []).map((id) => current.state.bots.find((b) => b.id === id)?.name || id);
        const coordinator = current.state.bots.find((b) => b.id === conversation.bot_id)?.name || "le coordinateur";
        const copy = node("div", "group-bar-copy");
        copy.append(node("span", "group-bar-title", names.length ? `Groupe · ${names.join(", ")}` : "Groupe sans membre actif"), node("span", "", names.length ? `Sans mention, ${coordinator} répond` : "Ajoutez un bot pour poursuivre"));
        copy.title = [...copy.children].map((n) => n.textContent).join(" · "); bar.append(copy);
        if (conversation.messages.some((m) => window.GrokbotChat.isPendingReply(m, true))) {
          bar.append(action("Arrêter les tours", async () => {
            await host.api("/api/stop", { conversation_id: conversation.id, message_id: "groupe" }); await host.refresh();
          }));
        }
      }
      if (dialog.open && !mutating) renderDialog();
    },
  });
})();
