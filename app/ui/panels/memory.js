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
  const forgotten = new Map(), forgottenIds = new Set();
  let activeReload, settingsSection, automaticRevision;
  const settingInputs = new Map();
  function renderAutomaticSettings(state) {
    if (!settingsSection) return;
    const availability = settingsSection.querySelector(".memory-auto-availability");
    if (state?.memory_auto?.available === false) {
      availability.hidden = false;
      availability.textContent = !state?.provider?.connected ? "Connectez ChatGPT pour utiliser l’extraction automatique."
        : !state?.provider?.models_ready ? "Vérification des petits modèles disponibles…"
          : "Indisponible : aucun petit modèle dans votre offre.";
    } else { availability.hidden = true; availability.textContent = ""; }
    const bots = (state?.bots || []).filter((bot) => !bot.deleted);
    const ids = new Set(bots.map((bot) => bot.id));
    for (const [id, input] of settingInputs) if (!ids.has(id)) { input.closest("label").remove(); settingInputs.delete(id); }
    for (const bot of bots) {
      let input = settingInputs.get(bot.id);
      if (!input) {
        const label = node("label", "setting-toggle memory-auto-toggle");
        const copy = node("span");
        copy.append(node("strong", "", bot.name));
        input = node("input"); input.type = "checkbox"; input.name = `automatic-${bot.id}`; input.setAttribute("role", "switch");
        const track = node("span", "toggle-track"); track.setAttribute("aria-hidden", "true");
        label.append(copy, input, track); settingsSection.append(label); settingInputs.set(bot.id, input);
        input.addEventListener("change", async () => {
          const enabled = input.checked;
          input.disabled = true;
          try { await panels.api("/api/memory/automatic", { bot_id: bot.id, enabled }); await panels.refresh(); }
          catch (error) { input.checked = !enabled; panels.notify(error.message); }
          finally { input.disabled = false; }
        });
      }
      const paused = state?.memory_auto?.available === false;
      const label = input.closest("label");
      label.className = `setting-toggle memory-auto-toggle${paused ? " is-paused" : ""}`;
      label.querySelector("strong").textContent = `${bot.name}${paused ? " (en pause)" : ""}`;
      input.setAttribute("aria-label", `Mémoire automatique — ${bot.name}${paused ? " — en pause" : ""}`);
      if (!input.disabled) input.checked = state?.memory_auto?.settings?.find((s) => s.bot_id === bot.id)?.enabled !== false;
    }
  }
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
    settingsSection(container) {
      settingsSection = node("section", "setting-section");
      settingsSection.append(node("h3", "", "Mémoire automatique"), node("p", "", "Activée par défaut pour retenir vos préférences, votre identité et vos projets après une réponse. Au plus 3 faits par tour, privés au départ. Les secrets et les réponses aux formulaires sont exclus. Vous pouvez annuler ou modifier chaque fait dans le fil."));
      const availability = node("p", "memory-auto-availability");
      availability.setAttribute("role", "status");
      settingsSection.append(availability);
      container.append(settingsSection);
      renderAutomaticSettings(panels.state);
    },
    onState(state) {
      renderAutomaticSettings(state);
      const revision = state?.memory_auto?.revision;
      if (revision !== automaticRevision) {
        automaticRevision = revision;
        activeReload?.();
        // The worker can commit between /api/state's conversation projection
        // and its revision/pending reads. Re-read once for each new revision so
        // a fast completion cannot stop polling with its last receipt missing.
        if (revision > 0) Promise.resolve().then(() => panels.refresh()).catch(() => {});
      }
    },
    wantsPolling() { return panels.state?.memory_auto?.pending === true; },
    async onCardAction(message, action, detail) {
      if (message.kind === "memory_share" && action === "share") {
        const offered = new Set((message.payload?.facts || []).map((f) => f.fact_id));
        if (!Array.isArray(detail?.fact_ids) || detail.fact_ids.some((id) => !offered.has(id))) throw new Error("Sélection invalide.");
        try {
          const report = await panels.api("/api/memory/promote", { bot_id: message.payload.bot_id, fact_ids: detail.fact_ids });
          detail.onConfirmed?.(report);
          return true;
        }
        finally { await panels.refresh().catch(() => {}); activeReload?.(); }
      }
      if (message.kind !== "memory_auto") return false;
      if (!["undo", "correct"].includes(action)) return false;
      try {
        if (action === "undo") await panels.api("/api/memory/automatic/undo", { card_id: message.id });
        else await panels.api("/api/memory/correct", { bot_id: message.payload.bot_id, fact_id: message.payload.fact_id, text: detail.text });
        return true;
      } finally {
        // Even a stale action must reconcile the card with the durable fact.
        await panels.refresh().catch(() => {});
        activeReload?.();
      }
    },
    botSection(bot, container) {
      const section = node("section", "memory-panel");
      // Enter on a checkbox otherwise implicitly submits the enclosing bot form.
      section.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && event.target instanceof HTMLInputElement) event.preventDefault();
      });
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
      let editingId, reloadTimer, sharingClasses = {};
      const search = node("input", "text-input");
      search.type = "search"; search.placeholder = "Rechercher un souvenir…"; search.setAttribute("aria-label", "Rechercher dans la mémoire");
      const filters = node("div", "memory-filters");
      const scopeFilter = node("select", "text-input"); scopeFilter.setAttribute("aria-label", "Filtrer la portée");
      for (const [value, text] of [["", "Toutes les portées"], ["private", "Privé"], ["team", "Équipe"]]) { const opt = node("option", "", text); opt.value = value; scopeFilter.append(opt); }
      const sourceFilter = node("select", "text-input"); sourceFilter.setAttribute("aria-label", "Filtrer la source");
      for (const [value, text] of [["", "Toutes les sources"], ["automatic", "Automatique"], ["user_stated", "Déclaré"], ["model_inferred", "Déduit"], ["imported", "Importé"]]) { const opt = node("option", "", text); opt.value = value; sourceFilter.append(opt); }
      filters.append(search, scopeFilter, sourceFilter);
      const normalize = (text) => text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase("fr");
      function visible() { return facts.filter((f) => (!scopeFilter.value || f.scope === scopeFilter.value) && (!sourceFilter.value || f.provenance?.source === sourceFilter.value) && normalize(f.text).includes(normalize(search.value.trim()))); }
      search.addEventListener("input", render); scopeFilter.addEventListener("change", render); sourceFilter.addEventListener("change", render);
      const saturation = node("p", "memory-saturation"); saturation.hidden = true; saturation.setAttribute("role", "status");
      const profile = node("details", "memory-profile");
      const profileTitle = node("summary", "", "Profil consolidé");
      const profileBody = node("div"); profile.append(profileTitle, profileBody);
      const controls = node("fieldset", "memory-controls");
      const toolbar = node("div", "memory-toolbar");
      const allLabel = node("label", "memory-select-all");
      const all = node("input");
      all.type = "checkbox";
      allLabel.append(all, document.createTextNode("Sélectionner les faits de travail visibles"));
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
      controls.append(filters, toolbar, list, editor);
      section.append(node("p", "memory-note", "Privé : ce bot seulement. Équipe : tous les bots de cet espace local. Tri local indicatif : les faits personnels et à vérifier sont exclus de la sélection par défaut. Les faits sensibles restent privés."), saturation, profile, status, result, controls, history);

      // Already-shared facts must not replay a cached receipt as a new sharing.
      function available() { return facts.filter((fact) => fact.scope !== "team" && !fact.sensitive && !processed.has(fact.fact_id)); }
      function defaultSelection() { return visible().filter((f) => available().includes(f) && sharingClasses[f.fact_id] === "work"); }
      function visibleSelected() {
        const shown = new Set(visible().map((fact) => fact.fact_id));
        return [...selected].filter((id) => shown.has(id));
      }
      function updateSelection() {
        const eligible = defaultSelection();
        const count = visibleSelected().length;
        all.disabled = !loaded || !eligible.length || bot.deleted;
        all.checked = eligible.length > 0 && eligible.every((fact) => selected.has(fact.fact_id));
        all.indeterminate = selected.size > 0 && !all.checked;
        promote.disabled = !loaded || !count || bot.deleted;
        promote.textContent = `Partager la sélection (${count})`;
      }
      all.addEventListener("change", () => {
        for (const fact of defaultSelection()) {
          if (all.checked) selected.add(fact.fact_id); else selected.delete(fact.fact_id);
        }
        render();
      });
      function showTotals() {
        const shared = totals.created + totals.already;
        result.textContent = `Bilan de cette ouverture : ${shared} ${shared === 1 ? "partagé" : "partagés"} (${totals.created} ${totals.created === 1 ? "nouveau" : "nouveaux"}, ${totals.already} déjà en équipe) · ${totals.kept} ${totals.kept === 1 ? "gardé privé" : "gardés privés"}.`;
      }
      function render() {
        if (!alive) return;
        facts = facts.filter((fact) => !forgottenIds.has(fact.fact_id));
        const ids = new Set(available().map((fact) => fact.fact_id));
        for (const id of selected) if (!ids.has(id)) selected.delete(id);
        list.replaceChildren();
        const shown = visible();
        if (loaded) list.append(node("p", "memory-note", `${shown.length} sur ${facts.length} faits actifs${shown.length ? "" : " · Aucun résultat"}.`));
        for (const fact of shown) {
          const row = node("article", "memory-fact");
          row.dataset.factId = fact.fact_id;
          const label = node("label", "memory-fact-choice");
          const check = node("input");
          check.type = "checkbox";
          check.checked = selected.has(fact.fact_id) || processed.has(fact.fact_id);
          check.disabled = processed.has(fact.fact_id) || fact.scope === "team" || fact.sensitive || bot.deleted;
          check.addEventListener("change", () => {
            if (check.checked) selected.add(fact.fact_id); else selected.delete(fact.fact_id);
            updateSelection();
          });
          label.append(check, node("span", "memory-text", `« ${fact.text} »`));
          const owner = panels.state?.bots?.find((item) => item.id === fact.bot_id);
          const scope = fact.scope === "team" ? "Équipe" : "Privé";
          const date = new Date(fact.learned_at);
          row.append(label, node("p", "memory-meta", `${scope} · ${{work: "Travail", personal: "Personnel — non présélectionné", review: "À vérifier — non présélectionné"}[sharingClasses[fact.fact_id]] || "À vérifier"}${fact.provenance?.source === "automatic" ? " · Automatique" : ""}${fact.scope === "team" ? " · Déjà en équipe" : ""}${fact.sensitive ? " · Sensible" : ""} · ${owner?.name || "Bot indisponible"}${owner?.deleted ? " (supprimé)" : ""}${Number.isNaN(date.getTime()) ? "" : ` · ${date.toLocaleString("fr")}`}${processed.has(fact.fact_id) ? " · Sélection traitée" : ""}`));
          const actions = node("div", "memory-actions");
          // Models may mutate only their own facts. The user can purge a visible
          // team fact through its owner, including a deleted owner (wrapper r2).
          if (fact.bot_id === bot.id && !bot.deleted) actions.append(button("Modifier", () => edit(fact)));
          actions.append(button("Oublier…", () => confirmForget(fact)));
          row.append(actions);
          list.append(row);
        }
        history.replaceChildren();
        const tombstones = [...forgotten.values()].filter((fact) => fact.bot_id === bot.id || fact.scope === "team");
        if (tombstones.length) {
          history.append(node("h4", "", "Oubliés dans cette fenêtre"), node("p", "memory-note", `${tombstones.length} ${tombstones.length === 1 ? "fait oublié" : "faits oubliés"} (avec ${tombstones.length === 1 ? "ses" : "leurs"} versions précédentes). Le texte et les versions précédentes ne sont pas conservés ici. Ce bilan local disparaît au rechargement.`));
        }
        controls.disabled = busy;
        section.setAttribute("aria-busy", String(busy));
        updateSelection();
        if (editingId && !facts.some((fact) => fact.fact_id === editingId)) closeEditor();
      }
      async function readFacts() {
        const data = await panels.api("/api/memory", { bot_id: bot.id });
        if (!Array.isArray(data.facts)) throw new Error("La liste mémoire est illisible.");
        sharingClasses = data.sharing_classes || {};
        saturation.hidden = !data.automatic?.saturated;
        saturation.textContent = data.automatic?.saturated ? `Apprentissage automatique en pause : le catalogue dépasse 6 000 caractères (${data.automatic.catalogue_chars.toLocaleString("fr")}). Vos souvenirs restent disponibles via le rappel ciblé. Corrigez ou oubliez les faits devenus inutiles pour reprendre ; aucun souvenir n’est supprimé automatiquement.` : "";
        const p = data.profile;
        profileTitle.textContent = `Profil consolidé · v${p?.version || 0} · ${data.facts.length} faits`;
        profileBody.replaceChildren(node("p", "memory-note", "Résumé local dérivé des faits actifs, mis à jour à chaque modification. Les versions gardent un journal sans copie du texte oublié."));
        for (const part of p?.sections || []) {
          profileBody.append(node("h4", "", `${part.title} (${part.count})`));
          for (const f of part.facts) profileBody.append(node("p", "memory-note", f.text));
        }
        const log = node("details"); log.append(node("summary", "", "Journal des versions"));
        const actions = { baseline: "Profil initial", learned: "Fait ajouté", forgotten: "Fait oublié", corrected: "Fait remplacé", shared: "Fait partagé", updated: "Mise à jour" };
        for (const item of p?.log || []) {
          const version = node("details");
          version.append(node("summary", "", `v${item.version} · ${actions[item.action] || item.action} · ${new Date(item.changed_at).toLocaleString("fr")}`));
          for (const f of item.summary || []) version.append(node("p", "memory-note", f.text));
          if (!item.summary?.length) version.append(node("p", "memory-note", "Aucun fait de cette version n’est encore actif."));
          log.append(version);
        }
        profileBody.append(log);
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
      function closeEditor() { editingId = undefined; editor.replaceChildren(); editor.hidden = true; }
      function openEditor(id) {
        closeEditor();
        editingId = id;
        editor.hidden = false;
        return button("Annuler", closeEditor);
      }
      function edit(fact) {
        const cancel = openEditor(fact.fact_id);
        const label = node("label", "memory-edit-label", "Modifier ce fait");
        const input = node("textarea", "text-input");
        input.rows = 3;
        input.value = fact.text;
        label.append(input);
        const hint = node("p", "memory-note", "1 à 2 000 caractères. Une correction remplace le fait, sans changer sa portée ni sa sensibilité.");
        const save = button("Enregistrer la modification", () => {
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
            status.textContent = "Fait modifié.";
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
        const cancel = openEditor(fact.fact_id);
        const characters = [...fact.text];
        const excerpt = characters.slice(0, 160).join("").trimEnd() + (characters.length > 160 ? "…" : "");
        editor.append(node("p", "memory-note", `Oublier « ${excerpt} » et ses versions précédentes ?${fact.scope === "team" ? " Ce fait sera retiré pour tous les bots de l’espace." : ""} Cette action ne réinitialise pas la conversation.`), button("Confirmer l’oubli", () => {
          mutate("/api/memory/forget", { bot_id: fact.bot_id, fact_id: fact.fact_id }, (data) => {
            for (const id of data.forgotten) forgottenIds.add(id);
            forgotten.set(fact.fact_id, { bot_id: fact.bot_id, scope: fact.scope });
            facts = facts.filter((item) => !forgottenIds.has(item.fact_id));
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
          // The mutation was confirmed. A failed state read must not report it
          // as failed or replace the server's original error on another action.
          await panels.refresh().catch(() => {});
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
        const ids = visibleSelected();
        if (!alive || busy || !ids.length) return;
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
            await panels.refresh().catch(() => {});
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
      function requestReload() {
        if (!alive) return;
        clearTimeout(reloadTimer);
        if (busy) { reloadTimer = setTimeout(requestReload, 200); return; }
        load();
      }
      activeReload = requestReload;
      load();
      return () => {
        alive = false;
        clearTimeout(reloadTimer);
        revision++;
        if (activeReload === requestReload) activeReload = undefined;
        facts = [];
        selected.clear();
        closeEditor();
        section.replaceChildren();
      };
    },
  });
})();
