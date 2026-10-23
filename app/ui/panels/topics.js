/* Sujets : des dossiers qui regroupent plusieurs fils, dans la barre latérale et sur leur page. */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const COLLAPSED_KEY = "ezerbot.topics.collapsed";
  const KINDS = ["chat", "group"];

  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const icon = (name, className = "icon") => {
    if (!document.createElementNS) return node("span", className);
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("class", className);
    svg.setAttribute("aria-hidden", "true");
    const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
    use.setAttribute("href", `#i-${name}`);
    svg.append(use);
    return svg;
  };
  const button = (className, label, action) => {
    const el = node("button", className, label);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  };
  const byId = (id) => document.getElementById(id);

  function stamp(value) {
    if (typeof value === "number") return value < 1e12 ? value * 1000 : value;
    if (typeof value === "string" && /^\d+$/.test(value)) return stamp(Number(value));
    const time = new Date(value).getTime();
    return Number.isNaN(time) ? Date.now() : time;
  }

  // « il y a 2 h » : des minutes aux jours, puis la date.
  function relative(value, now = Date.now()) {
    const then = stamp(value);
    const minutes = Math.max(0, Math.floor((now - then) / 60000));
    if (minutes < 1) return "à l’instant";
    if (minutes < 60) return `il y a ${minutes} min`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `il y a ${hours} h`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `il y a ${days} j`;
    return new Date(then).toLocaleDateString("fr-CA", { day: "numeric", month: "short" });
  }

  const topicsOf = (state) => state?.topics?.items || [];
  const membership = (state) => state?.topics?.membership || {};
  const newest = (a, b) => stamp(b.updated_at) - stamp(a.updated_at);
  function threadsOf(state, topicId) {
    const members = membership(state);
    return (state?.conversations || [])
      .filter((conversation) => members[conversation.id] === topicId && KINDS.includes(conversation.kind))
      .sort(newest);
  }
  const botName = (state, id) => (state?.bots || []).find((bot) => bot.id === id)?.name || "Bot";

  function readCollapsed() {
    try {
      const saved = JSON.parse(localStorage.getItem(COLLAPSED_KEY) || "[]");
      return new Set(Array.isArray(saved) ? saved.filter((id) => typeof id === "string") : []);
    } catch { return new Set(); }
  }
  function writeCollapsed() {
    try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed])); } catch { /* L’état replié reste valable pour cette session. */ }
  }

  let latest = { state: null, context: null };
  let collapsed = readCollapsed();
  let openTopicId = null;
  let lastActiveId = null;
  const painted = { sidebar: null, page: null, menu: null };

  const activeId = () => latest.context?.conversation?.id || null;
  const openTopic = () => topicsOf(latest.state).find((topic) => topic.id === openTopicId) || null;

  function searchState() {
    const query = (byId("search")?.value || "").trim().toLocaleLowerCase("fr");
    const favorites = byId("favorites-filter")?.getAttribute("aria-pressed") === "true";
    return { query, favorites };
  }
  function matches(conversation, query) {
    const text = (value) => String(value || "").toLocaleLowerCase("fr");
    return text(conversation.title).includes(query) || text(conversation.description).includes(query)
      || (conversation.messages || []).some((message) => text(window.GrokbotChat?.searchableMessageText?.(message)).includes(query));
  }

  // Un nouveau rendu remplace les boutons : la touche focalisée est retrouvée par sa clé.
  function repaint(host, signature, slot, build) {
    if (painted[slot] === signature) return;
    painted[slot] = signature;
    const focused = host.contains?.(document.activeElement) ? document.activeElement?.dataset?.focusKey : null;
    host.replaceChildren(...build());
    if (focused) host.querySelector?.(`[data-focus-key="${focused}"]`)?.focus?.();
  }

  async function run(action) {
    try { return await action(); }
    catch (error) { panels.notify(error.message || "L’action n’a pas pu être terminée."); return undefined; }
  }

  function toggle(topicId) {
    if (collapsed.has(topicId)) collapsed.delete(topicId); else collapsed.add(topicId);
    writeCollapsed();
    paint();
  }

  function showPage(topicId) {
    if (panels.busy) return;
    // La page remplace le fil affiché : le fil, son brouillon et le menu de conversation restent à l’écart.
    if (activeId()) panels.leaveConversation?.();
    openTopicId = topicId;
    collapsed.delete(topicId);
    writeCollapsed();
    painted.page = null;
    paint();
    byId("topic-page-title")?.focus?.();
  }

  function paintSidebar() {
    const host = byId("topic-list");
    if (!host) return;
    const state = latest.state;
    const { query, favorites } = searchState();
    const filtering = Boolean(query) || favorites;
    const now = Date.now();
    const active = activeId();
    const folders = [];
    for (const topic of topicsOf(state)) {
      const nameHit = Boolean(query) && topic.name.toLocaleLowerCase("fr").includes(query);
      const threads = threadsOf(state, topic.id)
        .filter((conversation) => (!favorites || conversation.favorite) && (!query || nameHit || matches(conversation, query)));
      if (filtering && !threads.length) continue;
      folders.push({
        topic,
        open: filtering || !collapsed.has(topic.id),
        page: openTopicId === topic.id,
        total: threadsOf(state, topic.id).length,
        threads: threads.map((conversation) => ({ conversation, time: relative(conversation.updated_at, now), active: conversation.id === active })),
      });
    }
    const signature = JSON.stringify([filtering, folders.map((folder) => [folder.topic.id, folder.topic.name, folder.topic.description, folder.open, folder.page, folder.total,
      folder.threads.map((row) => [row.conversation.id, row.conversation.title, row.conversation.kind, row.time, row.active])])]);
    repaint(host, signature, "sidebar", () => {
      if (!folders.length) return filtering ? [] : [node("p", "sidebar-empty topic-empty", "Regroupez plusieurs fils sous un sujet.")];
      return folders.map(sidebarFolder);
    });
  }

  function sidebarFolder(folder) {
    const { topic } = folder;
    const wrapper = node("div", "topic-folder");
    wrapper.dataset.topicId = topic.id;
    const row = node("div", `topic-row${folder.page ? " active" : ""}`);
    const listId = `topic-threads-${topic.id}`;
    const chevron = button("topic-toggle", "", () => toggle(topic.id));
    chevron.dataset.focusKey = `toggle:${topic.id}`;
    chevron.setAttribute("aria-expanded", String(folder.open));
    chevron.setAttribute("aria-controls", listId);
    chevron.setAttribute("aria-label", `${folder.open ? "Replier" : "Déplier"} le sujet ${topic.name}`);
    chevron.append(icon("chevron", "icon topic-chevron"), icon("folder", "icon topic-folder-icon"));
    const name = button("topic-name", topic.name, () => showPage(topic.id));
    name.dataset.focusKey = `open:${topic.id}`;
    name.title = topic.description ? `${topic.name} — ${topic.description}` : topic.name;
    if (folder.page) name.setAttribute("aria-current", "page");
    row.append(chevron, name, node("span", "topic-count", String(folder.total)));
    wrapper.append(row);
    if (folder.open) {
      const list = node("div", "topic-threads");
      list.id = listId;
      list.setAttribute("role", "group");
      list.setAttribute("aria-label", `Fils du sujet ${topic.name}`);
      if (!folder.threads.length) list.append(node("p", "topic-threads-empty", "Aucun fil pour l’instant."));
      for (const { conversation, time, active } of folder.threads) {
        const item = node("button", `conversation-item topic-thread${active ? " active" : ""}`);
        item.type = "button";
        item.dataset.conversationId = conversation.id;
        item.dataset.focusKey = `thread:${conversation.id}`;
        item.title = `${conversation.title} · ${conversation.kind === "group" ? "Groupe" : botName(latest.state, conversation.bot_id)}`;
        if (active) item.setAttribute("aria-current", "page");
        item.append(node("span", "conversation-name", conversation.title || "Nouvelle conversation"), node("small", "topic-thread-time", time));
        item.addEventListener("click", () => { if (!panels.busy) panels.openConversation(conversation.id); });
        list.append(item);
      }
      wrapper.append(list);
    }
    return wrapper;
  }

  function ensurePage() {
    let page = byId("topic-page");
    if (page) return page;
    page = node("section", "topic-page");
    page.id = "topic-page";
    page.hidden = true;
    const dock = document.querySelector?.(".composer-dock");
    if (dock?.parentElement) dock.parentElement.insertBefore(page, dock);
    else (document.querySelector?.(".main-panel") || document.body).append(page);
    return page;
  }

  function paintPage() {
    const page = ensurePage();
    const main = document.querySelector?.(".main-panel");
    const topic = openTopic();
    if (openTopicId && !topic && latest.state?.topics) openTopicId = null;
    if (!topic) {
      page.hidden = true;
      if (main?.dataset) delete main.dataset.view;
      painted.page = null;
      return;
    }
    page.hidden = false;
    if (main?.dataset) main.dataset.view = "topic";
    // Le haut de page porte le sujet, comme il porte le titre d’un fil.
    const title = byId("conversation-title");
    if (title) title.textContent = topic.name;
    const label = byId("workspace-label");
    if (label) { label.textContent = "Sujet"; label.disabled = true; label.removeAttribute?.("title"); }
    if (document.title !== undefined) document.title = `${topic.name} · Ezerbot`;
    const threads = threadsOf(latest.state, topic.id);
    const now = Date.now();
    const rows = threads.map((conversation) => [conversation.id, conversation.title, conversation.kind, conversation.bot_id, relative(conversation.updated_at, now)]);
    const signature = JSON.stringify([topic.id, topic.name, topic.description, rows]);
    repaint(page, signature, "page", () => pageContent(topic, threads, now));
  }

  function pageContent(topic, threads, now) {
    const body = node("div", "topic-page-main");
    const header = node("header", "topic-page-header");
    const heading = node("div", "topic-page-heading");
    const title = node("h2", "topic-page-title", topic.name);
    title.id = "topic-page-title";
    title.tabIndex = -1;
    heading.append(title);
    if (topic.description) heading.append(node("p", "topic-page-description", topic.description));
    const actions = node("div", "topic-page-actions");
    const rename = button("secondary-button topic-action", "Renommer", () => openForm({ topic }));
    rename.dataset.focusKey = "rename";
    const remove = button("secondary-button topic-action topic-delete", "Supprimer", () => openDelete(topic));
    remove.dataset.focusKey = "delete";
    actions.append(rename, remove);
    header.append(heading, actions);

    const bar = node("div", "topic-threads-bar");
    const count = threads.length;
    bar.append(node("h3", "topic-section-title", count === 1 ? "1 fil" : `${count} fils`));
    const create = button("primary-button topic-new-thread", "Nouveau fil", () => run(() => newThread(topic)));
    create.dataset.focusKey = "new-thread";
    create.prepend(icon("plus"));
    bar.append(create);

    const list = node("div", "topic-page-threads");
    if (!threads.length) list.append(node("p", "topic-page-empty", "Aucun fil dans ce sujet. « Nouveau fil » en ouvre un déjà rangé ici."));
    for (const conversation of threads) {
      const row = node("button", "topic-page-thread");
      row.type = "button";
      row.dataset.conversationId = conversation.id;
      row.dataset.focusKey = `thread:${conversation.id}`;
      const who = conversation.kind === "group" ? "Groupe" : botName(latest.state, conversation.bot_id);
      row.append(node("span", "topic-page-thread-title", conversation.title || "Nouvelle conversation"),
        node("small", "topic-page-thread-meta", `${who} · ${relative(conversation.updated_at, now)}`));
      row.addEventListener("click", () => { if (!panels.busy) panels.openConversation(conversation.id); });
      list.append(row);
    }
    body.append(header, bar, list);

    // Réservée à la mémoire partagée du sujet : vide tant qu’aucun fait n’est retenu.
    const memory = node("aside", "topic-memory");
    memory.setAttribute("aria-label", "Mémoire du sujet");
    memory.append(node("h3", "topic-section-title", "Mémoire du sujet"), node("p", "topic-memory-empty", "Aucun fait retenu pour l’instant."));
    return [body, memory];
  }

  async function newThread(topic) {
    if (panels.busy) return;
    const bot = latest.context?.bot;
    const input = { topic_id: topic.id };
    if (bot && !bot.deleted) input.bot_id = bot.id;
    const { conversation } = await panels.api("/api/conversations", input);
    await panels.refresh();
    panels.openConversation(conversation.id);
  }

  // Les dialogues naissent au premier usage et servent ensuite à chaque ouverture.
  const dialogs = {};
  function dialog(key, build) {
    if (!dialogs[key]) {
      const el = node("dialog", "dialog small-dialog topic-dialog");
      el.id = `topic-${key}-dialog`;
      build(el);
      document.body.append(el);
      dialogs[key] = el;
    }
    return dialogs[key];
  }
  const closeButton = (root) => {
    const close = button("icon-button", "", () => root.close());
    close.setAttribute("aria-label", "Fermer");
    close.append(icon("close"));
    return close;
  };
  // Ce que fait « Enregistrer » ou « Supprimer » dépend du sujet ouvert dans le dialogue partagé.
  let submitForm = null;
  let confirmDelete = null;

  function openForm({ topic = null, then = null } = {}) {
    const el = dialog("form", (root) => {
      const form = node("form");
      form.id = "topic-form";
      const header = node("div", "dialog-header");
      const heading = node("h2", "", "Nouveau sujet");
      heading.id = "topic-form-title";
      root.setAttribute("aria-labelledby", "topic-form-title");
      header.append(heading, closeButton(root));
      const nameLabel = node("label", "form-label", "Nom du sujet");
      nameLabel.setAttribute("for", "topic-name-input");
      const name = node("input", "text-input");
      name.id = "topic-name-input";
      name.setAttribute("maxlength", "80");
      name.setAttribute("autocomplete", "off");
      name.required = true;
      const descriptionLabel = node("label", "form-label topic-form-spaced", "Description (facultative)");
      descriptionLabel.setAttribute("for", "topic-description-input");
      const description = node("textarea", "text-input");
      description.id = "topic-description-input";
      description.setAttribute("maxlength", "280");
      description.setAttribute("rows", "3");
      const footer = node("div", "dialog-footer");
      const cancel = button("secondary-button", "Annuler", () => root.close());
      const save = node("button", "primary-button", "Enregistrer");
      save.type = "submit";
      footer.append(cancel, save);
      form.append(header, nameLabel, name, descriptionLabel, description, footer);
      root.append(form);
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        submitForm?.();
      });
    });
    el.querySelector("h2").textContent = topic ? "Renommer le sujet" : "Nouveau sujet";
    el.querySelector("#topic-name-input").value = topic?.name || "";
    el.querySelector("#topic-description-input").value = topic?.description || "";
    submitForm = async () => {
      const name = el.querySelector("#topic-name-input").value.trim();
      const description = el.querySelector("#topic-description-input").value.trim();
      if (!name) { el.querySelector("#topic-name-input").focus(); return; }
      const saved = await run(async () => {
        if (topic) await panels.api("/api/topics/update", { id: topic.id, name, description });
        else {
          const created = await panels.api("/api/topics", description ? { name, description } : { name });
          await then?.(created.topic);
        }
        await panels.refresh();
        return true;
      });
      if (saved) el.close();
    };
    el.showModal();
    el.querySelector("#topic-name-input").select?.();
  }

  function openDelete(topic) {
    const el = dialog("delete", (root) => {
      const header = node("div", "dialog-header");
      const heading = node("h2", "", "Supprimer ce sujet ?");
      heading.id = "topic-delete-title";
      root.setAttribute("aria-labelledby", "topic-delete-title");
      header.append(heading);
      const text = node("p", "delete-description");
      text.id = "topic-delete-description";
      const footer = node("div", "dialog-footer");
      const confirm = button("danger-button", "Supprimer", () => confirmDelete?.());
      confirm.id = "topic-confirm-delete";
      footer.append(button("secondary-button", "Annuler", () => root.close()), confirm);
      root.append(header, text, footer);
    });
    const count = threadsOf(latest.state, topic.id).length;
    el.querySelector("#topic-delete-description").textContent = count
      ? `Les ${count} fils de « ${topic.name} » ne sont pas supprimés : ils repassent dans Conversations, sans sujet.`
      : `« ${topic.name} » est vide. Aucune conversation n’est touchée.`;
    confirmDelete = async () => {
      const removed = await run(async () => {
        await panels.api("/api/topics/delete", { id: topic.id });
        if (openTopicId === topic.id) openTopicId = null;
        painted.page = null;
        await panels.refresh();
        return true;
      });
      if (removed) el.close();
    };
    el.showModal();
  }

  async function assign(conversation, topicId) {
    return run(async () => {
      await panels.api("/api/topics/assign", { conversation_id: conversation.id, topic_id: topicId });
      await panels.refresh();
      return true;
    });
  }

  // « Classer dans un sujet » : choisir le sujet du fil affiché, ou aucun.
  function openAssign(conversation) {
    const el = dialog("assign", (root) => {
      const header = node("div", "dialog-header");
      const heading = node("h2", "", "Classer dans un sujet");
      heading.id = "topic-assign-title";
      root.setAttribute("aria-labelledby", "topic-assign-title");
      header.append(heading, closeButton(root));
      const list = node("div", "topic-choices");
      list.id = "topic-choices";
      root.append(header, list);
    });
    const current = membership(latest.state)[conversation.id] || null;
    const choose = async (topicId) => { if (await assign(conversation, topicId)) el.close(); };
    const choice = (label, topicId, selected) => {
      const option = button(`topic-choice${selected ? " selected" : ""}`, "", () => choose(topicId));
      option.setAttribute("aria-pressed", String(selected));
      option.append(node("span", "", label));
      if (selected) option.append(icon("check"));
      return option;
    };
    const choices = [choice("Aucun sujet", null, current === null)];
    for (const topic of topicsOf(latest.state)) choices.push(choice(topic.name, topic.id, topic.id === current));
    const create = button("topic-choice topic-choice-new", "", () => {
      el.close();
      openForm({ then: (created) => assign(conversation, created.id) });
    });
    create.append(icon("plus"), node("span", "", "Nouveau sujet…"));
    choices.push(create);
    el.querySelector("#topic-choices").replaceChildren(...choices);
    el.showModal();
  }

  function paintMenu() {
    const entry = byId("topic-button");
    const label = entry?.querySelector?.("span");
    if (!label) return;
    const inTopic = Boolean(membership(latest.state)[activeId()]);
    label.textContent = inTopic ? "Changer de sujet…" : "Classer dans un sujet…";
  }

  function paint() {
    paintSidebar();
    paintPage();
    paintMenu();
  }

  const registered = panels.register({
    id: "topics",
    onState(state, context) {
      latest = { state, context };
      const active = context?.conversation?.id || null;
      // Ouvrir un fil rangé dans un sujet replié le déplie, une seule fois.
      if (active && active !== lastActiveId) {
        const topicId = membership(state)[active];
        if (topicId && collapsed.delete(topicId)) writeCollapsed();
      }
      lastActiveId = active;
      paint();
    },
    // Tout changement d’écran (fil, accueil, lien) ferme la page du sujet.
    onNavigate() {
      if (!openTopicId) return;
      openTopicId = null;
      painted.page = null;
      paint();
    },
    assign: openAssign,
  });
  if (!registered) return;

  const section = byId("sidebar-topics");
  if (section) section.hidden = false;
  byId("new-topic")?.addEventListener("click", () => openForm());
  // Les filtres de la liste valent aussi pour les fils rangés dans un sujet.
  byId("search")?.addEventListener("input", paintSidebar);
  byId("favorites-filter")?.addEventListener("click", paintSidebar);
  // Les temps relatifs avancent même sans nouvel état.
  setInterval(() => { if (latest.state) paint(); }, 60000);
  window.GrokbotTopics = { relative, threadsOf, openTopic: showPage, get openTopicId() { return openTopicId; } };
})();
