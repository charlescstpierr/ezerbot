/* Mémoire : le panneau « À revoir ». Les souvenirs que le moteur marque `needs_review` (rien n'est recalculé ici) y figurent avec leur raison
 * et « Encore vrai ? » : Oui (POST /api/memory/reconfirm), Modifier et Oublier (le panneau de détail, memory-detail.js, ouvert sur ces gestes).
 * memory-space.js le monte et lui passe la liste à afficher ; la liste est relue après chaque Oui. Le texte passe par textContent. */
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
  // Depuis quand rien n'a confirmé le souvenir : sa dernière reconfirmation, sinon le jour où il a été appris.
  const lastSeen = (fact) => Math.max(Number(fact.learned_at) || 0, Number(fact.reconfirmed_at) || 0);
  const reason = (fact, date) => (fact.reconfirmed_at ? `Dernière confirmation le ${date(fact.reconfirmed_at)}` : `Appris le ${date(fact.learned_at)}, jamais reconfirmé`);

  // `reload()` relit la liste (et rend la main une fois l'écran à jour), `detail` : le panneau de détail, `label` : { bot, scope, date }.
  function create({ reload, detail, label }) {
    let rows = [], busy = false, note = "", problem = "";
    const root = node("section", "memory-review");
    root.hidden = true;
    root.setAttribute("aria-label", "Souvenirs à revoir");
    const button = (text, className, action, aria) => {
      const el = Object.assign(node("button", `${className} memory-review-button`, text), { type: "button", disabled: busy });
      if (aria) el.setAttribute("aria-label", aria);
      el.addEventListener("click", action);
      return el;
    };
    function draw() {
      const held = root.contains(document.activeElement);
      const title = Object.assign(node("h2", "memory-review-title", rows.length ? `À revoir · ${rows.length} souvenir${rows.length > 1 ? "s" : ""}` : "À revoir"), { tabIndex: -1 });
      const list = node("ul", "memory-review-list");
      list.setAttribute("aria-label", "Souvenirs à revoir");
      for (const fact of rows) {
        const item = node("li", "memory-review-item"), meta = node("div", "memory-space-meta");
        item.dataset.reviewId = fact.fact_id;
        meta.append(node("span", "memory-space-chip", label.bot(fact)), node("span", "memory-space-chip", label.scope(fact)));
        const actions = node("div", "memory-review-actions");
        actions.append(node("span", "memory-review-ask", "Encore vrai ?"), button("Oui", "primary-button", () => confirm(fact), "Oui, ce souvenir est encore vrai"));
        if (detail && fact.scope !== "topic") actions.append(button("Modifier", "secondary-button", () => detail.open(fact.fact_id, "edit")));
        if (detail) actions.append(button("Oublier…", "secondary-button", () => detail.open(fact.fact_id, "confirm")));
        item.append(node("p", "memory-review-text", String(fact.text ?? "")), node("p", "memory-review-reason", reason(fact, label.date)), meta, actions);
        list.append(item);
      }
      const status = node("p", "memory-review-note", note), alert = node("p", "memory-review-error", problem);
      status.setAttribute("role", "status");
      alert.setAttribute("role", "alert");
      root.replaceChildren(title, ...(rows.length ? [node("p", "memory-review-help", "Ces souvenirs n’ont pas été confirmés depuis un moment."), list] : []), status, alert);
      root.hidden = !rows.length && !note && !problem;
      if (held && !root.contains(document.activeElement) && !root.hidden) title.focus(); // la ligne focalisée a disparu : le focus ne se perd pas
    }
    // Le moteur décide : après un Oui, réussi ou non, la liste est relue ; le drapeau levé fait disparaître la ligne et le halo.
    async function confirm(fact) {
      if (busy) return;
      busy = true;
      note = problem = "";
      draw();
      try {
        await panels.api("/api/memory/reconfirm", { fact_id: fact.fact_id });
        note = "Souvenir confirmé.";
      } catch (error) {
        problem = error?.message || "La confirmation n’a pas abouti.";
      }
      await reload();
      busy = false;
      draw();
    }
    // Appelé par l'écran à chaque dessin avec les souvenirs affichés ; seuls ceux que le moteur marque à revoir sont gardés, les plus anciens d'abord.
    function sync(facts) {
      rows = (facts || []).filter((fact) => fact.needs_review === true).sort((a, b) => lastSeen(a) - lastSeen(b));
      draw();
    }
    return { element: root, sync };
  }
  window.EzerbotMemoryReview = { create };
})();
