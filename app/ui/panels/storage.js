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
  const plural = (n, singular, many = `${singular}s`) => n <= 1 ? singular : many;
  const counted = (n, singular, many) => `${n} ${plural(n, singular, many)}`;
  let section, usage, facts, factsTitle, factsList, factsDescription, factNotice;
  let notice, confirmation, clean, refresh, dialog, activeConfirmation;
  let pending = false, loading = 0, loadRevision = 0, needsReload = false, botsSignature;
  function button(text, cls, action) {
    const el = node("button", cls, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  }
  function message(text, error = false, kind = "clean") {
    const target = kind === "fact" ? factNotice : notice;
    target.textContent = text;
    target.classList.toggle("storage-error", error);
  }
  function reloadIfNeeded() {
    if (!needsReload || loading || pending || activeConfirmation || !dialog?.open) return;
    needsReload = false;
    return load();
  }
  function busy(value) {
    pending = value;
    clean.disabled = refresh.disabled = value;
    section.setAttribute("aria-busy", String(value));
    facts.querySelectorAll("button").forEach((b) => { b.disabled = value; });
    if (!value) void reloadIfNeeded();
  }
  function closeConfirmation() {
    if (activeConfirmation?.card !== confirmation) activeConfirmation?.card.remove();
    activeConfirmation = null;
    confirmation.replaceChildren();
    confirmation.hidden = true;
  }
  function restoreFocus(kind, origin) {
    if (kind === "clean") clean.focus();
    else if (origin?.isConnected) origin.focus();
    else {
      const id = origin?.parentElement?.dataset.factId;
      const replacement = [...factsList.querySelectorAll(".storage-fact")].find((row) => row.dataset.factId === id)?.querySelector("button");
      (replacement || factsTitle).focus();
    }
  }
  function confirm(title, description, label, action, { anchor, origin, kind = "clean" } = {}) {
    closeConfirmation();
    message("", false, kind);
    const card = anchor ? node("div", "storage-confirmation storage-fact-confirmation") : confirmation;
    if (anchor) anchor.after(card);
    card.hidden = false;
    activeConfirmation = { card };
    const heading = node("h4", "", title);
    const actions = node("div", "storage-actions");
    const cancel = button("Annuler", "secondary-button", async () => {
      closeConfirmation();
      await reloadIfNeeded();
      restoreFocus(kind, origin);
    });
    actions.append(cancel, button(label, "danger-button", async () => {
      if (pending) return;
      busy(true);
      actions.querySelectorAll("button").forEach((b) => { b.disabled = true; });
      try {
        const result = await action();
        closeConfirmation();
        const loaded = await load();
        if (loaded) message(result, false, kind);
        await panels.refresh();
      } catch (error) { message(error.message, true, kind); closeConfirmation(); }
      finally { busy(false); restoreFocus(kind, origin); }
    }));
    card.append(heading, node("p", "", description), actions);
    card.scrollIntoView({ block: "nearest" });
    cancel.focus({ preventScroll: true });
  }
  async function load() {
    const revision = ++loadRevision;
    needsReload = false;
    loading++;
    try {
      const data = await panels.api("/api/storage");
      // A bot can be deleted while a prior storage read is still in flight.
      // Never briefly restore its old empty-state over the queued fresh read.
      if (revision !== loadRevision || needsReload) return false;
      const u = data.usage;
      usage.replaceChildren();
      for (const [label, bytes, count, unit] of [
        ["Images", u.images, u.image_count, "fichier"],
        ["Documents", u.documents, u.document_count, "extraction"],
        ["Captures d’écran", u.screen, u.screen_count, "capture"],
        ["Mémoire", u.memory, u.fact_count, "fait"],
      ]) {
        const row = node("div", "storage-usage-row");
        let detail = counted(count, unit);
        if (label === "Mémoire" && u.forget_marker_count) detail += ` · ${counted(u.forget_marker_count, "marqueur d’oubli", "marqueurs d’oubli")}`;
        row.append(node("span", "", label), node("strong", "", size(bytes)), node("small", "", detail));
        usage.append(row);
      }
      factsList.replaceChildren();
      factsDescription.textContent = data.orphan_facts.length
        ? "Les faits partagés avec l’équipe sont conservés. Supprimer un fait efface aussi ses anciennes versions privées, pas vos messages."
        : "Aucun fait privé à supprimer.";
      for (const fact of data.orphan_facts) {
        const row = node("div", "storage-fact");
        row.dataset.factId = fact.id;
        const copy = node("div");
        copy.append(node("strong", "", `${fact.bot_name} · Privé${fact.status === "superseded" ? " · Ancienne version" : ""}`), node("p", "", fact.text));
        const remove = button("Supprimer", "secondary-button", () => confirm("Supprimer ce fait privé ?", `« ${fact.text} » — Cette action est définitive. Les messages de vos conversations restent conservés.`, "Confirmer la suppression", async () => {
          await panels.api("/api/storage/facts/delete", { fact_id: fact.id, confirm: true });
          return "Fait privé supprimé.";
        }, { anchor: row, origin: remove, kind: "fact" }));
        remove.disabled = pending;
        row.append(copy, remove);
        factsList.append(row);
      }
      return true;
    } catch (error) {
      if (revision === loadRevision) message(error.message, true);
      return false;
    } finally { loading--; void reloadIfNeeded(); }
  }
  function cleanupResult(result) {
    if (!result.deleted && result.already_deleted && !result.retained) return "Déjà nettoyé ; rien d’autre à supprimer.";
    let text = `${counted(result.deleted, "élément nettoyé", "éléments nettoyés")} · ${size(result.bytes)} ${plural(result.bytes, "libéré", "libérés")}.`;
    if (result.retained) text += ` ${counted(result.retained, "élément conservé", "éléments conservés")} car ${plural(result.retained, "son état a changé", "leur état a changé")}.`;
    if (result.already_deleted) text += ` ${counted(result.already_deleted, "élément déjà nettoyé", "éléments déjà nettoyés")}.`;
    return text;
  }
  panels.register({
    id: "storage",
    onState(state) {
      if (!state.bots) return;
      const signature = JSON.stringify(state.bots.map((b) => [b.id, Boolean(b.deleted), b.name]).sort((a, b) => a[0].localeCompare(b[0])));
      if (signature !== botsSignature) { botsSignature = signature; needsReload = true; }
      void reloadIfNeeded();
    },
    settingsSection(container) {
      section = container;
      section.className = "setting-section storage-section";
      section.append(node("h3", "", "Stockage"), node("p", "", "Données locales de l’application. Seules les pièces jointes sans référence peuvent être nettoyées. Les brouillons et les fichiers utilisés sont conservés. Les pièces déjà envoyées restent conservées, y compris dans les conversations supprimées."));
      usage = node("div", "storage-usage");
      usage.setAttribute("aria-label", "Espace utilisé");
      const explanation = node("p", "", "Documents : taille des extractions conservées, sans copie du fichier d’origine. Mémoire : contenu et provenance, marqueurs d’oubli inclus, hors historique. L’espace SQLite libéré est réutilisé par l’application. Les captures d’écran restent dans leur fil et sont effacées quand ce fil est supprimé. Les imports abandonnés depuis plus de 24 h sont nettoyés au démarrage.");
      const actions = node("div", "storage-actions");
      refresh = button("Actualiser", "secondary-button", async () => { closeConfirmation(); message(""); message("", false, "fact"); busy(true); await load(); busy(false); });
      clean = button("Nettoyer", "secondary-button", async () => {
        if (pending) return;
        busy(true); message(""); closeConfirmation();
        try {
          const preview = await panels.api("/api/storage/preview", {});
          if (!preview.count) { message("Aucune pièce jointe inutilisée à nettoyer."); return; }
          confirm("Confirmer le nettoyage ?", `${counted(preview.count, "élément inutilisé", "éléments inutilisés")} · ${size(preview.bytes)} à libérer. Aucun fichier référencé par un message, un brouillon, une tâche ou une skill ne sera supprimé. L’aperçu expire après 10 minutes.`, "Confirmer le nettoyage", async () => cleanupResult(await panels.api("/api/storage/clean", { token: preview.token, confirm: true })));
        } catch (error) { message(error.message, true); }
        finally { busy(false); }
      });
      actions.append(clean, refresh);
      confirmation = node("div", "storage-confirmation");
      confirmation.hidden = true;
      notice = node("p", "storage-notice");
      notice.setAttribute("role", "status");
      facts = node("div", "storage-facts");
      factsTitle = node("h4", "", "Mémoire des bots supprimés");
      factsTitle.tabIndex = -1;
      factsDescription = node("p");
      factsList = node("div", "storage-facts-list");
      factNotice = node("p", "storage-notice storage-fact-notice");
      factNotice.setAttribute("role", "status");
      facts.append(factsTitle, factsDescription, factsList, factNotice);
      section.append(usage, explanation, actions, confirmation, notice, facts);
      dialog = document.getElementById("settings-dialog");
      new MutationObserver(() => {
        closeConfirmation();
        if (dialog.open) { message(""); message("", false, "fact"); needsReload = true; void reloadIfNeeded(); }
      }).observe(dialog, { attributes: true, attributeFilter: ["open"] });
    },
  });
})();
