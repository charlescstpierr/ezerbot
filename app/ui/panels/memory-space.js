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
  let mark = null, painted = null, entry = null, screen = null;
  async function load() {
    const token = ++generation;
    try {
      const response = await panels.api("/api/memory/all");
      if (token !== generation) return;
      facts = Array.isArray(response?.facts) ? response.facts : [];
      failed = false;
    } catch {
      if (token !== generation) return;
      failed = true;
    }
    paint();
  }
  function show() {
    if (panels.busy) return;
    // Quitter l’écran courant (fil ou page de sujet) passe par la navigation, qui ferme les autres pages : d’où cet ordre.
    panels.leaveConversation?.();
    open = true;
    painted = null;
    load();
    paint();
    screen.querySelector(".is-list").focus?.();
  }
  function close() {
    if (!open) return;
    open = false;
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
    entry.addEventListener("click", show);
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
      if (key === "graph") tab.append(node("small", "memory-space-soon", "bientôt"));
      tab.addEventListener("click", () => { view = key; paint(); });
      switcher.append(tab);
    }
    screen.append(switcher, node("p", "memory-space-help", "Tout ce que vos bots ont retenu : souvenirs privés, d’équipe et de sujet."), node("div", "memory-space-body"));
    dock.parentElement.insertBefore(screen, dock);
    return true;
  }
  function body() {
    const empty = (text) => [node("p", "memory-space-empty", text)];
    if (view === "graph") return empty("Le graphe de la mémoire arrive bientôt. La Liste montre déjà tous les souvenirs.");
    if (!facts) return empty(failed ? "La mémoire n’a pas pu être chargée." : "Chargement…");
    const note = failed ? [node("p", "memory-space-help", "La mémoire n’a pas pu être actualisée : voici le dernier état connu.")] : [];
    if (!facts.length) return [...note, ...empty("Aucun souvenir pour l’instant. Vos bots retiendront ici ce qui compte pour vous.")];
    const list = node("ul", "memory-space-list");
    list.setAttribute("aria-label", "Souvenirs");
    for (const fact of facts) {
      const item = node("li", "memory-space-item");
      item.dataset.factId = fact.fact_id;
      const meta = node("div", "memory-space-meta");
      meta.append(node("span", "memory-space-chip memory-space-bot", botLabel(fact)), node("span", "memory-space-chip memory-space-scope", scopeLabel(fact)), node("small", "memory-space-date", factDate(fact.learned_at)));
      item.append(node("p", "memory-space-text", String(fact.text ?? "")), meta);
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
    const signature = JSON.stringify([view, failed, facts, (latest?.state?.bots || []).map((bot) => [bot.id, bot.name])]);
    if (signature === painted) return;
    painted = signature;
    screen.querySelector(".memory-space-body").replaceChildren(...body());
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
})();
