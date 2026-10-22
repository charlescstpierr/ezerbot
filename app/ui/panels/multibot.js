"use strict";
(() => {
  const host = window.GrokbotPanels;
  const node = (tag, cls, text) => { const n = document.createElement(tag); n.className = cls || ""; if (text !== undefined) n.textContent = text; return n; };
  const COST = "Les groupes consomment plus de ton usage, car plusieurs bots traitent chaque message.";
  const HINT = "Pour la plupart des tâches, parle plutôt à un seul bot.";
  const BANNER_KEY = "ezerbot.group-cost-banner";
  const COLORS = [
    ["citron", "Jaune citron", "#E6FF3D"],
    ["orange", "Orange vif", "#FF5C1F"],
    ["ambre", "Ambre", "#FFB224"],
    ["menthe", "Vert menthe", "#35E0A1"],
    ["cyan", "Cyan", "#22D3EE"],
    ["magenta", "Rose magenta", "#FF3D8B"],
    ["vermillon", "Rouge vermillon", "#FF4530"],
    ["violet", "Violet", "#9B7BFF"],
    ["blanc", "Sans couleur", "#F5F5F5"],
  ];
  const dismissedMemory = new Set();
  const MEMBER_KINDS = ["chat", "group"];
  const INTENTS = Object.freeze({
    create: Object.freeze({
      label: "Créer un groupe",
      title: "Créer un groupe",
      visible: (conversation) => !conversation || MEMBER_KINDS.includes(conversation.kind),
    }),
    members: Object.freeze({
      label: "Membres",
      title: "Membres du fil",
      visible: (conversation) => Boolean(conversation) && MEMBER_KINDS.includes(conversation.kind),
    }),
  });
  let context;
  let session = null;
  function makePill(intent, row) {
    const el = node("button", "tool-pill", row.label);
    el.type = "button";
    el.dataset.topbarIntent = intent;
    return el;
  }
  const pills = new Map(Object.entries(INTENTS).map(([intent, row]) => [intent, makePill(intent, row)]));
  document.querySelector(".topbar-actions").prepend(...pills.values());
  const bar = node("div", "group-bar");
  document.querySelector(".topbar").after(bar);
  const banner = node("aside", "group-cost-banner");
  banner.setAttribute("role", "note");
  bar.after(banner);
  const dialog = node("dialog", "dialog group-dialog");
  dialog.setAttribute("aria-label", "Membres de la conversation");
  document.getElementById("panels").append(dialog);
  let selected = new Set();
  let mutating = false;
  let focusKey = null;
  let profile = { name: "", description: "", color: "citron", mark: "" };
  function dismissed() {
    try {
      const parsed = JSON.parse(sessionStorage.getItem(BANNER_KEY) || "[]");
      if (Array.isArray(parsed)) return new Set(parsed.filter((id) => typeof id === "string"));
    } catch { /* La session du document ne conserve pas le bandeau : la mémoire du panneau suffit. */ }
    return new Set(dismissedMemory);
  }
  function dismiss(id) {
    const ids = dismissed();
    ids.add(id);
    dismissedMemory.add(id);
    try { sessionStorage.setItem(BANNER_KEY, JSON.stringify([...ids])); } catch { /* déjà retenu en mémoire */ }
  }
  function blankProfile() { return { name: "", description: "", color: "citron", mark: "" }; }
  function profileFrom(conversation) {
    return {
      name: conversation?.title || "",
      description: conversation?.description || "",
      color: conversation?.avatar?.color || "citron",
      mark: conversation?.avatar?.mark || "",
    };
  }
  function canCreate() {
    return Boolean(profile.name.trim() && profile.description.trim() && selected.size >= 2 && selected.size <= 6);
  }
  function canSave() { return Boolean(profile.name.trim() && profile.description.trim()); }
  function isTextField(node) {
    return node.tagName === "TEXTAREA" || (node.tagName === "INPUT" && node.type !== "checkbox");
  }
  function avatarMark() {
    return profile.mark.trim() || Array.from(profile.name.trim() || "G")[0] || "G";
  }
  function payloadFields() {
    return {
      title: profile.name.trim(),
      description: profile.description.trim(),
      avatar: { color: profile.color, mark: avatarMark() },
    };
  }
  function syncActions() {
    const create = dialog.querySelector("[data-group-action=create]");
    if (create) create.disabled = mutating || !canCreate();
    const save = dialog.querySelector("[data-group-action=save]");
    if (save) save.disabled = mutating || !canSave();
    const preview = dialog.querySelector("[data-group-preview]");
    if (preview) {
      preview.dataset.botColor = profile.color;
      preview.textContent = profile.mark.trim() || Array.from(profile.name.trim() || "G")[0] || "G";
    }
  }
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
  function dialogView(current) {
    if (current.intent === "create") return { title: INTENTS.create.title, target: null, profile: "create", members: "choose" };
    const target = context.conversation?.id === current.conversationId ? context.conversation : null;
    return { title: INTENTS.members.title, target, profile: target?.kind === "group" ? "edit" : null, members: "manage" };
  }
  function sessionStillValid(current) {
    if (current.intent === "create") return true;
    return context.conversation?.id === current.conversationId && INTENTS.members.visible(context.conversation);
  }
  function dialogBots(bots, view) {
    return bots.filter((b) => !b.deleted || view.target?.member_ids?.includes(b.id));
  }
  function openDialog(next) {
    session = next;
    focusKey = null;
    selected = new Set();
    const view = dialogView(next);
    profile = view.target ? profileFrom(view.target) : blankProfile();
    renderDialog();
    dialog.showModal();
  }
  async function command(path, bot) {
    await host.api(path, { conversation_id: session.conversationId, bot_id: bot, command_id: crypto.randomUUID() });
    await host.refresh();
  }
  function profileSection(mode) {
    const section = node("section", "setting-section group-profile");
    section.append(node("h3", "", mode === "create" ? "Le groupe" : "Nom du groupe"));
    const nameLabel = node("label", "form-label", "Nom");
    const name = node("input", "text-input");
    name.type = "text";
    name.maxLength = 80;
    name.required = true;
    name.autocomplete = "off";
    name.dataset.groupField = "name";
    name.dataset.focusKey = "profil:nom";
    name.value = profile.name;
    name.setAttribute("aria-label", "Nom du groupe");
    name.oninput = () => { profile.name = name.value; syncActions(); };
    const descriptionLabel = node("label", "form-label", "Description");
    const description = node("textarea", "text-input");
    description.maxLength = 280;
    description.rows = 3;
    description.required = true;
    description.dataset.groupField = "description";
    description.dataset.focusKey = "profil:description";
    description.value = profile.description;
    description.setAttribute("aria-label", "Description du groupe");
    description.oninput = () => { profile.description = description.value; syncActions(); };
    const avatarLabel = node("p", "form-label", "Avatar");
    avatarLabel.id = "group-avatar-label";
    const row = node("div", "group-avatar-row");
    const preview = node("span", "group-mark group-mark-preview");
    preview.dataset.groupPreview = "true";
    preview.dataset.botColor = profile.color;
    preview.textContent = profile.mark.trim() || Array.from(profile.name.trim() || "G")[0] || "G";
    preview.setAttribute("aria-hidden", "true");
    const picker = node("div", "accent-options");
    picker.setAttribute("role", "group");
    picker.setAttribute("aria-labelledby", "group-avatar-label");
    for (const [color, label, swatch] of COLORS) {
      const choice = node("button", "");
      choice.type = "button";
      choice.dataset.avatarColor = color;
      choice.dataset.botColor = color;
      choice.dataset.focusKey = `avatar:${color}`;
      choice.setAttribute("aria-pressed", String(profile.color === color));
      choice.setAttribute("aria-label", label);
      const dot = node("i");
      dot.style.setProperty("--swatch", swatch);
      choice.append(dot, node("span", "", label));
      choice.onclick = () => { profile.color = color; renderDialog(); };
      picker.append(choice);
    }
    const markLabel = node("label", "form-label", "Signe");
    const mark = node("input", "text-input");
    mark.type = "text";
    mark.maxLength = 4;
    mark.autocomplete = "off";
    mark.placeholder = "A";
    mark.dataset.groupField = "mark";
    mark.dataset.focusKey = "profil:signe";
    mark.value = profile.mark;
    mark.setAttribute("aria-label", "Signe de l’avatar");
    mark.oninput = () => { profile.mark = mark.value; syncActions(); };
    row.append(preview, picker);
    section.append(nameLabel, name, descriptionLabel, description, avatarLabel, row, markLabel, mark);
    return section;
  }
  function renderDialog() {
    const active = document.activeElement;
    let selection = null;
    if (dialog.contains(active) && active.dataset.focusKey) {
      focusKey = active.dataset.focusKey;
      if (isTextField(active)) selection = { start: active.selectionStart ?? 0, end: active.selectionEnd ?? active.selectionStart ?? 0 };
    }
    const scrollTop = dialog.scrollTop;
    const view = dialogView(session);
    const conversation = view.target;
    const choosing = view.members === "choose";
    const bots = dialogBots(context.state.bots, view);
    dialog.replaceChildren();
    const header = node("div", "dialog-header");
    header.append(node("h2", "", view.title), action("Fermer", () => dialog.close()));
    dialog.append(header);
    if (view.profile) dialog.append(profileSection(view.profile));
    const section = node("section", "setting-section");
    section.append(node("h3", "", choosing ? "Deux à six bots" : "Vos bots · six au maximum"), node("p", "", choosing ? "Choisissez de deux à six bots. Ils partageront ce nouveau fil. Les conversations privées restent privées." : "Ajouter un bot lui donne accès à l’historique visible de ce fil, pas aux conversations privées des autres bots."));
    const ids = conversation?.member_ids || [];
    for (const bot of bots) {
      const row = node(choosing ? "label" : "div", "group-member");
      row.title = `Bot : ${bot.id}`;
      const avatar = node("span", "group-member-avatar"); avatar.setAttribute("aria-hidden", "true");
      avatar.append(host.createBotMascot(bot)());
      const copy = node("div", "group-member-copy");
      copy.append(node("strong", "", bot.name), node("small", "", host.botSummary(bot)));
      row.append(avatar, copy);
      if (choosing) {
        const check = document.createElement("input"); check.className = "group-select"; check.type = "checkbox"; check.checked = selected.has(bot.id);
        check.dataset.botChoice = bot.id;
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
    footer.append(action(choosing ? "Annuler" : "Terminé", () => dialog.close()));
    if (choosing) {
      const create = action("Créer le groupe", async () => {
        const response = await host.api("/api/conversations", { bot_ids: [...selected], ...payloadFields() });
        await host.refresh(); host.openConversation(response.conversation.id); dialog.close();
      }, "primary-button");
      create.dataset.groupAction = "create";
      create.disabled ||= !canCreate();
      footer.append(create);
    } else if (conversation?.kind === "group") {
      const save = action("Enregistrer", async () => {
        await host.api("/api/conversations/update", { id: conversation.id, ...payloadFields() });
        await host.refresh();
        dialog.close();
      }, "primary-button");
      save.dataset.groupAction = "save";
      save.disabled ||= !canSave();
      footer.append(save);
    }
    dialog.append(footer);
    if (choosing && !canCreate()) {
      const help = node("p", "group-help", selected.size < 2 ? "Choisissez au moins deux bots." : "Indiquez un nom et une description.");
      help.setAttribute("role", "status");
      dialog.append(help);
    }
    dialog.scrollTop = scrollTop;
    const focused = [...dialog.querySelectorAll("[data-focus-key]")].find((control) => control.dataset.focusKey === focusKey) || dialog.querySelector("button");
    if (dialog.open && focused && !focused.disabled) {
      focused.focus({ preventScroll: true });
      if (selection && isTextField(focused)) focused.setSelectionRange(selection.start, selection.end);
    }
  }
  function renderBars() {
    const conversation = context?.conversation;
    const group = conversation?.kind === "group";
    bar.replaceChildren();
    bar.hidden = !group;
    banner.hidden = !group || dismissed().has(conversation.id);
    if (banner.hidden) banner.setAttribute("hidden", "");
    else banner.removeAttribute("hidden");
    if (!group) return;
    const names = (conversation.member_ids || []).map((id) => context.state.bots.find((b) => b.id === id)?.name || id);
    const coordinator = context.state.bots.find((b) => b.id === conversation.bot_id)?.name || "le coordinateur";
    const mark = node("span", "group-mark");
    mark.dataset.botColor = conversation.avatar?.color || "citron";
    mark.textContent = conversation.avatar?.mark || Array.from((conversation.title || "G").trim() || "G")[0] || "G";
    mark.setAttribute("aria-hidden", "true");
    const copy = node("div", "group-bar-copy");
    const title = conversation.title?.trim() || (names.length ? `Groupe · ${names.join(", ")}` : "Groupe sans membre actif");
    copy.append(node("span", "group-bar-title", title), node("span", "", names.length ? `Sans mention, ${coordinator} répond` : "Ajoutez un bot pour poursuivre"));
    copy.title = [...copy.children].map((n) => n.textContent).join(" · ");
    bar.append(mark, copy);
    if ((conversation.messages || []).some((m) => window.GrokbotChat?.isPendingReply(m, true))) {
      bar.append(action("Arrêter les tours", async () => {
        await host.api("/api/stop", { conversation_id: conversation.id, message_id: "groupe" }); await host.refresh();
      }));
    }
    const text = node("div", "group-cost-copy");
    text.append(node("p", "", COST), node("p", "", HINT));
    const close = node("button", "secondary-button", "Fermer");
    close.type = "button";
    close.dataset.dismissBanner = "true";
    close.setAttribute("aria-label", "Fermer le bandeau de ce groupe");
    close.onclick = () => { dismiss(conversation.id); renderBars(); };
    banner.replaceChildren(text, close);
  }
  dialog.addEventListener("close", () => {
    session = null;
    focusKey = null;
    selected = new Set();
    profile = blankProfile();
  });
  pills.get("create").onclick = () => openDialog({ intent: "create" });
  pills.get("members").onclick = () => openDialog({ intent: "members", conversationId: context.conversation.id });
  host.register({
    id: "multibot",
    onState(_state, current) {
      context = current;
      for (const [intent, el] of pills) el.hidden = !INTENTS[intent].visible(current.conversation);
      renderBars();
      if (session && !sessionStillValid(session)) dialog.close();
      else if (dialog.open && !mutating) renderDialog();
    },
  });
})();
