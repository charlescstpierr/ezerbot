/* Mémoire : le panneau de détail d'un souvenir (texte, bot, origine, date, provenance, portée) avec Modifier et Oublier.
 * Rien n'est mis à jour d'avance : après chaque action la liste est relue, et les erreurs du moteur s'affichent telles quelles.
 * memory-space.js le monte ; il passe avant lui dans le bundle (ordre des noms) et se trouve sur window. Le texte passe par textContent. */
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
  const SOURCES = { user_stated: "Déclaré", model_inferred: "Déduit", automatic: "Automatique", imported: "Importé" };
  const REACH = { team: " Il sera aussi retiré pour tous les bots de l’espace.", topic: " Il sera aussi retiré pour tous les bots du sujet." };
  // Deux routes d'oubli : un fait de sujet n'a pas de bot propriétaire, la sienne ne prend que son identifiant.
  const forgetRequest = (fact) => (fact.scope === "topic" ? ["/api/topics/memory/forget", { fact_id: fact.fact_id }] : ["/api/memory/forget", { bot_id: fact.bot_id, fact_id: fact.fact_id }]);

  // `facts()` donne la liste lue par l'écran, `reload()` la relit (et rend la main une fois l'écran à jour), `label` : { bot, scope, date }.
  function create({ facts, reload, label }) {
    let id = null, mode = "view", busy = false, draft = "", note = "", problem = "", shown = null, epoch = 0;
    const moved = new Map(); // ancien identifiant → identifiant du souvenir qui le remplace (réponse du moteur à une correction)
    const root = node("aside", "memory-detail");
    root.hidden = true;
    root.setAttribute("aria-label", "Détail du souvenir");
    const current = () => (facts() || []).find((item) => item.fact_id === id) || null;
    const find = (key) => root.parentElement?.querySelector(`[data-id="${key}"], [data-fact-id="${key}"]`);
    const mark = () => {
      for (const el of root.parentElement.querySelectorAll(".is-selected")) el.classList.remove("is-selected");
      if (id !== null) find(id)?.classList.add("is-selected");
    };
    const button = (text, className, action, disabled = busy) => {
      const el = Object.assign(node("button", className, text), { type: "button", disabled });
      el.addEventListener("click", action);
      return el;
    };
    function render(want = "") {
      const fact = current();
      if (!fact) return;
      shown = JSON.stringify(fact);
      const editing = mode === "edit", confirming = mode === "confirm", conversation = fact.provenance?.conversation_id;
      const origin = conversation ? button("Ouvrir la conversation", "memory-detail-link", () => { if (!panels.openConversation?.(conversation)) { problem = "Cette conversation n’existe plus, ou une autre action est en cours."; render("origin"); } }, false) : "Hors conversation";
      const rows = [["Bot", label.bot(fact)], ["Origine", origin], ["Appris le", label.date(fact.learned_at)], ["Provenance", SOURCES[fact.provenance?.source] || "Inconnue"], ["Portée", label.scope(fact)]];
      const fields = node("dl", "memory-detail-fields");
      for (const [name, value] of rows) {
        const cell = node("dd");
        cell.append(value);
        fields.append(node("dt", "", name), cell);
      }
      const title = Object.assign(node("h2", "memory-detail-title", "Souvenir"), { tabIndex: -1 });
      const head = node("div", "memory-detail-head");
      head.append(title, button("Fermer", "secondary-button", () => close(true), false));
      const save = button("Enregistrer", "primary-button", () => act("/api/memory/correct", { bot_id: fact.bot_id, fact_id: fact.fact_id, text: draft }, "Souvenir modifié."), busy || !draft.trim());
      let body = node("p", "memory-detail-text", String(fact.text ?? ""));
      if (editing) {
        body = Object.assign(node("textarea", "text-input memory-detail-field"), { rows: 5, value: draft, disabled: busy });
        body.setAttribute("aria-label", "Texte du souvenir");
        body.addEventListener("input", () => { draft = body.value; save.disabled = busy || !draft.trim(); });
      }
      const back = button("Annuler", "secondary-button", () => { mode = "view"; problem = ""; render(editing ? "edit" : "ask"); });
      const edit = button("Modifier", "secondary-button", () => { mode = "edit"; draft = String(fact.text ?? ""); problem = ""; render("body"); });
      const ask = button("Oublier…", "secondary-button", () => { mode = "confirm"; problem = ""; render("back"); });
      const sure = button("Oublier ce souvenir", "danger-button", () => act(...forgetRequest(fact), "Souvenir oublié.", true));
      const actions = node("div", "memory-detail-actions");
      actions.append(...(editing ? [save, back] : confirming ? [sure, back] : fact.scope === "topic" ? [ask] : [edit, ask]));
      const hint = confirming ? `Oublier ce souvenir ? Son texte sera effacé pour de bon, avec ses versions précédentes ; seule une trace sans contenu (identifiant et dates) reste dans l’historique.${REACH[fact.scope] || ""}`
        : !editing && fact.scope === "topic" ? "Un souvenir de sujet ne se modifie pas : oubliez-le, puis ajoutez-en un autre." : "";
      const status = node("p", "memory-detail-note", note), alert = node("p", "memory-detail-error", problem);
      status.setAttribute("role", "status");
      alert.setAttribute("role", "alert");
      root.replaceChildren(head, body, fields, node("p", confirming ? "memory-detail-warning" : "memory-detail-note", hint), status, alert, actions);
      root.hidden = false;
      root.parentElement.dataset.detail = "open";
      mark();
      ({ title, body, back, edit, ask, origin })[want]?.focus();
    }
    // Le panneau suit le moteur : il ne montre que ce que la dernière lecture contient.
    async function act(path, request, done, forgetting = false) {
      const mine = epoch;
      busy = true;
      problem = "";
      render();
      let data;
      try {
        data = await panels.api(path, request);
      } catch (error) {
        if (mine !== epoch) return;
        busy = false;
        problem = error?.message || "L’action n’a pas abouti.";
        render(mode === "edit" ? "body" : "back");
        return;
      }
      if (data?.fact?.fact_id) moved.set(request.fact_id, data.fact.fact_id); // corriger remplace le souvenir par un nouveau
      if (mine === epoch) { mode = "view"; note = done; draft = ""; }
      await reload();
      refresh(); // la liste relue peut être identique à la précédente : l'écran n'appelle alors pas refresh
      if (mine !== epoch) return;
      busy = false;
      if (!current()) { close(true); return; } // oublié, et la relecture le confirme
      // Toujours là après la relecture : elle a échoué, et le panneau le dit au lieu de se taire.
      if (current()?.fact_id === request.fact_id) note = `${done.slice(0, -1)}, mais la liste n’a pas pu être relue : actualisez-la.`;
      render(forgetting ? "title" : "edit");
    }
    function close(restore = false) {
      const was = id;
      if (was === null) return;
      [id, shown, root.hidden] = [null, null, true];
      epoch += 1;
      root.replaceChildren();
      mark();
      delete root.parentElement.dataset.detail;
      const back = find(was) || root.parentElement.querySelector('.memory-space-tab[aria-pressed="true"]');
      if (restore) (back?.querySelector("button") || back)?.focus(); // une ligne de la Liste se focalise par son bouton
    }
    // Appelé par l'écran quand la liste a changé : un souvenir disparu ferme le panneau ; la saisie en cours n'est jamais écrasée.
    function refresh() {
      if (id === null) return;
      if (!current() && (facts() || []).some((item) => item.fact_id === moved.get(id))) id = moved.get(id);
      const fact = current();
      if (!fact && busy) return; // l'action en cours décide, une fois sa réponse reçue
      if (!fact) close(!document.activeElement || document.activeElement === document.body || root.contains(document.activeElement));
      else if (JSON.stringify(fact) !== shown) render();
      else mark(); // la vue a pu être reconstruite (Graphe ↔ Liste)
    }
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || id === null || event.defaultPrevented || document.querySelector("dialog[open]")) return;
      event.preventDefault();
      close(true);
    });
    function open(factId) {
      epoch += 1;
      [id, mode, busy, draft, note, problem] = [factId, "view", false, "", "", ""];
      render("title");
    }
    // Un clic n'importe où sur la carte ouvre le souvenir ; au clavier, le bouton du texte s'en charge.
    const bind = (el, factId) => el.addEventListener("click", () => open(factId));
    return { element: root, open, close, refresh, bind };
  }
  window.EzerbotMemoryDetail = { create };
})();
