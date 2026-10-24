/* Mémoire : l’entrée de la barre latérale et l’écran « Espace personnel / Mémoire » (vue Liste).
 * L’ancien panneau par bot (memory.js) reste : il porte Modifier, Oublier, le partage et la mémoire automatique. */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const byId = (id) => document.getElementById(id);
  const noun = (count) => (count < 2 ? `${count} souvenir` : `${count} souvenirs`);
  const factDate = (value) => new Date(Number(value) < 1e12 ? value * 1000 : value).toLocaleDateString("fr-CA", { day: "numeric", month: "short", year: "numeric" });
  let latest = null; // { state } du dernier onState
  let facts = null; // null tant que la liste n’est pas arrivée
  let failed = false, open = false, view = "list", generation = 0;
  let mark = null, painted = null, entry = null, screen = null, detail = null, found = null; // found : ids trouvés (Set), null sans filtre
  // Ouvert depuis « N souvenirs utilisés » sous une réponse : `recalled` attend la liste relue, `used` garde la demande
  // ({ origin, ask, all }) ; ce qui existe encore s'en déduit à chaque affichage (`scopeOf`), car un souvenir peut être oublié écran ouvert.
  let recalled = null, used = null, usedBar = null;
  const filter = window.EzerbotMemoryFilter?.create(() => paint());
  let loading = Promise.resolve();
  // `load` rend la main quand la liste est à jour : une lecture dépassée par une plus récente attend celle-ci, sans quoi un appelant (le panneau de détail) croirait la liste relue.
  function load() {
    const token = ++generation;
    return (loading = read(token));
  }
  async function read(token) {
    try {
      const response = await panels.api("/api/memory/all");
      if (token !== generation) return loading;
      facts = Array.isArray(response?.facts) ? response.facts : [];
      failed = false;
    } catch {
      if (token !== generation) return loading;
      failed = true;
    }
    paint();
    if (failed) recalled = null; // une liste qui n'a pas pu être lue ne dit pas quels souvenirs existent
    else settle();
  }
  // Les souvenirs de la réponse qui existent encore, d'après la liste qui vient d'être relue : le premier s'ouvre, les disparus sont dits.
  function settle() {
    if (!recalled || !open) return;
    const { ids, origin } = recalled;
    recalled = null;
    used = { origin, ask: ids, all: false };
    view = "list";
    paint();
    const first = scopeOf(used).present[0];
    if (first) detail?.open(first);
    else detail?.close();
  }
  // Ce que la demande vaut d'après la liste lue : les souvenirs qui existent encore (`present`, dans l'ordre du rappel), ceux qui manquent,
  // et `ids` (Set) quand la Liste ne doit montrer qu'eux : plusieurs souvenirs demandés, au moins un présent, « Tout afficher » pas encore pressé.
  function scopeOf({ ask, all, origin }) {
    const live = new Set((facts || []).map((fact) => String(fact.fact_id)));
    const present = ask.filter((id) => live.has(id));
    return { origin, total: ask.length, present, missing: ask.length - present.length, ids: !all && ask.length > 1 && present.length ? new Set(present) : null };
  }
  function openWith(ids, origin = null) {
    if (panels.busy) return { ok: false, message: "Une action est en cours : réessayez dans un instant." };
    if (!Array.isArray(ids) || !ids.length) return { ok: false, message: "Cette réponse n’a pas de souvenir à montrer." };
    show(); // quitter le fil passe par la navigation, qui ferme cet écran : on retient la demande ensuite
    recalled = { ids: ids.map(String), origin };
    used = null;
    paint();
    return { ok: true };
  }
  function show() {
    if (panels.busy) return;
    // Quitter l’écran courant (fil ou page de sujet) passe par la navigation, qui ferme les autres pages : d’où cet ordre.
    panels.leaveConversation?.();
    open = true;
    painted = null;
    load();
    paint();
    screen.querySelector(`.is-${view}`).focus?.(); // l'onglet actif : sur « Liste » alors que « Graphe » est affiché, l'anneau passerait pour un état actif
  }
  function close() {
    if (!open) return;
    open = false;
    recalled = used = null;
    detail?.close();
    paint();
  }
  function botLabel(fact) {
    const bot = (latest?.state?.bots || []).find((item) => item.id === fact.bot_id);
    if (bot) return fact.bot_deleted ? `${bot.name} (supprimé)` : bot.name;
    return fact.bot_deleted ? "Bot supprimé" : "Bot";
  }
  const scopeLabel = (fact) => (fact.scope === "topic" ? (fact.topic_name ? `Sujet · ${fact.topic_name}` : "Sujet") : fact.scope === "team" ? "Équipe" : "Privé");
  function mount() {
    const sidebar = byId("sidebar");
    const dock = document.querySelector(".composer-dock");
    if (!sidebar || !dock) return false;
    const wrapper = node("div", "sidebar-memory");
    entry = node("button", "conversation-item memory-entry");
    entry.type = "button";
    entry.append(node("span", "conversation-name", "Mémoire"), node("span", "memory-entry-count"));
    entry.addEventListener("click", () => { recalled = used = null; show(); });
    wrapper.append(entry);
    sidebar.insertBefore(wrapper, sidebar.querySelector(".sidebar-bots"));
    screen = node("section", "memory-space");
    screen.hidden = true;
    const switcher = node("div", "memory-space-switch");
    switcher.setAttribute("role", "group");
    switcher.setAttribute("aria-label", "Affichage de la mémoire");
    for (const [key, label] of [["graph", "Graphe"], ["list", "Liste"]]) {
      const tab = node("button", `memory-space-tab is-${key}`, label);
      tab.type = "button";
      tab.dataset.view = key;
      tab.addEventListener("click", () => { view = key; paint(); });
      switcher.append(tab);
    }
    // Le bouton « Ajouter un souvenir » (memory-add.js) rejoint les onglets ; son formulaire s’ouvre dessous.
    const add = window.EzerbotMemoryAdd?.create(load);
    // Le panneau de détail (memory-detail.js) s'ouvre à droite de la vue ; il fait relire la liste après chaque action.
    detail = window.EzerbotMemoryDetail?.create({ facts: () => facts, reload: load, label: { bot: botLabel, scope: scopeLabel, date: factDate } });
    usedBar = node("div", "memory-space-used");
    usedBar.hidden = true;
    const head = node("div", "memory-space-head");
    head.append(switcher, ...(add ? [add.button] : []));
    screen.append(head, ...(add ? [add.form] : []), node("p", "memory-space-help", "Tout ce que vos bots ont retenu : souvenirs privés, d’équipe et de sujet."), usedBar, ...(filter ? [filter.element] : []), node("div", "memory-space-body"), ...(detail ? [detail.element] : []));
    dock.parentElement.insertBefore(screen, dock);
    return true;
  }
  // Le bandeau de « N souvenirs utilisés » : ce que la réponse a rappelé, ce qui n'existe plus, et le chemin du retour à la réponse.
  let usedKey = "";
  function paintUsed(scope) {
    usedBar.hidden = !scope;
    const key = scope ? JSON.stringify([scope.total, scope.missing, Boolean(scope.ids), Boolean(scope.origin?.conversationId)]) : "";
    if (key === usedKey) return;
    usedKey = key;
    // Le bandeau est reconstruit quand la liste change : le focus qui y était ne se perd pas avec le bouton remplacé.
    const held = usedBar.contains(document.activeElement) ? document.activeElement.textContent : null;
    const keepFocus = () => {
      if (held === null) return;
      const buttons = [...usedBar.querySelectorAll("button")];
      (buttons.find((item) => item.textContent === held) || buttons[0] || screen.querySelector(`.is-${view}`)).focus?.();
    };
    if (!scope) { usedBar.replaceChildren(); keepFocus(); return; }
    const { total, missing, origin } = scope, plural = (n) => (n < 2 ? "" : "s");
    const text = node("p", "", total === missing
      ? `${total < 2 ? "Ce souvenir n’existe plus : il a été oublié ou remplacé" : "Ces souvenirs n’existent plus : ils ont été oubliés ou remplacés"} depuis cette réponse.`
      : `${noun(total)} utilisé${plural(total)} dans cette réponse.${missing ? ` ${missing} n’${missing < 2 ? "existe" : "existent"} plus (oublié${plural(missing)} ou remplacé${plural(missing)} depuis).` : ""}`);
    text.setAttribute("role", "status");
    const actions = node("div", "memory-space-used-actions");
    const button = (label, action) => {
      const el = Object.assign(node("button", "secondary-button", label), { type: "button" });
      el.addEventListener("click", action);
      return el;
    };
    if (scope.ids) {
      actions.append(button("Tout afficher", () => { used = { ...used, all: true }; paint(); screen.querySelector(`.is-${view}`).focus?.(); }));
    }
    if (origin?.conversationId) {
      actions.append(button("Retour à la réponse", () => {
        if (panels.openConversation?.(origin.conversationId, origin.messageId)) return;
        const problem = node("p", "memory-space-used-problem", "Cette conversation n’existe plus, ou une autre action est en cours.");
        problem.setAttribute("role", "alert");
        usedBar.append(problem);
      }));
    }
    usedBar.replaceChildren(text, ...(actions.children.length ? [actions] : []));
    keepFocus();
  }
  function body(shown) {
    const empty = (text) => [node("p", "memory-space-empty", text)];
    const note = failed ? [node("p", "memory-space-help", "La mémoire n’a pas pu être actualisée : voici le dernier état connu.")] : [];
    // Graphe : sans souvenir (ou pas encore chargé), les messages de la Liste s'appliquent.
    if (found && !found.size && shown?.length) return [...note, ...empty("Aucun souvenir ne correspond à ces filtres.")];
    const graph = view === "graph" && facts?.length ? window.GrokbotMemoryGraph?.view(shown, { bots: latest?.state?.bots || [], matches: found, onSelect: (id) => detail?.open(id) }) : null;
    if (graph) return [...note, graph];
    if (!facts) return empty(failed ? "La mémoire n’a pas pu être chargée." : "Chargement…");
    if (!facts.length) return [...note, ...empty("Aucun souvenir pour l’instant. Vos bots retiendront ici ce qui compte pour vous.")];
    const list = node("ul", "memory-space-list");
    list.setAttribute("aria-label", "Souvenirs");
    for (const fact of shown) {
      if (found && !found.has(String(fact.fact_id))) continue;
      const item = node("li", "memory-space-item");
      item.dataset.factId = fact.fact_id;
      detail?.bind(item, fact.fact_id);
      const meta = node("div", "memory-space-meta");
      meta.append(node("span", "memory-space-chip memory-space-bot", botLabel(fact)), node("span", "memory-space-chip memory-space-scope", scopeLabel(fact)), node("small", "memory-space-date", factDate(fact.learned_at)));
      const text = node("button", "memory-space-text", String(fact.text ?? ""));
      text.type = "button";
      item.append(text, meta);
      list.append(item);
    }
    return [...note, list];
  }
  function paint() {
    if (!entry) return;
    const count = facts ? facts.length : null;
    const main = document.querySelector(".main-panel");
    entry.classList.toggle("active", open);
    if (open) entry.setAttribute("aria-current", "page"); else entry.removeAttribute("aria-current");
    entry.querySelector(".memory-entry-count").textContent = count === null ? "" : String(count);
    entry.setAttribute("aria-label", count === null ? "Mémoire" : `Mémoire, ${noun(count)}`);
    if (open) main.dataset.memorySpace = "list"; else delete main.dataset.memorySpace;
    screen.hidden = !open;
    if (!open) { painted = null; return; }
    // Le haut de page porte « Espace personnel / Mémoire · N souvenirs », comme il porte le titre d’un fil.
    byId("conversation-title").textContent = count === null ? "Mémoire" : `Mémoire · ${noun(count)}`;
    const label = byId("workspace-label");
    label.textContent = "Espace personnel";
    label.disabled = true;
    document.title = "Mémoire · Ezerbot";
    for (const tab of screen.querySelectorAll(".memory-space-tab")) tab.setAttribute("aria-pressed", String(tab.dataset.view === view));
    const scope = used && scopeOf(used);
    paintUsed(scope);
    // Plusieurs souvenirs utilisés : la Liste ne montre qu'eux, le temps de « Tout afficher ».
    const shown = scope?.ids ? (facts || []).filter((fact) => scope.ids.has(String(fact.fact_id))) : facts;
    found = filter ? filter.sync(shown, latest?.state?.bots || [], botLabel) : null;
    const signature = JSON.stringify([view, failed, facts, (latest?.state?.bots || []).map((bot) => [bot.id, bot.name]), filter?.state, scope?.ids ? [...scope.ids] : null]);
    if (signature === painted) return;
    painted = signature;
    screen.querySelector(".memory-space-body").replaceChildren(...body(shown));
    detail?.refresh();
  }
  const registered = panels.register({
    id: "memory-space",
    onState(state) {
      latest = { state };
      // La liste se relit quand la mémoire change (révision automatique, souvenirs de sujet), et toutes les 30 s.
      const next = [state?.memory_auto?.revision, Object.values(state?.topics?.memory || {}).reduce((sum, list) => sum + list.length, 0)].join(":");
      if (next !== mark) { mark = next; load(); }
      paint();
    },
    // Tout changement d’écran (fil, accueil, lien) ferme l’écran Mémoire.
    onNavigate: close,
  });
  if (!registered || !mount()) return;
  paint();
  // La page d’un sujet ne passe pas par la navigation : son ouverture ferme l’écran Mémoire.
  byId("topic-list")?.addEventListener("click", (event) => { if (event.target?.closest?.(".topic-name")) close(); });
  setInterval(load, 30000);
  // Point d'entrée de « N souvenirs utilisés » sous une réponse (app.js) ; absent là où l'écran Mémoire ne se monte pas.
  window.EzerbotMemorySpace = { openWith };
})();
