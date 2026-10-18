"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const BATCH_BYTES = 8 * 1024 * 1024;
  const EXTENSIONS = ["txt", "md", "markdown", "csv", "json", "yaml", "yml"];
  const displayName = (skill) => skill.display_name || skill.name;
  function supportedText(path) {
    const base = path.split(/[\\/]/).pop();
    const dot = base.lastIndexOf(".");
    return dot > 0 && EXTENSIONS.includes(base.slice(dot + 1).toLowerCase());
  }
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
  const reason = (value) => ({ unsupported: "Format non accepté", empty: "Fichier vide", "too-large": "Texte trop volumineux", "too-many-skills": "Limite de 250 skills atteinte", "name-too-large": "Le nom ne doit pas dépasser 128 caractères.", "description-too-large": "La description ne doit pas dépasser 1 024 caractères.", "skill name must contain a letter or digit": "Le nom doit contenir une lettre latine (accents acceptés) ou un chiffre." }[value] || value);
  function session(botId) {
    if (!sessions.has(botId)) sessions.set(botId, { running: false, mutating: false, generation: 0, rows: [], message: "" });
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

  // SR03/SR04, reports/server-orchestration-residual.md: reject unsupported
  // extensions before reading. Conversion, UTF-8/binary checks and naming remain
  // authoritative on the server; preserve serial partial success.
  // Budget SERIALIZED JSON, not raw bytes (skills-wrapper review r1 #6).
  async function importFiles(botId, files) {
    const job = session(botId);
    if (!files.length || job.running || job.mutating) return;
    job.running = true;
    job.generation++;
    job.rows = files.map((file) => ({ path: file.webkitRelativePath || file.name, status: "pending" }));
    job.message = "Import en cours… Les paquets déjà écrits sont conservés.";
    changed();
    let batch = [], positions = [], interrupted = false, batchNumber = 0;
    const written = new Map();
    const overhead = encoder.encode(JSON.stringify({ bot_id: botId, files: [] })).length;
    let size = overhead;
    async function flush() {
      if (!batch.length) return;
      batchNumber++;
      try {
        const { report } = await panels.api("/api/skills/import", { bot_id: botId, files: batch });
        if (!Array.isArray(report) || report.length !== positions.length) throw new Error("Rapport incomplet du moteur local.");
        report.forEach((item, i) => {
          const status = item.status?.status;
          if (!["written", "skipped", "duplicate"].includes(status)) throw new Error("Statut d’import inconnu.");
          const row = job.rows[positions[i]], skillId = item.status.skill_id;
          Object.assign(row, { status, reason: item.status.reason, skillId });
          if (status === "written") {
            const previous = written.get(skillId);
            if (previous && previous.batch < batchNumber) row.reason = `Mise à jour de la skill écrite par « ${previous.path} » (paquet séparé)`;
            if (!previous) written.set(skillId, { path: row.path, batch: batchNumber });
          }
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
        if (!supportedText(row.path)) { Object.assign(row, { status: "skipped", reason: "unsupported" }); continue; }
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
    const sectionTitle = node("h3", "", "Skills"); sectionTitle.tabIndex = -1;
    section.append(sectionTitle);
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
    fileInput.accept = EXTENSIONS.map((extension) => `.${extension}`).join(",");
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
    name.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.isComposing) { event.preventDefault(); description.focus(); }
    });
    const companionFiles = node("section", "skills-companion-files");
    companionFiles.setAttribute("aria-label", "Fichiers texte annexes de la version courante");
    fields.append(companionFiles);
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
    let returnFocusId = null, focusPending = false, reportGeneration = -1;
    const active = (request) => alive && request === token && dialog.open;
    dialog.addEventListener("close", () => {
      if (alive && !dialog.open) { token++; focusPending = true; render(); }
    });
    function showVersion() {
      const version = versions.find((item) => String(item.version) === select.value);
      preview.textContent = version ? `${version.deleted ? "Supprimée · " : ""}${displayName(version.content)}\n${version.content.description}\n\n${version.content.editable_body ?? version.content.body}${(version.content.files || []).map((file) => `\n\nFichier : ${file.path}\n${file.content}`).join("")}` : "";
    }
    select.addEventListener("change", showVersion);
    function populate(skill, allVersions) {
      current = skill; versions = allVersions;
      name.value = skill ? displayName(skill.content) : "";
      description.value = skill?.content.description || "";
      body.value = skill?.content.editable_body ?? skill?.content.body ?? "";
      // Textareas normalize CRLF. Compare with the displayed initial values so
      // editing a description never silently rewrites an untouched imported body.
      baseline = { name: name.value, description: description.value, body: body.value };
      const files = skill?.content.files || [];
      companionFiles.hidden = !files.length;
      companionFiles.replaceChildren(node("h3", "", `Fichiers annexes · ${files.length}`), node("p", "skills-note", "Textes conservés avec cette version. Demandez au bot une nouvelle proposition pour les modifier."), ...files.map((file) => {
        const detail = node("details");
        detail.append(node("summary", "", file.path), node("pre", "skills-version-preview", file.content));
        return detail;
      }));
      versionLabel.textContent = skill ? `Version ${skill.version}` : "Votre bot retrouve cette skill par son nom.";
      history.hidden = !skill; history.open = false;
      removeButton.hidden = !skill; confirmation.hidden = true;
      reload.hidden = true;
      select.replaceChildren(...versions.map((item) => {
        const option = node("option", "", `Version ${item.version}${item.deleted ? " · supprimée" : ""}`);
        option.value = String(item.version); return option;
      }));
      showVersion();
    }
    function focusName() {
      name.focus();
      name.setSelectionRange(0, 0);
      name.scrollLeft = 0;
    }
    async function openEditor(skillId) {
      if (job.running || job.mutating || (!skillId && (panels.state.skills?.by_bot?.[bot.id]?.length || 0) >= 250)) return;
      const request = ++token;
      returnFocusId = skillId; focusPending = false;
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
          populate(all[0], all); error.textContent = ""; fields.disabled = false; focusName();
        } catch (failure) { if (active(request)) error.textContent = failure.message; }
      } else { fields.disabled = false; focusName(); }
    }
    async function mutate(path, payload, success) {
      if (job.running || job.mutating) return;
      const request = token;
      job.mutating = true; fields.disabled = true; reload.disabled = true; error.textContent = ""; changed();
      try {
        const response = await panels.api(path, payload);
        if (active(request)) { returnFocusId = response.skill?.skill_id || returnFocusId; dialog.close(); }
        panels.notify(typeof success === "function" ? success(response) : success);
        await refresh();
      } catch (failure) {
        if (active(request)) {
          const uncertain = ["La connexion au moteur local est interrompue.", "Le moteur local a renvoyé une réponse illisible."].includes(failure.message);
          error.textContent = uncertain
            ? `Action non confirmée : ${failure.message} Vos saisies sont conservées. Vérifiez la liste avant de réessayer.`
            : `Action refusée : ${reason(failure.message)} Vos saisies sont conservées.${current ? " En cas de conflit, rechargez explicitement la version courante." : ""}`;
          reload.hidden = !current;
        } else panels.notify(reason(failure.message));
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
      if (!values.name.trim()) { error.textContent = "Ajoutez un nom à cette skill."; name.focus(); return; }
      // The host owns lookup-key transliteration and collision handling; this
      // field is the human title, not an ASCII slug.
      if ((!current || values.name !== baseline.name) && [...values.name.trim()].length > 128) { error.textContent = reason("name-too-large"); name.focus(); return; }
      if ((!current || values.description !== baseline.description) && [...values.description].length > 1024) { error.textContent = reason("description-too-large"); description.focus(); return; }
      if (!values.body.trim()) { error.textContent = "Ajoutez un contenu à cette skill."; body.focus(); return; }
      // Imported annotations can make an unchanged body longer than the limit.
      if ((!current || values.body !== baseline.body) && values.body.length > 256000) {
        error.textContent = "Le contenu dépasse 256 000 caractères."; return;
      }
      const changes = current ? Object.fromEntries(Object.entries(values).filter(([key, value]) => value !== baseline[key])) : values;
      if (current && !Object.keys(changes).length) { dialog.close(); return; }
      void mutate(current ? "/api/skills/update" : "/api/skills", { bot_id: bot.id, ...(current ? { skill_id: current.skill_id, expected_version: current.version } : {}), ...changes }, ({ skill }) => `Skill « ${displayName(skill.content)} » enregistrée.`);
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
      add.disabled = locked || skills.length >= 250;
      const next = JSON.stringify([skills, locked, panels.state.skills?.error]);
      if (signature !== next) {
        signature = next;
        list.replaceChildren();
        sectionTitle.textContent = `Skills · ${skills.length}/250`;
        if (!skills.length) list.append(node("li", "skills-note", panels.state.skills?.error || "Aucune skill pour ce bot."));
        const titleCounts = new Map();
        for (const skill of skills) titleCounts.set(displayName(skill), (titleCounts.get(displayName(skill)) || 0) + 1);
        for (const skill of skills) {
          const row = node("li", "skills-row"), copy = node("div");
          const lookupKey = titleCounts.get(displayName(skill)) > 1 ? ` · ${skill.name}${skill.name_truncated ? "…" : ""}` : "";
          copy.append(node("strong", "", displayName(skill) + (skill.display_name_truncated ? "…" : "")), node("small", "", (skill.description || "Sans description") + (skill.description_truncated ? "…" : "")), node("small", "", `Version ${skill.version}${lookupKey}`));
          const edit = button("Modifier / historique", () => void openEditor(skill.skill_id));
          edit.setAttribute("aria-label", `Modifier / historique : ${displayName(skill)}`); edit.disabled = locked;
          edit.dataset.skillId = skill.skill_id;
          row.append(copy, edit); list.append(row);
        }
      }
      if (focusPending && !locked && !dialog.open) {
        focusPending = false;
        if (section.isConnected && document.getElementById("bot-dialog")?.open) {
          const target = [...list.querySelectorAll("button")].find((edit) => edit.dataset.skillId === returnFocusId);
          (target || sectionTitle).focus();
        }
      }
      if (reportGeneration !== job.generation) {
        reportGeneration = job.generation;
        report.open = job.rows.length > 0;
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
      }
    }
    mounts.add(render); render();
    return () => { alive = false; token++; mounts.delete(render); dialog.close(); dialog.remove(); };
  }
  panels.register({ id: "skills", botSection, onState: changed });
})();
