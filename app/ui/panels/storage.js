"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, cls, text) => {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const size = (bytes) => window.GrokbotChat.formatAttachmentSize(bytes);
  let section, usage, facts, notice, confirmation, clean, refresh, pending = false;
  function button(text, cls, action) {
    const el = node("button", cls, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  }
  function message(text, error = false) {
    notice.textContent = text;
    notice.classList.toggle("storage-error", error);
  }
  function busy(value) {
    pending = value;
    clean.disabled = refresh.disabled = value;
    section.setAttribute("aria-busy", String(value));
    facts.querySelectorAll("button").forEach((b) => { b.disabled = value; });
  }
  function closeConfirmation() { confirmation.replaceChildren(); confirmation.hidden = true; }
  function confirm(title, description, label, action) {
    closeConfirmation();
    confirmation.hidden = false;
    const heading = node("h4", "", title);
    heading.tabIndex = -1;
    const actions = node("div", "storage-actions");
    actions.append(button("Annuler", "secondary-button", () => { closeConfirmation(); clean.focus(); }),
      button(label, "danger-button", async () => {
        if (pending) return;
        busy(true);
        actions.querySelectorAll("button").forEach((b) => { b.disabled = true; });
        try {
          const result = await action();
          closeConfirmation();
          await load();
          message(result);
          await panels.refresh();
        } catch (error) { message(error.message, true); closeConfirmation(); }
        finally { busy(false); clean.focus(); }
      }));
    confirmation.append(heading, node("p", "", description), actions);
    heading.focus();
  }
  async function load() {
    try {
      const data = await panels.api("/api/storage");
      const u = data.usage;
      usage.replaceChildren();
      for (const [label, bytes, count, unit] of [
        ["Images", u.images, u.image_count, "fichier"],
        ["Documents", u.documents, u.document_count, "extraction"],
        ["Mémoire", u.memory, u.fact_count, "fait"],
      ]) {
        const row = node("div", "storage-usage-row");
        row.append(node("span", "", label), node("strong", "", size(bytes)), node("small", "", `${count} ${unit}${count === 1 ? "" : "s"}`));
        usage.append(row);
      }
      facts.replaceChildren(node("h4", "", "Mémoire des bots supprimés"));
      if (!data.orphan_facts.length) facts.append(node("p", "", "Aucun fait privé à supprimer."));
      else {
        facts.append(node("p", "", "Les faits partagés avec l’équipe sont conservés. Supprimer un fait efface aussi ses anciennes versions privées, pas vos messages."));
        for (const fact of data.orphan_facts) {
          const row = node("div", "storage-fact");
          const copy = node("div");
          copy.append(node("strong", "", `${fact.bot_name} · Privé${fact.status === "superseded" ? " · Ancienne version" : ""}`), node("p", "", fact.text));
          row.append(copy, button("Supprimer", "secondary-button", () => confirm("Supprimer ce fait privé ?", `« ${fact.text} » — Cette action est définitive. Les messages de vos conversations restent conservés.`, "Confirmer la suppression", async () => {
            await panels.api("/api/storage/facts/delete", { fact_id: fact.id, confirm: true });
            return "Fait privé supprimé.";
          })));
          facts.append(row);
        }
      }
      return true;
    } catch (error) { message(error.message, true); return false; }
  }
  panels.register({
    id: "storage",
    settingsSection(container) {
      section = container;
      section.className = "setting-section storage-section";
      section.append(node("h3", "", "Stockage"), node("p", "", "Données locales de l’application. Seules les pièces jointes sans référence peuvent être nettoyées. Les brouillons et les fichiers utilisés sont conservés."));
      usage = node("div", "storage-usage");
      usage.setAttribute("aria-label", "Espace utilisé");
      const explanation = node("p", "", "Documents : taille des extractions conservées, sans copie du fichier d’origine. Mémoire : contenu et provenance, marqueurs d’oubli inclus, hors historique. L’espace SQLite libéré est réutilisé par l’application. Les imports abandonnés depuis plus de 24 h sont nettoyés au démarrage.");
      const actions = node("div", "storage-actions");
      refresh = button("Actualiser", "secondary-button", async () => { closeConfirmation(); message(""); busy(true); await load(); busy(false); });
      clean = button("Nettoyer", "secondary-button", async () => {
        if (pending) return;
        busy(true); message(""); closeConfirmation();
        try {
          const preview = await panels.api("/api/storage/preview", {});
          if (!preview.count) { message("Aucune pièce jointe inutilisée à nettoyer."); return; }
          confirm("Confirmer le nettoyage ?", `${preview.count} élément(s) inutilisé(s) · ${size(preview.bytes)} à libérer. Aucun fichier référencé par un message, un brouillon, une tâche ou une skill ne sera supprimé. L’aperçu expire après 10 minutes.`, "Confirmer le nettoyage", async () => {
            const result = await panels.api("/api/storage/clean", { token: preview.token, confirm: true });
            return `${result.deleted} élément(s) nettoyé(s) · ${size(result.bytes)} libérés.${result.retained ? ` ${result.retained} élément(s) conservé(s) car leur état a changé.` : ""}`;
          });
        } catch (error) { message(error.message, true); }
        finally { busy(false); }
      });
      actions.append(clean, refresh);
      confirmation = node("div", "storage-confirmation");
      confirmation.hidden = true;
      notice = node("p", "storage-notice");
      notice.setAttribute("role", "status");
      facts = node("div", "storage-facts");
      section.append(usage, explanation, actions, confirmation, notice, facts);
      const dialog = document.getElementById("settings-dialog");
      new MutationObserver(() => {
        if (dialog.open) { closeConfirmation(); message(""); load(); }
      }).observe(dialog, { attributes: true, attributeFilter: ["open"] });
    },
  });
})();
