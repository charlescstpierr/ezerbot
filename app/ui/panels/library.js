/* Bibliothèque du bot : fichiers de contexte conservés pour les nouveaux fils. */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const MAX_BYTES = 20 * 1024 * 1024;
  const EXTENSIONS = ["pdf", "docx", "xlsx", "csv", "pptx", "md", "txt"];
  const FORMAT_ERROR = "Format non accepté : PDF, DOCX, XLSX, CSV, PPTX, MD ou TXT.";
  const SIZE_ERROR = "Le document dépasse la limite de 20 Mo.";
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
  function sizeLabel(bytes) {
    const value = Number(bytes) || 0;
    if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} Mo`;
    if (value >= 1024) return `${(value / 1024).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} Ko`;
    return `${value.toLocaleString("fr-FR")} octets`;
  }
  function dateLabel(stamp) {
    const date = new Date(stamp);
    if (!Number.isFinite(Number(stamp)) || Number.isNaN(date.getTime())) return "";
    return date.toLocaleString("fr-CA", { dateStyle: "medium", timeStyle: "short" });
  }
  function extension(name) {
    const base = String(name || "").split(/[\\/]/).pop();
    const dot = base.lastIndexOf(".");
    return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
  }
  async function base64(file) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = "";
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    return btoa(binary);
  }

  panels.register({
    id: "library",
    botSection(bot, container) {
      const section = node("section", "library-panel");
      section.setAttribute("aria-label", "Bibliothèque du bot");
      section.append(node("h3", "", "Bibliothèque"));
      section.append(node("p", "library-note", "Plafond : 20 Mo. Le fichier reste sur ce bot : un nouveau fil peut le citer. Un autre bot ne le reçoit pas."));
      container.append(section);
      if (!bot?.id || bot.deleted) {
        section.append(node("p", "library-note", "Enregistrez d’abord ce bot pour déposer des fichiers dans sa bibliothèque."));
        return;
      }
      let alive = true;
      let shown = [];
      let pending = null;
      const drop = node("div", "library-drop");
      drop.tabIndex = 0;
      drop.setAttribute("role", "button");
      drop.setAttribute("aria-label", "Déposer un fichier dans la bibliothèque");
      drop.append(node("strong", "", "Déposez un fichier ici"));
      drop.append(node("span", "", "Glissez-déposez un fichier, ou choisissez-le. PDF, DOCX, XLSX, CSV, PPTX, MD ou TXT."));
      const fileInput = node("input", "library-file-input");
      fileInput.type = "file";
      fileInput.accept = EXTENSIONS.map((item) => `.${item}`).join(",");
      fileInput.setAttribute("aria-label", "Fichier de bibliothèque");
      const status = node("p", "library-note library-status");
      status.setAttribute("role", "status");
      const error = node("p", "library-error");
      error.setAttribute("role", "alert");
      const list = node("ul", "library-list");
      const confirm = node("div", "library-confirm");
      confirm.hidden = true;
      const confirmText = node("p", "library-note");
      const confirmRemove = button("Retirer le fichier", () => { void commitRemove(); }, "danger-button");
      const cancel = button("Annuler", () => { pending = null; confirm.hidden = true; });
      confirm.append(confirmText, confirmRemove, cancel);
      section.append(drop, fileInput, status, error, list, confirm);

      function render(files) {
        if (!alive) return;
        shown = Array.isArray(files) ? files : [];
        if (pending && !shown.some((file) => file.id === pending.id)) {
          pending = null;
          confirm.hidden = true;
        }
        list.replaceChildren();
        if (!shown.length) {
          list.append(node("li", "library-empty", "Aucun fichier pour l’instant."));
          return;
        }
        for (const file of shown) {
          const row = node("li", "library-row");
          const copy = node("div", "library-copy");
          const format = String(file.format || "").toUpperCase();
          const when = dateLabel(file.added_at);
          copy.append(node("strong", "", file.name), node("small", "", `${format} · ${sizeLabel(file.byte_length)}${when ? ` · ${when}` : ""}`));
          const remove = button("Retirer", () => askRemove(file));
          remove.setAttribute("aria-label", `Retirer ${file.name}`);
          row.append(copy, remove);
          list.append(row);
        }
      }
      function askRemove(file) {
        pending = file;
        confirmText.textContent = `Retirer ${file.name} ? Les nouveaux fils de ce bot ne le citeront plus.`;
        confirm.hidden = false;
      }
      async function commitRemove() {
        if (!pending) return;
        const file = pending;
        confirmRemove.disabled = true;
        try {
          const saved = await panels.api("/api/library/remove", { bot_id: bot.id, id: file.id });
          if (!alive) return;
          pending = null;
          confirm.hidden = true;
          render(saved.files || []);
          error.textContent = "";
          status.textContent = `${file.name} a été retiré.`;
        } catch (failure) {
          if (!alive) return;
          error.textContent = failure.message;
          panels.notify(failure.message);
        } finally {
          confirmRemove.disabled = false;
        }
      }
      async function ingest(file) {
        if (!alive || !file) return;
        error.textContent = "";
        if (!EXTENSIONS.includes(extension(file.name))) {
          status.textContent = "";
          error.textContent = FORMAT_ERROR;
          return;
        }
        if (file.size > MAX_BYTES) {
          status.textContent = "";
          error.textContent = SIZE_ERROR;
          return;
        }
        let data;
        try { data = await base64(file); }
        catch {
          if (!alive) return;
          error.textContent = "Lecture du fichier impossible.";
          return;
        }
        const replacing = shown.some((item) => item.name === file.name);
        try {
          const saved = await panels.api("/api/library", { bot_id: bot.id, name: file.name, data });
          if (!alive) return;
          render(saved.files || []);
          status.textContent = replacing
            ? `${file.name} remplace la version précédente.`
            : `${file.name} est dans la bibliothèque.`;
        } catch (failure) {
          if (!alive) return;
          status.textContent = "";
          error.textContent = failure.message;
          panels.notify(failure.message);
        }
      }
      async function ingestAll(files) {
        for (const file of files) await ingest(file);
      }
      drop.addEventListener("click", () => fileInput.click());
      drop.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          fileInput.click();
        }
      });
      drop.addEventListener("dragover", (event) => {
        event.preventDefault();
        drop.classList.add("is-dropping");
      });
      drop.addEventListener("dragleave", () => drop.classList.remove("is-dropping"));
      drop.addEventListener("drop", async (event) => {
        event.preventDefault();
        drop.classList.remove("is-dropping");
        await ingestAll([...(event.dataTransfer?.files || [])]);
      });
      fileInput.addEventListener("change", async () => {
        const files = [...(fileInput.files || [])];
        fileInput.value = "";
        await ingestAll(files);
      });
      panels.api(`/api/library?bot_id=${encodeURIComponent(bot.id)}`).then((data) => {
        render(data.files || []);
      }).catch((failure) => {
        if (!alive) return;
        error.textContent = failure.message;
      });
      return () => { alive = false; };
    },
  });
})();
