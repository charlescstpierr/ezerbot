"use strict";

// Bot templates: the catalogue gallery, a template's detail, one-click creation,
// import by file, link or code, and export from the bot dialog. The engine
// validates every document; imported routines start paused.
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const LINK_PREFIX = "ezerbot://app/v1/bot-template?";
  const LINK_LIMIT = 2048;
  let catalogue = null;
  let loading = null;
  let dialog = null;
  let body = null;
  let filter = { category: "all", query: "" };

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
  const svg = (name) => {
    const el = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    el.setAttribute("class", "icon");
    el.setAttribute("aria-hidden", "true");
    const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
    use.setAttribute("href", `#i-${name}`);
    el.append(use);
    return el;
  };
  const fold = (text) => String(text || "").normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
  const plural = (count, one, many) => `${count} ${count > 1 ? many : one}`;
  const STARTER_KEY = "ezerbot.template-starter";

  // Open the bot's new thread with a starter. A draft the user typed is kept:
  // the starter then goes after it. Only an untouched earlier starter is replaced.
  function useStarter(botId, starter) {
    let last = "";
    try { last = localStorage.getItem(STARTER_KEY) || ""; } catch { /* Appending stays safe. */ }
    if (panels.selectBot?.(botId) === false) return false;
    const draft = panels.draft().trim();
    if (!draft || draft === last) {
      if (panels.selectBot?.(botId, starter) === false) return false;
    } else panels.appendDraft(starter);
    try { localStorage.setItem(STARTER_KEY, starter); } catch { /* Same as above. */ }
    return true;
  }

  async function load() {
    if (catalogue) return catalogue;
    loading ||= panels.api("/api/templates").then((value) => (catalogue = value)).finally(() => { loading = null; });
    return loading;
  }

  function mascotTile(bot, size = "") {
    const tile = node("span", `tpl-mascot ${size}`.trim());
    tile.dataset.botColor = bot.color;
    tile.setAttribute("aria-hidden", "true");
    tile.append(panels.createBotMascot({ id: `modele-${bot.name}`, shape: bot.shape, color: bot.color })());
    return tile;
  }

  function connectorInfo(id) {
    return catalogue?.connectors?.[id] || { name: id, color: "#6b7080" };
  }

  function monogram(id) {
    const info = connectorInfo(id);
    const mark = node("span", "tpl-monogram", info.name.replace(/^Google /, "").charAt(0).toUpperCase());
    mark.style.setProperty("--connector-color", info.color);
    mark.title = info.name;
    mark.setAttribute("aria-hidden", "true");
    return mark;
  }

  function categoryName(id) {
    return catalogue?.categories.find((category) => category.id === id)?.name || "";
  }

  // A template is recommended when one of its connectors is already set up.
  function recommended(template) {
    const servers = (panels.state.mcp?.servers || []).map((server) => fold(server.name));
    return template.connectors.some((id) => {
      const name = fold(connectorInfo(id).name);
      return servers.some((server) => server.includes(name) || name.includes(server));
    });
  }

  function existingBot(templateId) {
    return panels.state.bots.find((bot) => !bot.deleted && bot.template === templateId);
  }

  function ensureDialog() {
    if (dialog) return dialog;
    dialog = node("dialog", "dialog tpl-dialog");
    dialog.setAttribute("aria-labelledby", "tpl-title");
    body = node("div", "tpl-body");
    dialog.append(body);
    // The close event is queued: a reopen in the same task must keep its content.
    dialog.addEventListener("close", () => { if (!dialog.open) body.replaceChildren(); });
    document.body.append(dialog);
    return dialog;
  }

  function header(title, eyebrow, ...actions) {
    const head = node("div", "dialog-header tpl-header");
    const copy = node("div");
    if (title) {
      const h2 = node("h2", "", title); h2.id = "tpl-title";
      copy.append(node("p", "dialog-eyebrow", eyebrow), h2);
    }
    const tools = node("div", "tpl-header-actions");
    const close = button("", () => dialog.close(), "icon-button");
    close.setAttribute("aria-label", "Fermer"); close.append(svg("close"));
    tools.append(...actions, close);
    head.append(copy, tools);
    return head;
  }

  async function open(target = {}) {
    ensureDialog();
    if (!dialog.open) dialog.showModal();
    body.replaceChildren(node("p", "tpl-loading", "Chargement des modèles…"));
    try { await load(); }
    catch (error) {
      body.replaceChildren(header("Modèles de bots", "VOS BOTS"), node("p", "tpl-error", error.message || "Les modèles n’ont pas pu être chargés."));
      return false;
    }
    if (target.templateId) {
      const template = catalogue.templates.find((item) => item.id === target.templateId);
      if (!template) { dialog.close(); return false; }
      renderDetail(template, { templateId: template.id });
    } else if (target.document) renderDetail(target.document, { imported: true });
    else if (target.import) renderImport();
    else renderGallery();
    return true;
  }

  function card(template) {
    const item = node("button", "tpl-card");
    item.type = "button";
    item.dataset.template = template.id;
    item.dataset.botColor = template.bot.color;
    const top = node("span", "tpl-card-top");
    top.append(mascotTile(template.bot));
    if (recommended(template)) top.append(node("span", "tpl-badge", "Recommandé"));
    else if (existingBot(template.id)) top.append(node("span", "tpl-badge muted", "Déjà créé"));
    const copy = node("span", "tpl-card-copy");
    copy.append(node("strong", "", template.bot.name), node("small", "", template.bot.description));
    const meta = node("span", "tpl-card-meta");
    const counts = [plural(template.skills.length, "skill", "skills")];
    if (template.routines.length) counts.push(plural(template.routines.length, "routine", "routines"));
    meta.append(node("span", "", counts.join(" · ")));
    const marks = node("span", "tpl-monograms");
    marks.append(...template.connectors.slice(0, 4).map(monogram));
    meta.append(marks);
    item.append(top, copy, meta);
    item.addEventListener("click", () => renderDetail(template, { templateId: template.id }));
    return item;
  }

  function renderGallery() {
    const importButton = button("", () => renderImport(), "secondary-button tpl-import-button");
    importButton.append(svg("download"), node("span", "", "Importer un bot"));
    const search = node("label", "tpl-search");
    const input = node("input");
    input.type = "search"; input.placeholder = "Rechercher un modèle, une skill, un outil";
    input.setAttribute("aria-label", "Rechercher un modèle"); input.autocomplete = "off";
    input.value = filter.query;
    search.append(svg("search"), input);
    const chips = node("div", "tpl-chips");
    chips.setAttribute("role", "group"); chips.setAttribute("aria-label", "Catégories");
    const results = node("div", "tpl-results");
    const categories = [{ id: "all", name: "Tout" }, ...catalogue.categories];
    for (const category of categories) {
      const chip = button(category.name, () => { filter.category = category.id; paint(); }, "tpl-chip");
      chip.dataset.category = category.id;
      chips.append(chip);
    }
    function paint() {
      for (const chip of chips.children) chip.setAttribute("aria-pressed", String(chip.dataset.category === filter.category));
      const words = fold(filter.query).split(/\s+/).filter(Boolean);
      const matches = catalogue.templates.filter((template) => {
        if (filter.category !== "all" && template.category !== filter.category) return false;
        const haystack = fold([template.bot.name, template.bot.description, categoryName(template.category),
          ...template.skills.map((skill) => skill.name), ...template.connectors.map((id) => connectorInfo(id).name)].join(" "));
        return words.every((word) => haystack.includes(word));
      });
      const sections = [];
      if (filter.category === "all" && !words.length) {
        const picks = matches.filter(recommended);
        if (picks.length) sections.push(["Recommandés pour vous", picks]);
        for (const category of catalogue.categories) {
          const list = matches.filter((template) => template.category === category.id);
          if (list.length) sections.push([category.name, list]);
        }
      } else if (matches.length) sections.push([words.length ? plural(matches.length, "résultat", "résultats") : categoryName(filter.category), matches]);
      results.replaceChildren(...sections.map(([title, list]) => {
        const section = node("section", "tpl-section");
        const grid = node("div", "tpl-grid");
        grid.append(...list.map(card));
        section.append(node("h3", "", title), grid);
        return section;
      }));
      if (!sections.length) results.append(node("p", "tpl-empty", "Aucun modèle ne correspond. Essayez un autre mot, ou créez un bot vide."));
    }
    input.addEventListener("input", () => { filter.query = input.value; paint(); });
    const intro = node("p", "tpl-intro", `${catalogue.templates.length} modèles prêts à l’emploi, avec leurs instructions, skills, routines et connecteurs suggérés.`);
    body.replaceChildren(header("Modèles de bots", "VOS BOTS", importButton), intro, search, chips, results);
    paint();
    if (!matchMedia("(pointer: coarse)").matches) input.focus();
  }

  function section(title, ...content) {
    const box = node("section", "tpl-detail-section");
    box.append(node("h3", "", title), ...content);
    return box;
  }

  function renderDetail(template, { templateId = null, imported = false } = {}) {
    const bot = template.bot;
    const back = button("", () => (imported ? renderImport() : renderGallery()), "tpl-back");
    back.append(svg("chevron"), node("span", "", imported ? "Importer un autre bot" : "Tous les modèles"));
    const hero = node("div", "tpl-hero");
    hero.dataset.botColor = bot.color;
    hero.append(mascotTile(bot, "large"));
    const title = node("div", "tpl-hero-copy");
    const h2 = node("h2", "", bot.name); h2.id = "tpl-title";
    title.append(node("p", "dialog-eyebrow", imported ? "BOT IMPORTÉ" : (categoryName(template.category) || "MODÈLE").toUpperCase()), h2, node("p", "", bot.description || "Aucune présentation."));
    hero.append(title);

    const sections = [];
    const instructions = node("details", "tpl-instructions");
    instructions.append(node("summary", "", "Voir les instructions"), node("p", "", bot.instructions || "Aucune instruction."));
    sections.push(section("Personnalité et mission", instructions));
    if (template.skills.length) {
      const list = node("ul", "tpl-list");
      for (const skill of template.skills) {
        const row = node("li");
        const icon = node("span", "tpl-list-icon"); icon.append(svg("learn"));
        const copy = node("span");
        copy.append(node("strong", "", skill.name), node("small", "", skill.description || "Sans description"));
        row.append(icon, copy); list.append(row);
      }
      sections.push(section(`Skills (${template.skills.length})`, list));
    }
    let activate = null;
    if (template.routines.length) {
      const list = node("ul", "tpl-list");
      for (const routine of template.routines) {
        const row = node("li");
        const icon = node("span", "tpl-list-icon"); icon.append(svg("refresh"));
        const copy = node("span");
        copy.append(node("strong", "", routine.name), node("small", "", routine.schedule_description || "Horaire à vérifier dans les routines du bot"));
        row.append(icon, copy); list.append(row);
      }
      const extra = [];
      if (imported) extra.push(node("p", "tpl-note", "Les routines importées sont créées en pause. Relisez-les dans les routines du bot avant de les activer."));
      else {
        const toggle = node("label", "setting-toggle tpl-toggle");
        const copy = node("span");
        copy.append(node("strong", "", "Activer les routines dès la création"), node("small", "", "Sinon, elles restent en pause jusqu’à ce que vous les activiez."));
        activate = node("input"); activate.type = "checkbox"; activate.setAttribute("role", "switch");
        const track = node("span", "toggle-track"); track.setAttribute("aria-hidden", "true");
        toggle.append(copy, activate, track);
        extra.push(toggle);
      }
      sections.push(section(`Routines (${template.routines.length})`, list, ...extra));
    }
    if (template.connectors.length) {
      const pills = node("div", "tpl-pills");
      for (const id of template.connectors) {
        const pill = node("span", "tpl-pill");
        pill.append(monogram(id), node("span", "", connectorInfo(id).name));
        pills.append(pill);
      }
      sections.push(section("Connecteurs suggérés", pills, node("p", "tpl-note", "Le bot fonctionne sans eux. Ajoutez-les dans Paramètres › Connecteurs pour qu’il agisse directement dans ces outils.")));
    }
    if (template.memories?.length) {
      const list = node("ul", "tpl-plain");
      list.append(...template.memories.slice(0, 8).map((text) => node("li", "", text)));
      if (template.memories.length > 8) list.append(node("li", "tpl-note", `et ${template.memories.length - 8} autres`));
      sections.push(section(`Souvenirs de départ (${template.memories.length})`, list));
    }
    let starters = null;
    if (template.starters?.length) {
      starters = node("div", "tpl-try");
      sections.push(section("Pour commencer", starters, node("p", "tpl-note", "Un clic crée le bot et place cette demande dans la barre d’écriture.")));
    }
    const detail = node("div", "tpl-detail");
    const columns = node("div", "tpl-sections");
    columns.append(...sections);
    detail.append(hero, columns);

    const footer = node("div", "dialog-footer tpl-footer");
    const status = node("p", "tpl-status"); status.setAttribute("role", "status");
    const existing = templateId && existingBot(templateId);
    if (existing) status.textContent = `Vous avez déjà « ${existing.name} », créé depuis ce modèle.`;
    const botId = crypto.randomUUID();
    async function make(starter) {
      if (create.disabled) return;
      create.disabled = true;
      status.textContent = "Création du bot…";
      try {
        const payload = { bot_id: botId, activate_routines: Boolean(activate?.checked) };
        // Schedule descriptions are display-only; the engine reads the schedule itself.
        if (templateId) payload.template_id = templateId;
        else payload.document = { ...template, routines: template.routines.map(({ schedule_description, ...routine }) => routine) };
        const result = await panels.api("/api/templates/create", payload);
        await panels.refresh();
        if (!panels.state.bots?.some((bot) => bot.id === result.bot.id && !bot.deleted)) await panels.refresh();
        dialog.close();
        if (starter) useStarter(result.bot.id, starter);
        else panels.selectBot?.(result.bot.id);
        const routines = template.routines.length ? (result.routines_enabled ? " Ses routines sont actives." : " Ses routines sont en pause.") : "";
        panels.notify(`« ${result.bot.name} » est prêt. Écrivez-lui dès maintenant.${routines}`);
      } catch (error) {
        status.textContent = error.message || "Le bot n’a pas pu être créé.";
        create.disabled = false;
      }
    }
    const create = button(existing ? "En créer un autre" : "Créer ce bot", () => make(), "primary-button");
    for (const text of template.starters || []) starters?.append(button(text, () => make(text), "tpl-try-chip"));
    create.dataset.templateCreate = "";
    const actions = node("div", "tpl-footer-actions");
    if (existing) actions.append(button(`Ouvrir ${existing.name}`, () => { dialog.close(); panels.selectBot?.(existing.id); }));
    actions.append(create);
    footer.append(status, actions);
    body.replaceChildren(header("", "", back), detail, footer);
    body.scrollTop = 0;
    h2.tabIndex = -1; h2.focus();
  }

  function renderImport() {
    const back = button("", () => renderGallery(), "tpl-back");
    back.append(svg("chevron"), node("span", "", "Tous les modèles"));
    const form = node("form", "tpl-import");
    const label = node("label", "form-label", "Lien ou code de partage");
    label.htmlFor = "tpl-import-text";
    const text = node("textarea", "text-input");
    text.id = "tpl-import-text"; text.rows = 4; text.spellcheck = false;
    text.placeholder = "ezerbot://app/v1/bot-template?data=…";
    const file = node("input"); file.type = "file"; file.accept = ".json,application/json"; file.hidden = true;
    const pick = button("", () => file.click(), "secondary-button tpl-file-button");
    pick.append(svg("download"), node("span", "", "Choisir un fichier .ezerbot.json"));
    const status = node("p", "tpl-status"); status.setAttribute("role", "status");
    const read = node("button", "primary-button", "Aperçu du bot"); read.type = "submit";
    const footer = node("div", "dialog-footer tpl-footer");
    const actions = node("div", "tpl-footer-actions"); actions.append(read);
    footer.append(status, actions);
    async function decode(value) {
      if (!value.trim()) { status.textContent = "Collez un lien, un code ou choisissez un fichier."; return; }
      read.disabled = true; status.textContent = "Lecture…";
      try {
        const result = await panels.api("/api/templates/decode", { text: value });
        if (result.template_id) {
          const template = catalogue.templates.find((item) => item.id === result.template_id);
          if (template) return renderDetail(template, { templateId: template.id });
        }
        renderDetail(result.document, { imported: true });
      } catch (error) {
        status.textContent = error.message || "Ce contenu n’a pas pu être lu.";
        read.disabled = false;
      }
    }
    file.addEventListener("change", async () => {
      const chosen = file.files?.[0];
      if (!chosen) return;
      if (chosen.size > 4 * 1024 * 1024) { status.textContent = "Ce fichier dépasse 4 Mo."; return; }
      await decode(await chosen.text());
      file.value = "";
    });
    form.addEventListener("submit", (event) => { event.preventDefault(); void decode(text.value); });
    form.append(
      node("p", "tpl-intro", "Importez un bot partagé par un proche ou exporté d’un autre Mac. Vous verrez son contenu avant de le créer."),
      label, text, node("p", "tpl-or", "ou"), pick, file,
      node("p", "tpl-note", "Un bot partagé ne contient jamais de conversations, de secrets ni de fichiers. Ses routines arrivent en pause."),
      footer,
    );
    const title = node("h2", "tpl-import-title", "Importer un bot"); title.id = "tpl-title";
    form.prepend(title);
    body.replaceChildren(header("", "", back), form);
    text.focus();
  }

  async function copy(text) {
    try { await navigator.clipboard.writeText(text); return true; }
    catch {
      const area = node("textarea", "visually-hidden");
      area.value = text; document.body.append(area); area.select();
      const ok = document.execCommand("copy"); area.remove();
      return ok;
    }
  }

  function slug(name) {
    return fold(name).replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "bot";
  }

  function shareSection(bot, container) {
    container.className = "tpl-share";
    const extras = panels.state.templates?.bots?.[bot.id];
    if (extras?.starters?.length) {
      const list = node("div", "tpl-try");
      for (const starter of extras.starters) {
        list.append(button(starter, () => {
          document.getElementById("bot-dialog")?.close();
          useStarter(bot.id, starter);
        }, "tpl-try-chip"));
      }
      container.append(node("p", "form-label bot-field", "Pour commencer"), list);
    }
    if (extras?.connectors?.length) {
      const pills = node("div", "tpl-pills compact");
      const paint = () => pills.replaceChildren(...extras.connectors.map((id) => {
        const pill = node("span", "tpl-pill");
        pill.append(monogram(id), node("span", "", connectorInfo(id).name));
        return pill;
      }));
      paint();
      if (!catalogue) load().then(paint, () => {});
      container.append(node("p", "form-label bot-field", "Connecteurs suggérés"), pills);
    }
    const box = node("div", "tpl-share-box");
    const copyText = node("div");
    copyText.append(node("strong", "", "Partager ce bot"), node("small", "", "Sa personnalité, ses skills, ses routines et ses connecteurs suggérés. Jamais ses conversations, secrets ni fichiers."));
    const memories = node("label", "tpl-check");
    const include = node("input"); include.type = "checkbox";
    memories.append(include, node("span", "", "Inclure ses souvenirs (sauf les données sensibles)"));
    const exportFile = button("", async () => {
      try {
        const { document: exported } = await panels.api("/api/templates/export", { bot_id: bot.id, include_memories: include.checked });
        const url = URL.createObjectURL(new Blob([JSON.stringify(exported, null, 2)], { type: "application/json" }));
        const anchor = node("a"); anchor.href = url; anchor.download = `${slug(bot.name)}.ezerbot.json`;
        document.body.append(anchor); anchor.click(); anchor.remove();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        panels.notify("Bot exporté. Le fichier s’importe depuis Modèles de bots › Importer un bot.");
      } catch (error) { panels.notify(error.message || "L’export a échoué."); }
    });
    exportFile.append(svg("download"), node("span", "", "Exporter un fichier"));
    const copyLink = button("", async () => {
      try {
        const { link } = await panels.api("/api/templates/export", { bot_id: bot.id, include_memories: include.checked });
        if (!await copy(link)) throw new Error("Le lien n’a pas pu être copié.");
        panels.notify(link.length > LINK_LIMIT
          ? "Lien copié. Il est long : collez-le dans Modèles de bots › Importer un bot."
          : "Lien de partage copié. Il contient le bot, sans aucune conversation.");
      } catch (error) { panels.notify(error.message || "Le lien n’a pas pu être copié."); }
    });
    copyLink.append(svg("copy"), node("span", "", "Copier le lien de partage"));
    const actions = node("div", "tpl-share-actions");
    actions.append(exportFile, copyLink);
    box.append(copyText, memories, actions);
    container.append(box);
  }

  function startSection(container) {
    container.className = "tpl-start";
    const copy = node("div");
    copy.append(node("strong", "", "Partir d’un modèle"), node("small", "", "Tri de boîte, Chercheur, Veille, Dev… prêts en un clic, avec leurs skills et routines."));
    const browse = button("Parcourir les modèles", () => {
      document.getElementById("bot-dialog")?.close();
      void open();
    });
    browse.dataset.openTemplates = "";
    container.append(copy, browse);
  }

  // The welcome bot chips (and the composer bot menu of the new home) are rebuilt
  // on every render: keep one « Créer depuis un modèle » entry after « Nouveau bot ».
  function keepEntry(container, make) {
    const place = () => {
      if (!container.isConnected || container.querySelector("[data-template-entry]") || !container.children.length) return;
      container.append(make());
    };
    new MutationObserver(place).observe(container, { childList: true });
    place();
  }
  function welcomeEntry() {
    const chip = button("", () => void open(), "bot-chip add-bot tpl-entry");
    chip.dataset.templateEntry = "";
    chip.append(svg("bot"), node("span", "", "Créer depuis un modèle"));
    return chip;
  }
  function menuEntry() {
    const item = button("", () => {
      const menu = document.getElementById("bot-menu");
      if (menu) { menu.hidden = true; menu.replaceChildren(); }
      document.getElementById("composer-bot")?.setAttribute("aria-expanded", "false");
      void open();
    }, "bot-menu-action");
    item.dataset.templateEntry = "";
    item.setAttribute("role", "menuitem");
    item.append(svg("bot"), node("span", "", "Créer depuis un modèle"));
    return item;
  }
  const chipsRow = document.getElementById("bot-chips");
  if (chipsRow) keepEntry(chipsRow, welcomeEntry);
  const botMenu = document.getElementById("bot-menu");
  if (botMenu) keepEntry(botMenu, menuEntry);

  window.EzerbotTemplates = { open };
  panels.register({
    id: "templates",
    settingsSection(container) {
      container.className = "setting-section tpl-settings";
      const heading = node("div", "setting-heading");
      const copy = node("div");
      copy.append(node("h3", "", "Modèles de bots"), node("p", "", "Créez un bot prêt à l’emploi, ou importez un bot partagé."));
      const actions = node("div", "tpl-settings-actions");
      const browse = button("", () => { document.getElementById("settings-dialog")?.close(); void open(); });
      browse.append(svg("bot"), node("span", "", "Parcourir"));
      const importer = button("", () => { document.getElementById("settings-dialog")?.close(); void open({ import: true }); });
      importer.append(svg("download"), node("span", "", "Importer"));
      actions.append(browse, importer);
      heading.append(copy, actions);
      container.append(heading);
    },
    botSection(bot, container) {
      if (!bot) startSection(container);
      else if (!bot.deleted) shareSection(bot, container);
    },
    async openLink(destination) {
      if (destination.id) return open({ templateId: destination.id });
      try {
        await load();
        const { document: shared } = await panels.api("/api/templates/decode", { text: `${LINK_PREFIX}data=${destination.data}` });
        return open({ document: shared });
      } catch { return false; }
    },
  });
})();
