/* Mémoire : recherche et filtres par bot et par portée. État du filtre `{ q, bot, scope }` ; `filtre` rend les ids correspondants,
 * que la Liste garde et que le Graphe met en avant. `normalize` est aussi celle de memory.js (ancien panneau). Le bundle des panneaux
 * est trié par nom : ce fichier passe avant memory-graph.js, memory-space.js et memory.js. */
(() => {
  "use strict";
  const SCOPES = [["", "Toutes les portées"], ["private", "Privé"], ["team", "Équipe"], ["topic", "Sujet"]];
  const normalize = (text) => String(text ?? "").normalize("NFD").replace(/[̀-ͯ]/g, "").toLocaleLowerCase("fr");
  const scopeOf = (fact) => (fact.scope === "team" || fact.scope === "topic" ? fact.scope : "private");
  const isActive = ({ q = "", bot = "", scope = "" } = {}) => Boolean(String(q).trim() || bot || scope);
  function filtre(facts, { q = "", bot = "", scope = "" } = {}) {
    const needle = normalize(String(q).trim());
    return (facts || [])
      .filter((fact) => (!bot || fact.bot_id === bot) && (!scope || scopeOf(fact) === scope) && normalize(fact.text).includes(needle))
      .map((fact) => String(fact.fact_id));
  }
  // Les bots du menu : ceux de l'état, plus ceux que les souvenirs citent (un bot supprimé reste filtrable).
  function botChoices(facts, bots, label) {
    const found = new Map((bots || []).filter((bot) => !bot.deleted).map((bot) => [bot.id, bot.name]));
    const cited = new Set();
    for (const fact of facts || []) if (fact.bot_id && !cited.has(fact.bot_id)) { cited.add(fact.bot_id); found.set(fact.bot_id, label(fact)); }
    return [...found].map(([id, name]) => ({ id, label: name })).sort((a, b) => a.label.localeCompare(b.label, "fr"));
  }
  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const option = (value, label) => Object.assign(node("option", "", label), { value });
  // La barre d'outils vit hors du corps de l'écran : elle n'est pas reconstruite à chaque frappe, le champ garde le focus.
  function create(onChange) {
    const state = { q: "", bot: "", scope: "" };
    const element = node("div", "memory-filter");
    element.hidden = true;
    element.setAttribute("role", "search");
    element.setAttribute("aria-label", "Filtrer les souvenirs");
    const search = node("input", "text-input"), bot = node("select", "text-input"), scope = node("select", "text-input");
    search.type = "search";
    search.placeholder = "Rechercher…";
    search.setAttribute("aria-label", "Rechercher un souvenir");
    bot.setAttribute("aria-label", "Filtrer par bot");
    scope.setAttribute("aria-label", "Filtrer par portée");
    scope.append(...SCOPES.map(([value, text]) => option(value, text)));
    const count = node("p", "memory-filter-count"), clear = node("button", "secondary-button", "Effacer les filtres");
    count.setAttribute("role", "status");
    clear.type = "button";
    element.append(search, bot, scope, count, clear);
    const read = () => { Object.assign(state, { q: search.value, bot: bot.value, scope: scope.value }); onChange(); };
    search.addEventListener("input", read);
    bot.addEventListener("change", read);
    scope.addEventListener("change", read);
    clear.addEventListener("click", () => { search.value = bot.value = scope.value = ""; read(); search.focus?.(); });
    let shown = null;
    // Met la barre à jour et rend les ids trouvés, ou null quand rien n'est filtré (alors rien ne s'estompe).
    function sync(facts, bots, label) {
      element.hidden = !facts?.length;
      const choices = botChoices(facts, bots, label);
      if (facts && !choices.some((choice) => choice.id === state.bot)) state.bot = "";
      const signature = JSON.stringify(choices);
      if (signature !== shown) { shown = signature; bot.replaceChildren(option("", "Tous les bots"), ...choices.map((choice) => option(choice.id, choice.label))); }
      bot.value = state.bot;
      const ids = isActive(state) ? new Set(filtre(facts, state)) : null;
      count.textContent = ids ? `${ids.size < 2 ? `${ids.size} souvenir` : `${ids.size} souvenirs`} sur ${facts?.length ?? 0}` : "";
      clear.hidden = !ids;
      return ids;
    }
    // Efface la recherche et les filtres (sans les relire : l'appelant repeint) : un écran ouvert pour des souvenirs précis ne les cache pas derrière un filtre d'avant.
    const reset = () => { search.value = bot.value = scope.value = ""; Object.assign(state, { q: "", bot: "", scope: "" }); };
    return { element, state, sync, reset };
  }
  window.EzerbotMemoryFilter = { normalize, filtre, botChoices, create };
})();
