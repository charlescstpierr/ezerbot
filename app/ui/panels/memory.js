/* REA: goal-context-memory-learning.md CM09–CM12, server-context-residual.md
 * RC02–RC03 and memory-widget-consumer.md MW03–MW06: select fact IDs, submit
 * sequential batches of 100, retain successful batches, count shared separately
 * from kept-private. Correction/forgetting UI is a local design, not REA evidence.
 */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  // /api/memory lists ACTIVE facts only. Keep confirmed tombstones for this
  // window, including across section remounts, never their text or provenance.
  // A durable forgotten-history endpoint belongs to a separate server slice.
  const forgotten = new Map();
  let activeReload;
  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const button = (text, run, className = "secondary-button") => {
    const el = node("button", className, text);
    el.type = "button"; // The section lives inside #bot-form.
    el.addEventListener("click", run);
    return el;
  };

  panels.register({
    id: "memory",
    botSection(bot, container) {
      const section = node("section", "memory-panel");
      section.setAttribute("aria-label", "Mémoire du bot");
      section.append(node("h3", "", "Mémoire"));
      container.append(section);
      if (!bot?.id) {
        section.append(node("p", "memory-note", "Créez le bot pour consulter sa mémoire."));
        return;
      }
      const selected = new Set();
      const processed = new Set();
      const totals = { created: 0, already: 0, kept: 0 };
      let facts = [], alive = true, busy = false, loaded = false, revision = 0;
      const controls = node("fieldset", "memory-controls");
      const toolbar = node("div", "memory-toolbar");
      const allLabel = node("label", "memory-select-all");
      const all = node("input");
      all.type = "checkbox";
      allLabel.append(all, document.createTextNode("Tout sélectionner"));
      const promote = button("Partager la sélection", share, "primary-button");
      const reload = button("Actualiser", () => load());
      toolbar.append(allLabel, promote, reload);
      const status = node("p", "memory-note");
      status.setAttribute("role", "status");
      const result = node("p", "memory-result");
      result.setAttribute("role", "status");
      const list = node("div", "memory-list");
      const history = node("div", "memory-history");
      const editor = node("div", "memory-editor");
      editor.hidden = true;
      controls.append(toolbar, list, editor);
      section.append(node("p", "memory-note", "Privé : ce bot seulement. Équipe : tous les bots de cet espace local. Les faits sensibles restent privés. Oublier n’est pas réinitialiser une conversation."), status, result, controls, history);

      function available() { return facts.filter((fact) => !processed.has(fact.fact_id)); }
      function updateSelection() {
        const eligible = available();
        all.disabled = !loaded || !eligible.length || bot.deleted;
        all.checked = eligible.length > 0 && eligible.every((fact) => selected.has(fact.fact_id));
        all.indeterminate = selected.size > 0 && !all.checked;
        promote.disabled = !loaded || !selected.size || bot.deleted;
        promote.textContent = `Partager la sélection (${selected.size})`;
      }
      all.addEventListener("change", () => {
        for (const fact of available()) {
          if (all.checked) selected.add(fact.fact_id); else selected.delete(fact.fact_id);
        }
        render();
      });
      function showTotals() {
        result.textContent = `Bilan de cette ouverture : ${totals.created + totals.already} partagés (${totals.created} nouveaux, ${totals.already} déjà en équipe) · ${totals.kept} gardés privés.`;
      }
      function render() {
        if (!alive) return;
        facts = facts.filter((fact) => !forgotten.has(fact.fact_id));
        const ids = new Set(available().map((fact) => fact.fact_id));
        for (const id of selected) if (!ids.has(id)) selected.delete(id);
        list.replaceChildren();
        if (loaded && !facts.length) list.append(node("p", "memory-note", "Aucun fait actif."));
        for (const fact of facts) {
          const row = node("article", "memory-fact");
          row.dataset.factId = fact.fact_id;
          const label = node("label", "memory-fact-choice");
          const check = node("input");
          check.type = "checkbox";
          check.checked = selected.has(fact.fact_id) || processed.has(fact.fact_id);
          check.disabled = processed.has(fact.fact_id) || bot.deleted;
          check.setAttribute("aria-label", `Sélectionner ${fact.fact_id}`);
          check.addEventListener("change", () => {
            if (check.checked) selected.add(fact.fact_id); else selected.delete(fact.fact_id);
            updateSelection();
          });
          label.append(check, node("span", "memory-text", fact.text));
          const owner = panels.state?.bots?.find((item) => item.id === fact.bot_id);
          const scope = fact.scope === "team" ? "Équipe" : "Privé";
          const date = new Date(fact.learned_at);
          row.append(label, node("code", "memory-id", fact.fact_id), node("p", "memory-meta", `${scope}${fact.sensitive ? " · Sensible" : ""} · ${owner?.name || fact.bot_id}${owner?.deleted ? " (supprimé)" : ""}${Number.isNaN(date.getTime()) ? "" : ` · ${date.toLocaleString("fr")}`}${processed.has(fact.fact_id) ? " · Sélection traitée" : ""}`));
          const actions = node("div", "memory-actions");
          // Models may mutate only their own facts. The user can purge a visible
          // team fact through its owner, including a deleted owner (wrapper r2).
          if (fact.bot_id === bot.id && !bot.deleted) actions.append(button("Corriger", () => edit(fact)));
          actions.append(button("Oublier…", () => confirmForget(fact)));
          row.append(actions);
          list.append(row);
        }
        history.replaceChildren();
        const tombstones = [...forgotten.values()].filter((fact) => fact.bot_id === bot.id || fact.scope === "team");
        if (tombstones.length) {
          history.append(node("h4", "", "Oubliés dans cette fenêtre"), node("p", "memory-note", "Identifiants uniquement ; le texte n’est pas conservé ici. Cet historique local disparaît au rechargement."));
          for (const fact of tombstones) {
            const row = node("p", "memory-tombstone", "Oublié · ");
            row.append(node("code", "memory-id", fact.fact_id));
            history.append(row);
          }
        }
        controls.disabled = busy;
        section.setAttribute("aria-busy", String(busy));
        updateSelection();
      }
      async function readFacts() {
        const data = await panels.api("/api/memory", { bot_id: bot.id });
        if (!Array.isArray(data.facts)) throw new Error("La liste mémoire est illisible.");
        return data.facts;
      }
      async function load() {
        if (!alive || busy) return;
        const request = ++revision;
        busy = true;
        status.textContent = "Chargement de la mémoire…";
        render();
        try {
          const items = await readFacts();
          if (!alive || request !== revision) return;
          facts = items;
          loaded = true;
          status.textContent = "";
        } catch (error) {
          if (alive && request === revision) status.textContent = `Lecture impossible : ${error.message} Cliquez sur Actualiser pour réessayer.`;
        } finally {
          if (alive && request === revision) { busy = false; render(); }
        }
      }
      function closeEditor() { editor.replaceChildren(); editor.hidden = true; }
      function openEditor() {
        closeEditor();
        editor.hidden = false;
        return button("Annuler", closeEditor);
      }
      function edit(fact) {
        const cancel = openEditor();
        const label = node("label", "memory-edit-label", `Corriger ${fact.fact_id}`);
        const input = node("textarea", "text-input");
        input.rows = 3;
        input.value = fact.text;
        label.append(input);
        const hint = node("p", "memory-note", "1 à 2 000 caractères. Une correction crée un nouvel identifiant, sans changer la portée ni la sensibilité.");
        const save = button("Enregistrer la correction", () => {
          const text = input.value.trim();
          if (!text || [...text].length > 2000) { hint.textContent = "Saisissez entre 1 et 2 000 caractères."; input.focus(); return; }
          if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(text)) {
            hint.textContent = "Retirez les caractères de contrôle du texte.";
            input.focus();
            return;
          }
          mutate("/api/memory/correct", { bot_id: bot.id, fact_id: fact.fact_id, text }, (data) => {
            facts = facts.filter((item) => item.fact_id !== fact.fact_id);
            facts.unshift(data.fact);
            selected.delete(fact.fact_id);
            status.textContent = `Fait corrigé : ${data.fact.fact_id}.`;
          });
        }, "primary-button");
        // Never submit the enclosing bot-profile form with Cmd/Ctrl+Enter.
        input.addEventListener("keydown", (event) => {
          if (event.key === "Enter") {
            event.stopPropagation();
            if (event.metaKey || event.ctrlKey) { event.preventDefault(); if (!busy) save.click(); }
          }
        });
        editor.append(label, hint, save, cancel);
        input.focus();
      }
      function confirmForget(fact) {
        const cancel = openEditor();
        editor.append(node("p", "memory-note", `Oublier ${fact.fact_id} et ses versions précédentes ?${fact.scope === "team" ? " Ce fait sera retiré pour tous les bots de l’espace." : ""} Cette action ne réinitialise pas la conversation.`), button("Confirmer l’oubli", () => {
          mutate("/api/memory/forget", { bot_id: fact.bot_id, fact_id: fact.fact_id }, (data) => {
            for (const id of data.forgotten) forgotten.set(id, { fact_id: id, bot_id: fact.bot_id, scope: fact.scope });
            facts = facts.filter((item) => !forgotten.has(item.fact_id));
            status.textContent = "Fait oublié.";
          });
        }, "danger-button"), cancel);
      }
      async function mutate(path, body, apply) {
        if (!alive || busy) return;
        busy = true;
        status.textContent = "Enregistrement…";
        render();
        try {
          const data = await panels.api(path, body);
          apply(data); // Record confirmed forgetting even if the dialog closed.
          closeEditor(); // Drop the corrected/forgotten text from the DOM.
          if (alive) {
            render();
            try { facts = await readFacts(); }
            catch (error) { status.textContent += ` Liste à actualiser : ${error.message}`; }
          } else activeReload?.();
        } catch (error) {
          if (alive) {
            status.textContent = `Action non confirmée : ${error.message} Actualisez la liste avant de réessayer.`;
            try { facts = await readFacts(); } catch { /* Keep the actionable error. */ }
          }
        } finally {
          if (alive) { busy = false; render(); }
        }
      }
      async function share() {
        if (!alive || busy || !selected.size) return;
        const ids = [...selected];
        busy = true;
        closeEditor();
        status.textContent = "Partage en cours…";
        render();
        try {
          for (let offset = 0; offset < ids.length; offset += 100) {
            if (!alive) break; // Do not start another batch after closing.
            const batch = ids.slice(offset, offset + 100);
            const data = await panels.api("/api/memory/promote", { bot_id: bot.id, fact_ids: batch });
            totals.created += data.created.length;
            totals.already += data.already_in_team.length;
            totals.kept += data.kept_private_count;
            for (const id of batch) { processed.add(id); selected.delete(id); }
            showTotals(); // Kept-private IDs are processed too (MW03).
          }
          if (alive) status.textContent = "Partage terminé.";
        } catch (error) {
          if (alive) status.textContent = `Partage interrompu : ${error.message} Les lots confirmés sont conservés ; réessayez la sélection restante.`;
        } finally {
          // Re-read on success AND failure, without losing confirmed counts.
          if (alive) {
            try { facts = await readFacts(); }
            catch (error) { status.textContent += ` Liste à actualiser : ${error.message}`; }
            busy = false;
            render();
          } else activeReload?.();
        }
      }
      activeReload = load;
      load();
      return () => {
        alive = false;
        revision++;
        if (activeReload === load) activeReload = undefined;
        facts = [];
        selected.clear();
        closeEditor();
        section.replaceChildren();
      };
    },
  });
})();
