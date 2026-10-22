"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const BATCH_BYTES = 8 * 1024 * 1024;
  const EXTENSIONS = ["txt", "md", "markdown", "csv", "json", "yaml", "yml"];
  const displayName = (skill) => skill.display_name || skill.name;
  function replyFollowsAnnex(reply, annex) {
    const marker = "Réponds exactement : ";
    const at = String(annex).indexOf(marker);
    if (at >= 0) {
      const phrase = annex.slice(at + marker.length).split("\n")[0].trim();
      return phrase.length > 0 && String(reply).trim() === phrase;
    }
    const text = String(annex).trim();
    return text.length > 0 && String(reply).includes(text);
  }
  function supportedText(path) {
    const base = path.split(/[\\/]/).pop();
    const dot = base.lastIndexOf(".");
    return dot > 0 && EXTENSIONS.includes(base.slice(dot + 1).toLowerCase());
  }
  const encoder = new TextEncoder();
  const sessions = new Map();
  const mounts = new Set();
  const linkEditors = new Map();
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
  const reason = (value) => ({ unsupported: "Format non accepté", empty: "Fichier vide", "too-large": "Texte trop volumineux", "combined-too-large": "Le contenu et les annexes conservées dépassent ensemble 256 000 caractères", "too-many-annexes": "Une skill accepte au plus 20 fichiers texte annexes.", "too-many-skills": "Limite de 250 skills atteinte", "name-too-large": "Le nom ne doit pas dépasser 128 caractères.", "description-too-large": "La description ne doit pas dépasser 1 024 caractères.", "skill name must contain a letter or digit": "Le nom doit contenir une lettre latine (accents acceptés) ou un chiffre.", "invalid-path": "Ce chemin sort du dossier de la skill ou le nom d’annexe est invalide.", symlink: "Un lien symbolique n’est pas importé.", "app-data": "Les données d’Ezerbot ne s’importent pas comme skills.", "duplicate-path": "Ce nom d’annexe est déjà utilisé." }[value] || value);
  function session(botId) {
    if (!sessions.has(botId)) sessions.set(botId, { running: false, mutating: false, generation: 0, rows: [], message: "", points: { import: false, annex: false, restore: false, shortcut: false } });
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
  function normalizedParts(path) {
    const raw = String(path);
    if (raw.startsWith("/") || raw.startsWith("\\")) return null;
    const parts = [];
    for (const part of raw.replaceAll("\\", "/").split("/")) {
      if (!part || part === ".") continue;
      if (part === "..") {
        if (!parts.length) return null;
        parts.pop();
        continue;
      }
      parts.push(part);
    }
    return parts.length ? parts : null;
  }
  const parentDir = (path) => { const parts = normalizedParts(path); return !parts || parts.length < 2 ? "" : parts.slice(0, -1).join("/"); };
  const baseName = (path) => { const parts = normalizedParts(path); return parts ? parts[parts.length - 1] : ""; };
  const climbsOut = (path) => String(path).split(/[\\/]/).includes("..");
  function skillFolders(files) {
    const members = new Map();
    files.forEach((file, index) => {
      const path = file.webkitRelativePath || file.name;
      const dir = parentDir(path);
      if (dir && baseName(path) === "SKILL.md" && !members.has(dir)) members.set(dir, [index]);
    });
    files.forEach((file, index) => {
      const path = file.webkitRelativePath || file.name;
      const dir = parentDir(path);
      const group = dir && members.get(dir);
      if (!group || group[0] === index || baseName(path) === "SKILL.md") return;
      group.push(index);
    });
    return members;
  }
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
    const folders = skillFolders(files);
    const grouped = new Set([...folders.values()].flat());
    async function flush() {
      if (!batch.length) return;
      batchNumber++;
      try {
        const { report } = await panels.api("/api/skills/import", { bot_id: botId, files: batch });
        if (!Array.isArray(report) || report.length !== positions.length) throw new Error("Rapport incomplet du moteur local.");
        report.forEach((item, i) => {
          const status = item.status?.status;
          if (!["written", "skipped", "duplicate", "annex"].includes(status)) throw new Error("Statut d’import inconnu.");
          const row = job.rows[positions[i]], skillId = item.status.skill_id;
          Object.assign(row, { status, reason: item.status.reason, skillId });
          if (status === "annex") job.points.import = true;
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
    async function prepare(index) {
      const row = job.rows[index], file = files[index];
      if (file.symlink) { Object.assign(row, { status: "skipped", reason: "symlink" }); return null; }
      if (!normalizedParts(row.path) || (climbsOut(row.path) && !folders.has(parentDir(row.path)))) {
        Object.assign(row, { status: "skipped", reason: "invalid-path" });
        return null;
      }
      if (!supportedText(row.path)) { Object.assign(row, { status: "skipped", reason: "unsupported" }); return null; }
      // No file this large can fit a batch, even before base64 expansion.
      if (file.size > BATCH_BYTES) { Object.assign(row, { status: "skipped", reason: "too-large" }); return null; }
      let upload;
      try { upload = { path: row.path, base64: await base64(file) }; }
      catch { Object.assign(row, { status: "skipped", reason: "Lecture du fichier impossible" }); return null; }
      const bytes = encoder.encode(JSON.stringify(upload)).length;
      if (overhead + bytes > BATCH_BYTES) { Object.assign(row, { status: "skipped", reason: "too-large" }); return null; }
      return { upload, bytes };
    }
    const groupCost = (prepared, existing) => prepared.reduce((sum, item, index) => sum + item.bytes + ((existing || index) ? 1 : 0), 0);
    try {
      const queued = new Set();
      for (let i = 0; i < files.length; i++) {
        if (interrupted || queued.has(i)) continue;
        const dir = parentDir(job.rows[i].path);
        const indexes = grouped.has(i) ? folders.get(dir) : [i];
        const prepared = [];
        const primary = indexes[0];
        const folder = indexes.length > 1 && grouped.has(primary);
        for (const index of indexes) {
          queued.add(index);
          if (folder && index !== primary && job.rows[primary].status === "skipped") {
            Object.assign(job.rows[index], { status: "skipped", reason: job.rows[primary].reason });
            continue;
          }
          const item = await prepare(index);
          if (item) prepared.push({ index, ...item });
        }
        if (!prepared.length || interrupted) continue;
        if (batch.length && (batch.length + prepared.length > 1000 || size + groupCost(prepared, batch.length) > BATCH_BYTES)) await flush();
        if (interrupted) break;
        if (prepared.length > 1000 || overhead + groupCost(prepared, 0) > BATCH_BYTES) {
          for (const item of prepared) Object.assign(job.rows[item.index], { status: "skipped", reason: "too-large" });
          continue;
        }
        for (const item of prepared) {
          size += item.bytes + (batch.length ? 1 : 0);
          batch.push(item.upload); positions.push(item.index);
        }
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

  function grantedFolders() {
    const folders = panels.state.local?.folders;
    return Array.isArray(folders) ? folders.filter((folder) => typeof folder === "string" && folder.startsWith("/")) : [];
  }
  async function importGrantedFolder(botId, path) {
    const job = session(botId);
    if (job.running || job.mutating) return;
    job.running = true;
    job.generation++;
    job.rows = [{ path, status: "pending" }];
    job.message = "Import en cours… Les paquets déjà écrits sont conservés.";
    changed();
    try {
      const { report } = await panels.api("/api/skills/import-folder", { bot_id: botId, path });
      if (!Array.isArray(report)) throw new Error("Rapport incomplet du moteur local.");
      const rows = report.map((item) => {
        const status = item.status?.status;
        if (!["written", "skipped", "duplicate", "annex"].includes(status)) throw new Error("Statut d’import inconnu.");
        if (status === "annex") job.points.import = true;
        return { path: item.file_name, status, reason: item.status.reason, skillId: item.status.skill_id };
      });
      job.rows = rows;
      job.message = "Import terminé. Les succès sont conservés même si d’autres fichiers sont refusés.";
    } catch (error) {
      job.rows = [{ path, status: "unknown", reason: error.message }];
      job.message = "Import interrompu : certains résultats sont incertains. Vérifiez la liste avant de réimporter; les écritures déjà reçues restent conservées.";
    } finally {
      await refresh();
      job.running = false;
      changed();
    }
  }
  let folderPicker = null;
  function chooseGrantedFolder(botId) {
    const folders = grantedFolders();
    if (!folders.length) {
      panels.notify("Accordez d’abord ce dossier dans les fichiers locaux.");
      return;
    }
    if (folders.length === 1) {
      void importGrantedFolder(botId, folders[0]);
      return;
    }
    folderPicker?.close();
    const picker = node("dialog", "dialog skills-folder-picker");
    folderPicker = picker;
    picker.setAttribute("aria-label", "Dossier accordé à importer");
    const list = node("ul", "skills-folder-choices");
    for (const path of folders) {
      const item = node("li");
      item.append(button(path, () => {
        picker.close();
        void importGrantedFolder(botId, path);
      }));
      list.append(item);
    }
    picker.append(node("h2", "", "Importer un dossier accordé"), list, button("Annuler", () => picker.close()));
    picker.addEventListener("close", () => { if (folderPicker === picker) folderPicker = null; picker.remove?.(); });
    (document.getElementById("panels") || document.body).append(picker);
    picker.showModal();
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
    const scenario = node("div", "skills-scenario");
    const scenarioScore = node("p", "skills-scenario-score", "Scénario SKL-3 · 0/4");
    const scenarioList = node("ul", "skills-scenario-points");
    const pointNodes = new Map([["import", "Import du dossier"], ["annex", "Annexe utilisable"], ["restore", "Version précédente restaurable"], ["shortcut", "Raccourci ⌘⇧W"]].map(([key, label]) => {
      const item = node("li", "", label);
      item.dataset.point = key;
      item.dataset.done = "false";
      scenarioList.append(item);
      return [key, item];
    }));
    scenario.append(scenarioScore, scenarioList);
    section.append(node("p", "skills-note", "Textes UTF-8 : txt, md, markdown, csv, json, yaml, yml. 256 000 caractères par skill, annexes comprises · 250 skills par bot."), scenario, actions, list, progress, report);
    const fileInput = node("input", "skills-file-input");
    fileInput.type = "file"; fileInput.multiple = true;
    fileInput.accept = EXTENSIONS.map((extension) => `.${extension}`).join(",");
    fileInput.name = "skills-files";
    fileInput.setAttribute("aria-label", "Fichiers de skills");
    fileInput.addEventListener("change", () => {
      const files = Array.from(fileInput.files || []);
      fileInput.value = ""; // Re-selecting the same file must still trigger change.
      void importFiles(bot.id, files);
    });
    const add = button("Nouvelle skill", () => void openEditor(null));
    const chooseFiles = button("Importer des fichiers", () => fileInput.click());
    const chooseDirectory = button("Importer un dossier", () => chooseGrantedFolder(bot.id));
    chooseDirectory.title = "Lit un dossier déjà accordé dans Fichiers locaux, sans suivre les liens.";
    const demonstrate = button("Montrer au bot", () => window.EzerbotDemonstration?.open(bot.id));
    actions.append(add, demonstrate, chooseFiles, chooseDirectory, fileInput);
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
    const preview = node("div", "skills-history-content");
    history.append(node("summary", "", "Historique des versions"), select, preview);
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
    function fileDetails(file) {
      const detail = node("details");
      detail.append(node("summary", "", file.path), node("pre", "skills-version-preview", file.content));
      return detail;
    }
    function showVersion() {
      const version = versions.find((item) => String(item.version) === select.value);
      preview.replaceChildren();
      if (!version) return;
      const content = version.content;
      preview.append(node("h4", "", `${version.deleted ? "Supprimée · " : ""}${displayName(content)}`),
        node("p", "skills-note", content.description), node("pre", "skills-version-preview", content.editable_body ?? content.body));
      if (content.files?.length) {
        const files = node("section", "skills-companion-files");
        files.setAttribute("aria-label", `Fichiers annexes de la version ${version.version}`);
        files.append(node("h3", "", `Fichiers annexes · ${content.files.length}`), ...content.files.map(fileDetails));
        preview.append(files);
      }
      if (current && String(version.version) !== String(current.version) && !version.deleted) {
        preview.append(button("Restaurer cette version", () => void restoreVersion(version.version)));
      }
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
      companionFiles.replaceChildren(node("h3", "", `Fichiers annexes · ${files.length}`), node("p", "skills-note", "Textes conservés avec cette version. Demandez au bot une nouvelle proposition pour les modifier."), ...files.map(fileDetails));
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
    async function restoreVersion(version) {
      if (!current || fields.disabled || job.running || job.mutating) return;
      const request = token;
      job.mutating = true; fields.disabled = true; error.textContent = ""; changed();
      try {
        await panels.api("/api/skills/restore", { bot_id: bot.id, skill_id: current.skill_id, expected_version: current.version, version: Number(version) });
        if (!active(request)) return;
        const response = await panels.api(`/api/skills/versions?bot_id=${encodeURIComponent(bot.id)}&skill_id=${encodeURIComponent(current.skill_id)}`);
        if (!active(request)) return;
        const all = response.versions.slice().sort((a, b) => b.version - a.version);
        if (!all.length || all[0].deleted) throw new Error("Cette skill a été supprimée. Rouvrez la liste.");
        populate(all[0], all);
        job.points.restore = true;
        error.textContent = "";
        panels.notify(`Version ${version} restaurée, annexes comprises.`);
        await refresh();
      } catch (failure) {
        if (active(request)) error.textContent = `Action refusée : ${reason(failure.message)}`;
        await refresh();
      } finally {
        job.mutating = false;
        if (active(request)) { fields.disabled = false; }
        changed();
      }
    }
    async function launchSkill(skill) {
      if (job.running || job.mutating) return;
      for (const dialog of document.querySelectorAll("dialog[open]")) dialog.close();
      job.mutating = true; changed();
      try {
        let conversation = (panels.state.conversations || []).find((item) => item.bot_id === bot.id && !item.deleted && item.kind !== "task" && item.kind !== "routine");
        if (!conversation) {
          const created = await panels.api("/api/conversations", { bot_id: bot.id });
          conversation = created.conversation;
        }
        const versions = await panels.api(`/api/skills/versions?bot_id=${encodeURIComponent(bot.id)}&skill_id=${encodeURIComponent(skill.skill_id)}`);
        const latest = (versions.versions || []).slice().sort((a, b) => b.version - a.version).find((item) => !item.deleted);
        const annexes = (latest?.content?.files || []).map((file) => file.content).filter((content) => content);
        const messageId = crypto.randomUUID();
        const sent = await panels.api("/api/send", {
          conversation_id: conversation.id,
          message_id: messageId,
          body: `Lance la skill « ${displayName(skill)} ».`,
          apply_skill_id: skill.skill_id,
        });
        job.pendingLaunch = {
          conversationId: conversation.id,
          messageId,
          annexes,
          snapshot: sent.conversation?.messages ?? [],
        };
        job.points.annex = false;
        evaluateLaunch();
        // A just-created thread is absent from state until refresh. Opening
        // first leaves the welcome screen up and hides the annex reply.
        await refresh();
        panels.openConversation?.(conversation.id);
        panels.notify(`Skill « ${displayName(skill)} » lancée.`);
      } catch (failure) {
        panels.notify(reason(failure.message));
      } finally {
        job.mutating = false;
        changed();
      }
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
    function replyStillPending(message, conversation) {
      const group = Boolean(conversation && (conversation.kind === "group" || Array.isArray(conversation.bot_ids)));
      if (typeof window.GrokbotChat?.isPendingReply === "function") return window.GrokbotChat.isPendingReply(message, group);
      return message?.state === "streaming" || (group && ["queued", "running"].includes(message?.payload?.turn_status));
    }
    function evaluateLaunch() {
      const pending = job.pendingLaunch;
      if (!pending) return;
      const live = (panels.state.conversations || []).find((item) => item.id === pending.conversationId);
      const liveMessages = live?.messages;
      const messages = Array.isArray(liveMessages) && liveMessages.some((message) => message.id === pending.messageId)
        ? liveMessages
        : (pending.snapshot || []);
      const start = messages.findIndex((message) => message.id === pending.messageId);
      if (start < 0) {
        job.points.annex = false;
        return;
      }
      const reply = [...messages.slice(start + 1)].reverse().find((message) => message.role === "assistant");
      if (!reply || replyStillPending(reply, live)) {
        job.points.annex = false;
        return;
      }
      job.points.annex = pending.annexes.some((annex) => replyFollowsAnnex(reply.body ?? "", annex));
    }
    function render() {
      if (!alive) return;
      evaluateLaunch();
      const locked = job.running || job.mutating;
      const recording = panels.state.demonstrations?.session;
      demonstrate.textContent = recording?.status === "recording" && recording.bot_id === bot.id ? "Démonstration en cours…" : "Montrer au bot";
      demonstrate.title = recording?.status === "recording" && recording.bot_id === bot.id ? "Ouvrir les contrôles de la démonstration" : "Montrer une tâche sur votre Mac";
      for (const control of [add, demonstrate, chooseFiles, chooseDirectory, fileInput]) control.disabled = locked;
      const skills = panels.state.skills?.by_bot?.[bot.id] || [];
      add.disabled = locked || skills.length >= 250;
      const next = JSON.stringify([skills, locked, panels.state.skills?.error, panels.state.demonstrations?.links]);
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
          copy.append(node("strong", "", displayName(skill) + (skill.display_name_truncated ? "…" : "")), node("small", "", (skill.description || "Sans description") + (skill.description_truncated ? "…" : "")), node("small", "", `Version ${skill.version}${skill.file_count ? ` · ${skill.file_count} ${skill.file_count === 1 ? "annexe" : "annexes"}` : ""}${lookupKey}`));
          const edit = button("Modifier / historique", () => void openEditor(skill.skill_id));
          edit.setAttribute("aria-label", `Modifier / historique : ${displayName(skill)}`); edit.disabled = locked;
          edit.dataset.skillId = skill.skill_id;
          const launch = button("Lancer", () => void launchSkill(skill));
          launch.disabled = locked;
          launch.setAttribute("aria-label", `Lancer : ${displayName(skill)}`);
          const rowActions = node("div", "skills-row-actions"); rowActions.append(edit);
          const demonstration = panels.state.demonstrations?.links?.find(link => link.skill_id === skill.skill_id);
          if (demonstration) {
            const view = button("Voir la démonstration", () => window.EzerbotDemonstration?.showSummary(demonstration.id));
            view.setAttribute("aria-label", `Voir la démonstration : ${displayName(skill)}`); rowActions.append(view);
          }
          rowActions.append(launch);
          row.append(copy, rowActions); list.append(row);
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
      const done = ["import", "annex", "restore", "shortcut"].filter((key) => job.points[key]).length;
      scenarioScore.textContent = `Scénario SKL-3 · ${done}/4`;
      for (const [key, item] of pointNodes) item.dataset.done = job.points[key] ? "true" : "false";
      reportTitle.textContent = `Rapport : ${countLabel("written", "écrite")} · ${countLabel("duplicate", "doublon")} · ${countLabel("skipped", "refusée")} · ${countLabel("unknown", "incertaine")} · ${countLabel("not_sent", "non envoyée")} · ${countLabel("pending", "en attente", "en attente")}`;
      const nextReport = JSON.stringify(job.rows);
      if (nextReport !== reportSignature) {
        reportSignature = nextReport;
        const labels = { written: "Écrite", duplicate: "Déjà importée", annex: "Annexe conservée", skipped: "Refusée", unknown: "Résultat incertain", not_sent: "Non envoyée", pending: "En attente" };
        reportRows.replaceChildren(...job.rows.map((item) => {
          const row = node("li");
          row.dataset.status = item.status;
          row.append(node("span", "skills-report-path", item.path), node("span", "", `${labels[item.status]}${item.reason ? ` — ${reason(item.reason)}` : ""}`));
          return row;
        }));
      }
    }
    mounts.add(render); render();
    linkEditors.set(bot.id, openEditor);
    return () => { alive = false; token++; mounts.delete(render); linkEditors.delete(bot.id); dialog.close(); dialog.remove(); };
  }
  panels.register({ id: "skills", botSection, onState: changed,
    noteShortcut(botId) {
      const job = session(botId);
      job.points.shortcut = true;
      changed();
    },
    openLink({ id, agent }) {
      const open = linkEditors.get(agent);
      if (!open || session(agent).running || session(agent).mutating) return false;
      return open(id);
    },
  });
})();
