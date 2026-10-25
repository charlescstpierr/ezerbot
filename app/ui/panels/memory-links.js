/* Mémoire : les liens entre souvenirs. Deux souvenirs sont liés s'ils viennent de la même conversation (`provenance.conversation_id`).
 * Le lien se dérive à chaque lecture de la liste de `GET /api/memory/all` : aucun lien ni thème n'est stocké, et un souvenir sans
 * conversation n'est lié à rien. Les souvenirs de sujet portent la conversation où ils ont été appris : ils suivent la même règle.
 * Utilisé par memory-graph.js (traits, taille des points, survol) et memory-detail.js (« Lié à »). */
(() => {
  "use strict";
  const conversationOf = (fact) => String(fact?.provenance?.conversation_id ?? "").trim();
  // Les identifiants de chaque conversation qui en réunit au moins deux, dans l'ordre de la liste.
  function conversations(facts) {
    const found = new Map();
    for (const fact of facts || []) {
      const key = conversationOf(fact);
      if (key) found.set(key, [...(found.get(key) || []), String(fact.fact_id)]);
    }
    return [...found.values()].filter((ids) => ids.length > 1);
  }
  // id → nombre de souvenirs liés (tous ceux de sa conversation, lui exclu) ; absent de la table : isolé. Coût linéaire.
  const counts = (facts) => new Map(conversations(facts).flatMap((ids) => ids.map((id) => [id, ids.length - 1])));
  // Les traits à dessiner : une chaîne par conversation, dans l'ordre de la liste (n − 1 traits pour n souvenirs). Ils relient tous les
  // souvenirs d'une conversation sans tracer les n(n−1)/2 paires : une conversation de 500 souvenirs resterait lisible et dessinable.
  const pairs = (facts) => conversations(facts).flatMap((ids) => ids.slice(1).map((id, i) => [ids[i], id]));
  // Les souvenirs liés à `fact` (même conversation, lui exclu), dans l'ordre de la liste. Ne parcourt la liste qu'une fois.
  function of(facts, fact) {
    const key = conversationOf(fact), self = String(fact?.fact_id);
    return key ? (facts || []).filter((item) => String(item.fact_id) !== self && conversationOf(item) === key) : [];
  }
  // Un point de 6 grandit d'un demi-point par lien, jusqu'à 9 : plus lié, un peu plus gros.
  const radius = (count) => 6 + Math.min(count || 0, 6) / 2;
  const text = (fact) => { const chars = [...String(fact?.text ?? "").replace(/\s+/g, " ").trim()]; return chars.length > 80 ? `${chars.slice(0, 79).join("")}…` : chars.join(""); };
  // La section « Lié à » du détail : un bouton par souvenir lié, `open(id)` l'ouvre. Le texte passe par textContent.
  function section(linked, open) {
    const make = (tag, className, content) => Object.assign(document.createElement(tag), { className, textContent: content ?? "" });
    const root = make("section", "memory-detail-links"), list = make("ul", "memory-detail-linked");
    root.setAttribute("aria-label", "Lié à");
    for (const fact of linked) {
      const item = document.createElement("li"), button = Object.assign(make("button", "memory-detail-link-item", text(fact)), { type: "button" });
      button.addEventListener("click", () => open(String(fact.fact_id)));
      item.append(button);
      list.append(item);
    }
    root.append(make("h3", "memory-detail-links-title", "Lié à"), list);
    return root;
  }
  window.EzerbotMemoryLinks = { counts, conversationOf, pairs, of, radius, section };
})();
