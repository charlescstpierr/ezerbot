/* Panneau Secrets d’un bot. La valeur va au coffre (trousseau si le
 * backend l’utilise) et ne réapparaît ni ici, ni dans le fil, ni dans un export.
 * « Stockage des secrets » dans Paramètres et l’écran ChatGPT restent distincts.
 */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const sections = new Set();
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
  const field = (name, label, type) => {
    const input = node("input", "text-input");
    input.name = name;
    input.setAttribute("name", name);
    input.type = type;
    input.setAttribute("type", type);
    input.autocomplete = type === "password" ? "new-password" : "off";
    input.spellcheck = false;
    input.setAttribute("aria-label", label);
    const caption = node("label", "form-label", label);
    return { caption, input };
  };
  const pad = (value) => String(value).padStart(2, "0");
  function formatWhen(ms) {
    const date = new Date(ms);
    if (!Number.isFinite(date.getTime()) || date.getTime() <= 0) return "";
    return `${pad(date.getDate())}/${pad(date.getMonth() + 1)}/${date.getFullYear()} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
  }
  function rowsFor(bot) {
    const rows = panels.state?.forms?.bot_secrets?.[bot.id];
    if (!Array.isArray(rows)) return [];
    return rows.filter((row) => row && typeof row.name === "string");
  }
  function render(view) {
    const rows = rowsFor(view.bot).map(({ name, description, created_at, updated_at }) => ({ name, description, created_at, updated_at }));
    view.empty.textContent = rows.length ? "" : "Aucun secret";
    view.empty.hidden = rows.length > 0;
    view.list.replaceChildren();
    for (const row of rows) {
      const item = node("article", "secret-row");
      const title = node("strong", "secret-name", row.name);
      const description = node("p", "secret-description", row.description || "");
      const created = formatWhen(row.created_at);
      const updated = formatWhen(row.updated_at);
      const dates = node("p", "secret-dates", `Créé le ${created} · Mis à jour le ${updated}`);
      const remove = button("Supprimer", () => removeSecret(view, row.name));
      remove.disabled = Boolean(view.bot.deleted);
      item.append(title, description, dates, remove);
      view.list.append(item);
    }
  }
  async function removeSecret(view, name) {
    view.status.textContent = "";
    try {
      await panels.api("/api/secrets/delete", { bot_id: view.bot.id, name });
      await panels.refresh();
    } catch {
      view.status.textContent = "Le secret n’a pas pu être supprimé.";
    }
  }
  function botSection(bot, container) {
    const section = node("section", "secrets-panel");
    section.setAttribute("aria-label", "Secrets du bot");
    section.addEventListener("keydown", (event) => {
      const tag = event.target?.tagName;
      if (event.key === "Enter" && (tag === "input" || tag === "INPUT")) event.preventDefault();
    });
    section.append(node("h3", "", "Secrets"));
    container.append(section);
    if (!bot?.id || bot.deleted) {
      section.append(node("p", "secrets-note", "Enregistrez d’abord ce bot pour gérer ses secrets."));
      return;
    }
    section.append(node("p", "secrets-note", "La valeur part dans le coffre de ce bot, et dans le trousseau lorsqu’il est disponible. Elle n’entre ni dans cette liste, ni dans le fil, ni dans un export."));
    const empty = node("p", "secrets-empty", "Aucun secret");
    const list = node("div", "secrets-list");
    const form = node("div", "secrets-form");
    form.hidden = true;
    const name = field("secret-name", "Nom", "text");
    const description = field("secret-description", "Description", "text");
    const value = field("secret-value", "Valeur", "password");
    const status = node("p", "secrets-status");
    status.setAttribute("role", "status");
    const view = { bot, empty, list, status, save: null };
    const save = button("Enregistrer", () => store(), "primary-button");
    view.save = save;
    const cancel = button("Annuler", () => {
      form.hidden = true;
      value.input.value = "";
      status.textContent = "";
    });
    form.append(name.caption, name.input, description.caption, description.input, value.caption, value.input, save, cancel);
    const add = button("Ajouter un secret", () => {
      form.hidden = false;
      status.textContent = "";
      name.input.focus();
    }, "primary-button");
    section.append(add, empty, list, form, status);
    sections.add(view);
    render(view);
    async function store() {
      const secret = value.input.value;
      value.input.value = "";
      status.textContent = "";
      const trimmedDescription = description.input.value.trim();
      if (!name.input.value.trim() || !trimmedDescription || !secret.trim()) {
        status.textContent = trimmedDescription ? "Le nom, la description et la valeur sont requis." : "La description est requise.";
        return;
      }
      save.disabled = true;
      try {
        await panels.api("/api/secrets/put", {
          bot_id: bot.id,
          name: name.input.value,
          description: description.input.value,
          value: secret,
        });
        name.input.value = "";
        description.input.value = "";
        form.hidden = true;
        await panels.refresh();
      } catch {
        status.textContent = "Le secret n’a pas pu être enregistré.";
      } finally {
        save.disabled = false;
      }
    }
    return () => sections.delete(view);
  }
  panels.register({
    id: "secrets",
    botSection,
    onState() {
      for (const view of sections) render(view);
    },
  });
})();
