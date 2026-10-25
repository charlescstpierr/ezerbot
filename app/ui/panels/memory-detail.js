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
    // mode : "view" (lecture), "edit" (édition ; intacte tant que le brouillon égale le texte du souvenir, modifiée sinon),
    // "forget" (confirmation d'oubli) ou "discard" (confirmation d'abandon d'une édition modifiée).
    let id = null, mode = "view", busy = false, draft = "", note = "", problem = "", shown = null, epoch = 0;
    let opener = null; // le bouton d'où le panneau « À revoir » a ouvert le souvenir : la fermeture lui rend le focus
    let after = null; const saved = new Map(); // after : ce que la sortie demandée fera si l'on abandonne ; saved : souvenir → brouillon, pour un écran quitté en pleine modification
    const moved = new Map(); // ancien identifiant → identifiant du souvenir qui le remplace (réponse du moteur à une correction)
    const root = node("aside", "memory-detail");
    root.hidden = true;
    root.setAttribute("aria-label", "Détail du souvenir");
    const stamp = (fact) => JSON.stringify([fact, label.bot(fact), label.scope(fact)]); // un bot renommé change aussi ce que le panneau affiche
    const current = () => (facts() || []).find((item) => item.fact_id === id) || null;
    const find = (key) => root.parentElement?.querySelector(`[data-id="${key}"], [data-fact-id="${key}"]`);
    const mark = () => {
      for (const el of root.parentElement.querySelectorAll(".is-selected")) el.classList.remove("is-selected");
      if (id !== null) find(id)?.classList.add("is-selected");
    };
    const dirty = () => (mode === "edit" || mode === "discard") && draft !== String(current()?.text ?? "");
    // Toute sortie de l'édition (Échap, Fermer, un autre souvenir) passe par ici : une saisie modifiée demande d'abord confirmation, sauf pendant l'envoi.
    function leave(next) {
      if (mode === "edit" && busy) return;
      if (dirty()) { after = next; mode = "discard"; problem = ""; render("keep"); return; }
      drop(next);
    }
    function drop(next) { [mode, draft, problem, after] = ["view", "", "", null]; next?.(); }
    function resume() { [mode, after] = ["edit", null]; render("body"); } // « Continuer » : le texte et le focus restent dans le champ
    const button = (text, className, action, disabled = busy) => {
      const el = Object.assign(node("button", className, text), { type: "button", disabled });
      el.addEventListener("click", action);
      return el;
    };
    function render(want = "") {
      const fact = current();
      if (!fact) return;
      shown = stamp(fact);
      const controls = () => [...root.querySelectorAll("h2, button, textarea")], held = controls().indexOf(document.activeElement); // le focus reste sur le même contrôle quand la liste change autour du panneau
      const discarding = mode === "discard", editing = mode === "edit" || discarding, confirming = mode === "forget", conversation = fact.provenance?.conversation_id;
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
      head.append(title, button("Fermer", "secondary-button", () => leave(() => close(true)), busy && mode === "edit"));
      const save = button("Enregistrer", "primary-button", () => act("/api/memory/correct", { bot_id: fact.bot_id, fact_id: fact.fact_id, text: draft }, "Souvenir modifié."), busy || !draft.trim());
      let body = node("p", "memory-detail-text", String(fact.text ?? ""));
      if (editing) {
        body = Object.assign(node("textarea", "text-input memory-detail-field"), { rows: 5, value: draft, disabled: busy, readOnly: discarding });
        body.setAttribute("aria-label", "Texte du souvenir");
        body.addEventListener("input", () => { draft = body.value; save.disabled = busy || !draft.trim(); });
      }
      const back = button("Annuler", "secondary-button", () => { mode = "view"; problem = ""; render(editing ? "edit" : "ask"); });
      const edit = button("Modifier", "secondary-button", () => { mode = "edit"; draft = String(fact.text ?? ""); problem = ""; render("body"); });
      const ask = button("Oublier…", "secondary-button", () => { mode = "forget"; problem = ""; render("back"); });
      const sure = button("Oublier ce souvenir", "danger-button", () => act(...forgetRequest(fact), "Souvenir oublié.", true));
      const abandon = button("Abandonner", "danger-button", () => drop(after));
      const keep = button("Continuer", "secondary-button", resume);
      const actions = node("div", "memory-detail-actions");
      actions.append(...(discarding ? [abandon, keep] : editing ? [save, back] : confirming ? [sure, back] : fact.scope === "topic" ? [ask] : [edit, ask]));
      const hint = discarding ? "Abandonner la modification ? Le texte saisi n’a pas été enregistré : il sera perdu."
        : confirming ? `Oublier ce souvenir ? Son texte sera effacé pour de bon, avec ses versions précédentes ; seule une trace sans contenu (identifiant et dates) reste dans l’historique.${REACH[fact.scope] || ""}`
        : !editing && fact.scope === "topic" ? "Un souvenir de sujet ne se modifie pas : oubliez-le, puis ajoutez-en un autre." : "";
      const status = node("p", "memory-detail-note", note), alert = node("p", "memory-detail-error", problem);
      status.setAttribute("role", "status");
      alert.setAttribute("role", "alert");
      root.replaceChildren(head, body, fields, node("p", confirming || discarding ? "memory-detail-warning" : "memory-detail-note", hint), status, alert, actions);
      root.hidden = false;
      root.parentElement.dataset.detail = "open";
      mark();
      ({ title, body, back, edit, ask, origin, keep })[want]?.focus();
      if (!want) controls()[held]?.focus();
    }
    // Après une action, la liste ET l'état partagé de l'application sont relus : un fil en cache afficherait sinon un reçu de mémoire périmé (un texte oublié, des commandes actives).
    // L'échec de cette relecture ne devient pas celui de l'action : le panneau ne l'affiche pas comme une erreur.
    // `refresh` partage la lecture déjà en vol : une lecture partie avant l'action rendrait un état périmé. La seconde, lancée une fois la première rendue, part après l'action.
    const refreshState = async () => { for (let pass = 0; pass < 2; pass += 1) await Promise.resolve(panels.refresh?.()).catch(() => {}); };
    const sync = () => Promise.all([reload(), refreshState()]);
    // Au retour d'une action, le focus ne va au panneau que s'il y était (ou nulle part) : quelqu'un qui a cherché ailleurs entre-temps le garde.
    const focusing = (want) => (!document.activeElement || document.activeElement === document.body || root.contains(document.activeElement) ? want : "");
    // Le panneau suit le moteur : il ne montre que ce que la dernière lecture contient.
    async function act(path, request, done, forgetting = false) {
      const mine = epoch;
      busy = true; // une seule action à la fois : tant qu'elle attend, aucun autre souvenir ne s'ouvre
      problem = "";
      render();
      let data;
      const before = new Set((facts() || []).map((item) => item.fact_id)), origin = current();
      try {
        data = await panels.api(path, request);
      } catch (error) {
        // Une erreur ne dit pas si le moteur a appliqué l'action (réponse perdue) : on relit avant de rendre la main, puis le panneau se règle sur la relecture.
        await sync();
        busy = false;
        // Une correction appliquée remplace le souvenir par un nouveau, de même bot, de même portée et de même texte, absent de la liste d'avant. Deux candidats : on ne devine pas.
        const same = request.text === undefined || !origin ? [] : (facts() || []).filter((item) => !before.has(item.fact_id) && item.bot_id === request.bot_id && item.scope === origin.scope && item.text === request.text);
        if (same.length === 1 && !current()) moved.set(request.fact_id, same[0].fact_id);
        refresh();
        if (mine !== epoch) return;
        if (!current()) { close(true); return; } // oublié (ou disparu) pendant l'envoi : le panneau se ferme
        if (current().fact_id !== request.fact_id) { mode = "view"; note = done; draft = ""; problem = ""; render(focusing("edit")); return; } // corrigé : le texte relu s'affiche
        problem = error?.message || "L’action n’a pas abouti.";
        render(focusing(mode === "edit" ? "body" : "back"));
        return;
      }
      if (data?.fact?.fact_id) moved.set(request.fact_id, data.fact.fact_id); // corriger remplace le souvenir par un nouveau
      if (mine === epoch) { mode = "view"; note = done; draft = ""; }
      await sync();
      busy = false;
      refresh(); // la liste relue peut être identique à la précédente : l'écran n'appelle alors pas refresh
      if (mine !== epoch) return;
      if (!current()) { close(true); return; } // oublié, et la relecture le confirme
      // Toujours là après la relecture : elle a échoué, et le panneau le dit au lieu de se taire.
      if (current()?.fact_id === request.fact_id) note = `${done.slice(0, -1)}, mais la liste n’a pas pu être relue : actualisez-la.`;
      render(focusing(forgetting ? "title" : "edit"));
    }
    function close(restore = false) {
      const was = id;
      if (was === null) return;
      if (dirty()) saved.set(was, draft); // quitter l'écran ne se refuse pas : le brouillon attend le retour sur ce souvenir
      [mode, after] = ["view", null];
      [id, shown, root.hidden] = [null, null, true];
      epoch += 1;
      root.replaceChildren();
      mark();
      delete root.parentElement.dataset.detail;
      const back = find(was) || root.parentElement.querySelector('.memory-space-tab[aria-pressed="true"]'), from = opener?.isConnected !== false ? opener : null;
      opener = null;
      if (restore) (from || back?.querySelector("button") || back)?.focus(); // une ligne de la Liste se focalise par son bouton ; le panneau « À revoir » reprend son propre bouton
    }
    // Appelé par l'écran quand la liste a changé : un souvenir disparu ferme le panneau ; la saisie en cours n'est jamais écrasée.
    function refresh() {
      if (id === null) return;
      if (!current() && (facts() || []).some((item) => item.fact_id === moved.get(id))) id = moved.get(id);
      const fact = current();
      if (!fact && busy) return; // l'action en cours décide, une fois sa réponse reçue
      if (!fact) close(!document.activeElement || document.activeElement === document.body || root.contains(document.activeElement));
      else if (stamp(fact) !== shown) render();
      else mark(); // la vue a pu être reconstruite (Graphe ↔ Liste)
    }
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || id === null || event.defaultPrevented || document.querySelector("dialog[open]")) return;
      event.preventDefault();
      if (mode === "discard") { resume(); return; } // Échap répond « Continuer »
      leave(mode === "edit" ? () => render("edit") : () => close(true)); // en édition, Échap ne quitte que l'édition
    });
    // `start` : "edit" ou "forget" ouvre directement Modifier ou la confirmation d'Oublier (le panneau « À revoir »), `from` : le bouton à refocaliser à la fermeture ; un souvenir de sujet ne se modifie pas.
    function open(factId, start = "view", from = null) {
      if (busy || (factId === id && (mode === "edit" || mode === "discard"))) return; // re-cliquer le souvenir en cours de modification ne l'efface pas
      leave(() => {
        if (id !== factId && !(facts() || []).some((item) => item.fact_id === factId)) { render("title"); return; } // le souvenir visé a disparu pendant la confirmation : le panneau reste où il est
        const kept = saved.get(factId) ?? null, fact = (facts() || []).find((item) => item.fact_id === factId);
        const first = kept !== null || (start === "edit" && fact?.scope !== "topic") ? "edit" : start === "forget" ? "forget" : "view";
        epoch += 1;
        saved.delete(factId);
        opener = from;
        [id, mode, draft, note, problem] = [factId, first, kept ?? (first === "edit" ? String(fact?.text ?? "") : ""), kept === null ? "" : "Brouillon retrouvé : Enregistrer ou Annuler.", ""];
        render(first === "edit" ? "body" : first === "forget" ? "back" : "title");
      });
    }
    // Un clic n'importe où sur la carte ouvre le souvenir ; au clavier, le bouton du texte s'en charge.
    const bind = (el, factId) => el.addEventListener("click", () => open(factId));
    return { element: root, open, close, refresh, bind };
  }
  window.EzerbotMemoryDetail = { create };
})();
