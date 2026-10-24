/* Mémoire : le formulaire « Ajouter un souvenir », rangé dans l’en-tête de l’écran Mémoire (memory-space.js).
 * La validation qui compte est celle du moteur : le texte part tel quel et sa réponse s’affiche telle quelle.
 * Le bundle des panneaux est trié par nom : ce fichier passe avant memory-space.js, qui le trouve sur window. */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, className, text) => {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const choice = (value, label, disabled = false) => Object.assign(node("option", "", label), { value, disabled });
  const control = (tag, id, label) => {
    const wrapper = node("div", "memory-add-field"), caption = node("label", "form-label", label), input = node(tag, "text-input");
    caption.setAttribute("for", id);
    input.id = id;
    wrapper.append(caption, input);
    return { wrapper, input };
  };
  let state = null, refresh = null;
  if (!panels.register({ id: "memory-add", onState(next) { state = next; refresh?.(); } })) return;

  // `onAdded` relit la liste des souvenirs et rend la main une fois l’écran à jour.
  function create(onAdded) {
    let busy = false, signature = null;
    const button = node("button", "secondary-button memory-add-open", "Ajouter un souvenir");
    button.type = "button";
    button.setAttribute("aria-expanded", "false");
    const form = node("form", "memory-add");
    form.hidden = true;
    const [note, who, where, which] = [control("textarea", "memory-add-text", "Souvenir"), control("select", "memory-add-bot", "Bot"), control("select", "memory-add-scope", "Portée"), control("select", "memory-add-topic", "Sujet")];
    const [text, bot, scope, topic] = [note, who, where, which].map((item) => item.input);
    text.setAttribute("rows", "4");
    const problem = node("p", "memory-add-error"), cancel = node("button", "secondary-button", "Annuler"), submit = node("button", "primary-button", "Valider");
    problem.setAttribute("role", "alert");
    cancel.type = "button";
    submit.type = "submit";
    const row = node("div", "memory-add-row"), footer = node("div", "memory-add-footer");
    row.append(who.wrapper, where.wrapper, which.wrapper);
    footer.append(cancel, submit);
    form.append(note.wrapper, row, problem, footer);

    // Remplit une liste sans écraser le choix en cours ; s’il n’y a qu’un choix possible, il est pris d’office.
    function fill(select, choices, placeholder) {
      const keep = select.value, usable = choices.filter((item) => !item.disabled);
      select.replaceChildren(...(usable.length === 1 ? [] : [choice("", placeholder)]), ...choices);
      select.value = usable.some((item) => item.value === keep) ? keep : usable.length === 1 ? usable[0].value : "";
    }
    function update() {
      const onTopic = scope.value === "topic";
      which.wrapper.hidden = !onTopic;
      text.readOnly = cancel.disabled = busy;
      submit.disabled = busy || !text.value.trim() || !bot.value || (onTopic && !topic.value);
    }
    refresh = () => {
      const bots = (state?.bots || []).filter((item) => !item.deleted).map((item) => choice(item.id, item.name));
      const topics = (state?.topics?.items || []).map((item) => choice(item.id, item.name));
      const next = JSON.stringify([bots, topics].map((list) => list.map((item) => item.value + item.textContent)));
      if (next !== signature) {
        signature = next;
        fill(bot, bots, "Choisir un bot");
        fill(topic, topics, "Choisir un sujet");
        const kept = scope.value === "topic" && topics.length;
        scope.replaceChildren(choice("private", "Privé"), choice("topic", topics.length ? "Sujet" : "Sujet (aucun sujet pour l’instant)", !topics.length));
        scope.value = kept ? "topic" : "private";
      }
      update();
    };
    // Fermer garde le texte saisi : seul un ajout réussi le vide.
    function close() {
      if (busy) return;
      form.hidden = true;
      button.setAttribute("aria-expanded", "false");
      problem.textContent = "";
      button.focus();
    }
    button.addEventListener("click", () => {
      if (!form.hidden) return close();
      form.hidden = false;
      button.setAttribute("aria-expanded", "true");
      refresh();
      text.focus();
    });
    cancel.addEventListener("click", close);
    form.addEventListener("keydown", (event) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close();
    });
    for (const item of [text, bot, scope, topic]) for (const name of ["input", "change"]) item.addEventListener(name, update);
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      update();
      if (busy || submit.disabled) return;
      const body = { bot_id: bot.value, text: text.value, scope: scope.value };
      if (body.scope === "topic") body.topic_id = topic.value;
      busy = true;
      problem.textContent = "";
      update();
      try {
        await panels.api("/api/memory/add", body);
      } catch (error) {
        busy = false;
        problem.textContent = error?.message || "Le souvenir n’a pas pu être ajouté.";
        update();
        text.focus();
        return;
      }
      busy = false;
      text.value = "";
      close();
      update();
      await onAdded?.();
      panels.notify("Souvenir ajouté.");
    });
    refresh();
    return { button, form };
  }
  window.EzerbotMemoryAdd = { create };
})();
