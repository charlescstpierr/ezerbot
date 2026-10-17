"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const BATCH_BYTES = 8 * 1024 * 1024;
  const encoder = new TextEncoder();
  const sessions = new Map();
  const mounts = new Set();
  let nextId = 0;
  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const button = (text, action, className = "secondary-button") => {
    const el = node("button", className, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  };
  const reason = (value) => ({ unsupported: "Format non accepté", binary: "Fichier binaire", "invalid-utf8": "UTF-8 invalide", empty: "Fichier vide", "too-large": "Texte trop volumineux", "too-many-skills": "Limite de 250 skills atteinte" }[value] || value);
  function session(botId) {
    if (!sessions.has(botId)) sessions.set(botId, { running: false, mutating: false, rows: [], message: "" });
    return sessions.get(botId);
  }
  function changed() { for (const render of mounts) render(); }
  async function refresh() {
    try { await panels.refresh(); }
    catch { panels.notify("La liste n’a pas pu être actualisée. Rouvrez les réglages après reconnexion."); }
  }
  async function base64(file) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
    return btoa(binary);
  }

  // SR03/SR04, reports/server-orchestration-residual.md: leave conversion,
  // UTF-8/binary checks and naming to the server; preserve serial partial success.
  // Budget SERIALIZED JSON, not raw bytes (skills-wrapper review r1 #6).
  async function importFiles(botId, files) {
    const job = session(botId);
    if (!files.length || job.running || job.mutating) return;
    job.running = true;
    job.rows = files.map((file) => ({ path: file.webkitRelativePath || file.name, status: "pending" }));
    job.message = "Import en cours… Les paquets déjà écrits sont conservés.";
    changed();
    let batch = [], positions = [], interrupted = false;
    const overhead = encoder.encode(JSON.stringify({ bot_id: botId, files: [] })).length;
    let size = overhead;
    async function flush() {
      if (!batch.length) return;
      try {
        const { report } = await panels.api("/api/skills/import", { bot_id: botId, files: batch });
        if (!Array.isArray(report) || report.length !== positions.length) throw new Error("Rapport incomplet du moteur local.");
        report.forEach((item, i) => {
          const status = item.status?.status;
          if (!["written", "skipped", "duplicate"].includes(status)) throw new Error("Statut d’import inconnu.");
          Object.assign(job.rows[positions[i]], { status, reason: item.status.reason, skillId: item.status.skill_id });
        });
      } catch (error) {
        // A lost response is NOT evidence of rollback. Never claim these files
        // were refused or silently retry a possibly committed named update.
        for (const i of positions) Object.assign(job.rows[i], { status: "unknown", reason: error.message });
        interrupted = true;
      }
      batch = []; positions = []; size = overhead;
      changed();
    }
    try {
      for (let i = 0; i < files.length; i++) {
        if (interrupted) break;
        const row = job.rows[i], file = files[i];
        // No file this large can fit a batch, even before base64 expansion.
        if (file.size > BATCH_BYTES) { Object.assign(row, { status: "skipped", reason: "too-large" }); continue; }
        let upload;
        try { upload = { path: row.path, base64: await base64(file) }; }
        catch { Object.assign(row, { status: "skipped", reason: "Lecture du fichier impossible" }); continue; }
        const bytes = encoder.encode(JSON.stringify(upload)).length;
        if (overhead + bytes > BATCH_BYTES) { Object.assign(row, { status: "skipped", reason: "too-large" }); continue; }
        if (batch.length >= 1000 || size + bytes + (batch.length ? 1 : 0) > BATCH_BYTES) await flush();
        if (interrupted) break;
        size += bytes + (batch.length ? 1 : 0);
        batch.push(upload); positions.push(i);
      }
      if (!interrupted) await flush();
      for (const row of job.rows) if (row.status === "pending") row.status = "not_sent";
      job.message = interrupted ? "Import interrompu : certains résultats sont incertains. Vérifiez la liste avant de réimporter; les écritures déjà reçues restent conservées." : "Import terminé. Les succès sont conservés même si d’autres fichiers sont refusés.";
    } finally {
      // Keep new imports/edits locked until the list reflects this batch. An
      // overlapping refresh could otherwise briefly replace newer metadata.
      await refresh();
      job.running = false;
      changed();
    }
  }

  function botSection(bot, container) {
    const section = node("section", "skills-section");
    section.append(node("h3", "", "Skills"));
    container.append(section);
    if (!bot || bot.deleted) {
      section.append(node("p", "skills-note", "Enregistrez d’abord ce bot pour gérer ses skills."));
      return;
    }
    const job = session(bot.id);
    const actions = node("div", "skills-actions");
    const list = node("ul", "skills-list");
    const report = node("details", "skills-report");
    const reportTitle = node("summary", "", "Rapport d’import");
    const progress = node("p", "skills-note");
    progress.setAttribute("role", "status");
    const reportRows = node("ol", "skills-report-rows");
    report.append(reportTitle, reportRows);
    section.append(node("p", "skills-note", "Textes UTF-8 : txt, md, markdown, csv, json, yaml, yml. 256 000 caractères par texte · 250 skills par bot."), actions, list, progress, report);
    const fileInput = node("input", "skills-file-input");
    fileInput.type = "file"; fileInput.multiple = true;
    const directoryInput = fileInput.cloneNode();
    directoryInput.setAttribute("webkitdirectory", "");
    directoryInput.setAttribute("directory", "");
    fileInput.name = "skills-files"; directoryInput.name = "skills-directory";
    fileInput.setAttribute("aria-label", "Fichiers de skills");
    directoryInput.setAttribute("aria-label", "Dossier de skills");
    for (const input of [fileInput, directoryInput]) input.addEventListener("change", () => {
      const files = Array.from(input.files || []);
      input.value = ""; // Re-selecting the same file must still trigger change.
      void importFiles(bot.id, files);
    });
    const add = button("Nouvelle skill", () => void openEditor(null));
    const chooseFiles = button("Importer des fichiers", () => fileInput.click());
    const chooseDirectory = button("Importer un dossier", () => directoryInput.click());
    actions.append(add, chooseFiles, chooseDirectory, fileInput, directoryInput);
    section.append(node("p", "skills-note", "Import par paquets de 8 Mio. Chaque paquet est indépendant : des fichiers de même nom de frontmatter dans des paquets différents peuvent mettre à jour la même skill (historique conservé)."));

    // Dialogs live outside #bot-form: nested forms would submit bot settings.
    const dialog = node("dialog", "dialog skills-dialog");
    const form = node("form");
    const headingId = `skills-editor-${++nextId}`;
    const heading = node("h2"); heading.id = headingId;
    dialog.setAttribute("aria-labelledby", headingId);
    const header = node("div", "dialog-header");
    header.append(heading, button("Fermer", () => dialog.close()));
    const fields = node("fieldset", "skills-fields");
    function field(label, tag, rows) {
      const input = node(tag, "text-input");
      input.id = `${headingId}-${tag}-${fields.children.length}`;
      if (rows) input.rows = rows;
      const caption = node("label", "form-label", label); caption.htmlFor = input.id;
      fields.append(caption, input);
      return input;
    }
    const name = field("Nom", "input"); name.autocomplete = "off";
    const description = field("Description", "textarea", 2);
    const body = field("Contenu", "textarea", 12); body.spellcheck = false;
    const versionLabel = node("p", "skills-note");
    const error = node("p", "skills-error"); error.setAttribute("role", "alert");
    const reload = button("Recharger la version courante (remplace vos saisies)", () => void openEditor(current?.skill_id));
    const history = node("details", "skills-history");
    const select = node("select", "text-input"); select.name = "skill-version";
    select.setAttribute("aria-label", "Version à consulter");
    const preview = node("pre", "skills-version-preview");
    history.append(node("summary", "", "Historique des versions (lecture seule)"), select, preview);
    const confirmation = node("div", "skills-confirmation"); confirmation.hidden = true;
    confirmation.append(node("p", "skills-note", "Supprimer cette skill ? Elle ne sera plus proposée au bot. Les versions déjà épinglées restent conservées."), button("Confirmer la suppression", () => void remove(), "danger-button"), button("Garder la skill", () => { confirmation.hidden = true; }));
    const footer = node("div", "dialog-footer");
    const removeButton = button("Supprimer", () => { confirmation.hidden = false; }, "danger-link");
    const save = node("button", "primary-button", "Enregistrer"); save.type = "submit";
    footer.append(removeButton, button("Annuler", () => dialog.close()), save);
    fields.append(versionLabel, history, confirmation, footer);
    form.append(header, error, reload, fields);
    dialog.append(form);
    (document.getElementById("panels") || document.body).append(dialog);
    let alive = true, token = 0, current = null, versions = [], baseline = {}, signature = "", reportSignature = "";
    const active = (request) => alive && request === token && dialog.open;
    dialog.addEventListener("close", () => { if (!dialog.open) token++; });
    function showVersion() {
      const version = versions.find((item) => String(item.version) === select.value);
      preview.textContent = version ? `${version.deleted ? "Supprimée · " : ""}${version.content.name}\n${version.content.description}\n\n${version.content.body}` : "";
    }
    select.addEventListener("change", showVersion);
    function populate(skill, allVersions) {
      current = skill; versions = allVersions;
      name.value = skill?.content.name || "";
      description.value = skill?.content.description || "";
      body.value = skill?.content.body || "";
      // Textareas normalize CRLF. Compare with the displayed initial values so
      // editing a description never silently rewrites an untouched imported body.
      baseline = { name: name.value, description: description.value, body: body.value };
      versionLabel.textContent = skill ? `Version ${skill.version} · ${skill.skill_id}` : "Le nom sera normalisé; les collisions reçoivent un suffixe.";
      history.hidden = !skill; history.open = false;
      removeButton.hidden = !skill; confirmation.hidden = true;
      reload.hidden = true;
      select.replaceChildren(...versions.map((item) => {
        const option = node("option", "", `Version ${item.version}${item.deleted ? " · supprimée" : ""}`);
        option.value = String(item.version); return option;
      }));
      showVersion();
    }
    async function openEditor(skillId) {
      if (job.running || job.mutating) return;
      const request = ++token;
      heading.textContent = skillId ? "Modifier une skill" : "Nouvelle skill";
      populate(null, []); error.textContent = ""; reload.disabled = false;
      if (!dialog.open) dialog.showModal();
      fields.disabled = Boolean(skillId);
      if (skillId) {
        error.textContent = "Chargement…";
        try {
          const response = await panels.api(`/api/skills/versions?bot_id=${encodeURIComponent(bot.id)}&skill_id=${encodeURIComponent(skillId)}`);
          if (!active(request)) return;
          const all = response.versions.slice().sort((a, b) => b.version - a.version);
          if (!all.length || all[0].deleted) throw new Error("Cette skill a été supprimée. Rouvrez la liste.");
          populate(all[0], all); error.textContent = ""; fields.disabled = false; name.focus();
        } catch (failure) { if (active(request)) error.textContent = failure.message; }
      } else { fields.disabled = false; name.focus(); }
    }
    async function mutate(path, payload, success) {
      if (job.running || job.mutating) return;
      const request = token;
      job.mutating = true; fields.disabled = true; reload.disabled = true; error.textContent = ""; changed();
      try {
        await panels.api(path, payload);
        if (active(request)) dialog.close();
        panels.notify(success);
        await refresh();
      } catch (failure) {
        if (active(request)) {
          error.textContent = `Action non confirmée : ${failure.message} Vos saisies sont conservées. Vérifiez la liste avant de réessayer. En cas de conflit, rechargez explicitement la version courante.`;
          reload.hidden = !current;
        } else panels.notify(failure.message);
        await refresh();
      } finally {
        job.mutating = false;
        if (active(request)) { fields.disabled = false; reload.disabled = false; }
        changed();
      }
    }
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      if (fields.disabled) return;
      const values = { name: name.value, description: description.value, body: body.value };
      if (!values.name.trim()) { error.textContent = "Donnez un nom à cette skill."; name.focus(); return; }
      // Imported annotations can make an unchanged body longer than the limit.
      if ((!current || values.body !== baseline.body) && values.body.length > 256000) {
        error.textContent = "Le contenu dépasse 256 000 caractères."; return;
      }
      const changes = current ? Object.fromEntries(Object.entries(values).filter(([key, value]) => value !== baseline[key])) : values;
      if (current && !Object.keys(changes).length) { dialog.close(); return; }
      void mutate(current ? "/api/skills/update" : "/api/skills", { bot_id: bot.id, ...(current ? { skill_id: current.skill_id, expected_version: current.version } : {}), ...changes }, "Skill enregistrée.");
    });
    async function remove() {
      if (!current || fields.disabled) return;
      await mutate("/api/skills/delete", { bot_id: bot.id, skill_id: current.skill_id, expected_version: current.version }, "Skill supprimée.");
    }
    function render() {
      if (!alive) return;
      const locked = job.running || job.mutating;
      for (const control of [add, chooseFiles, chooseDirectory, fileInput, directoryInput]) control.disabled = locked;
      const skills = panels.state.skills?.by_bot?.[bot.id] || [];
      const next = JSON.stringify([skills, locked, panels.state.skills?.error]);
      if (signature !== next) {
        signature = next;
        list.replaceChildren();
        section.querySelector("h3").textContent = `Skills · ${skills.length}/250`;
        if (!skills.length) list.append(node("li", "skills-note", panels.state.skills?.error || "Aucune skill pour ce bot."));
        for (const skill of skills) {
          const row = node("li", "skills-row"), copy = node("div");
          copy.append(node("strong", "", skill.name + (skill.name_truncated ? "…" : "")), node("small", "", (skill.description || "Sans description") + (skill.description_truncated ? "…" : "")), node("small", "", `Version ${skill.version} · ${skill.skill_id}`));
          const edit = button("Modifier / historique", () => void openEditor(skill.skill_id));
          edit.setAttribute("aria-label", `Modifier / historique : ${skill.name}`); edit.disabled = locked;
          row.append(copy, edit); list.append(row);
        }
      }
      const countLabel = (status, singular, plural = `${singular}s`) => {
        const count = job.rows.filter((row) => row.status === status).length;
        return `${count} ${count === 1 ? singular : plural}`;
      };
      progress.textContent = job.message;
      report.hidden = !job.rows.length;
      reportTitle.textContent = `Rapport : ${countLabel("written", "écrite")} · ${countLabel("duplicate", "doublon")} · ${countLabel("skipped", "refusée")} · ${countLabel("unknown", "incertaine")} · ${countLabel("not_sent", "non envoyée")} · ${countLabel("pending", "en attente", "en attente")}`;
      const nextReport = JSON.stringify(job.rows);
      if (nextReport !== reportSignature) {
        reportSignature = nextReport;
        const labels = { written: "Écrite", duplicate: "Déjà importée", skipped: "Refusée", unknown: "Résultat incertain", not_sent: "Non envoyée", pending: "En attente" };
        reportRows.replaceChildren(...job.rows.map((item) => {
          const row = node("li");
          row.dataset.status = item.status;
          row.append(node("span", "skills-report-path", item.path), node("span", "", `${labels[item.status]}${item.reason ? ` — ${reason(item.reason)}` : ""}`));
          return row;
        }));
        if (job.rows.length) report.open = true;
      }
    }
    mounts.add(render); render();
    return () => { alive = false; token++; mounts.delete(render); dialog.close(); dialog.remove(); };
  }
  panels.register({ id: "skills", botSection, onState: changed });
})();
