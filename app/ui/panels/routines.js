/* REA reports/cron-detail.md CR01, CR08–13, CR16–19: per-bot routines,
 * inline prompts, five-field cron (DOM OR DOW), @every phase, explicit timezone,
 * manual cooldown and conversational model confirmation. The editor, page-session
 * nonce retention and missed-range history are local reconstruction, not server REA.
 * No approval is accepted here: pending model confirmations open the main chat.
 */
(() => {
  "use strict";
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, cls, text) => {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text !== undefined) el.textContent = text;
    return el;
  };
  const button = (text, action, cls = "secondary-button") => {
    const el = node("button", cls, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  };
  const date = (at, zone) => {
    try { return new Intl.DateTimeFormat("fr", { dateStyle: "medium", timeStyle: "medium", timeZone: zone }).format(new Date(at)); }
    catch { return "Date indisponible"; }
  };
  const localZone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  const aliases = { "@hourly": "0 * * * *", "@daily": "0 0 * * *", "@midnight": "0 0 * * *", "@weekly": "0 0 * * 0", "@monthly": "0 0 1 * *", "@yearly": "0 0 1 1 *", "@annually": "0 0 1 1 *" };
  function validZone(zone) {
    try { new Intl.DateTimeFormat("fr", { timeZone: zone }).format(); }
    catch { throw new Error(`Fuseau IANA inconnu : ${zone || "(vide)"}.`); }
    return zone;
  }
  function duration(text) {
    // Match the scheduler's exact decimal sum, including .5h and fractional
    // millisecond components which become integral only after addition.
    let rest = text, total = 0n, scale = 1n;
    const max = (1n << 128n) - 1n;
    while (rest) {
      const part = /^(\d+(?:\.\d+)?|\.\d+)(ms|s|m|h|d)/.exec(rest);
      if (!part) throw new Error("Durée attendue : 1h30m, 10m ou .5h (unités minuscules).");
      const [whole, decimal = ""] = part[1].split("."), fraction = decimal.replace(/0+$/, "");
      const digits = `${whole}${fraction}`.replace(/^0+/, "") || "0";
      if (fraction.length > 38 || digits.length > 39) throw new Error("La précision ou la durée dépasse la limite du moteur.");
      const componentScale = 10n ** BigInt(fraction.length), nextScale = scale > componentScale ? scale : componentScale;
      const coefficient = BigInt(digits), multiplier = { ms: 1n, s: 1000n, m: 60000n, h: 3600000n, d: 86400000n }[part[2]];
      total = total * (nextScale / scale) + coefficient * multiplier * (nextScale / componentScale);
      if (coefficient > max || total > max) throw new Error("La précision ou la durée dépasse la limite du moteur.");
      scale = nextScale; rest = rest.slice(part[0].length);
    }
    if (total % scale !== 0n || total / scale > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error("La durée doit être un nombre entier de millisecondes représentable.");
    return Number(total / scale);
  }
  function cadence(ms) {
    for (const [unit, divisor] of [["jour", 86400000], ["heure", 3600000], ["minute", 60000], ["seconde", 1000]]) {
      if (ms && ms % divisor === 0) return `${ms / divisor} ${unit}${ms / divisor === 1 ? "" : "s"}`;
    }
    return `${ms} millisecondes`;
  }
  // This is a description/early check only, never a client-side next-run oracle.
  // In particular, never describe 1-7 DOM + Monday as "first Monday" (CR09).
  function schedule(text, zone) {
    text = text.trim().replace(/\s+/g, " ");
    const prefix = /^(?:CRON_TZ|TZ)=(\S+) (.+)$/.exec(text);
    if (prefix) { zone = prefix[1]; text = prefix[2]; }
    validZone(zone);
    if (/^@every /i.test(text)) {
      if (prefix) throw new Error("@every n’accepte pas de préfixe TZ ; utilisez le champ Fuseau.");
      const parts = text.slice(7).split("/");
      if (parts.length > 2 || parts.some((part) => !part)) throw new Error("Intervalle attendu : @every 1h30m ou @every 1h/10m.");
      const interval = duration(parts[0]), phase = parts.length === 2 ? duration(parts[1]) : null;
      if (interval < 60000) throw new Error("L’intervalle d’exécution minimum est d’une minute.");
      if (phase !== null && phase >= interval) throw new Error("La phase doit être inférieure à l’intervalle.");
      const intervalText = cadence(interval);
      const repeat = intervalText.startsWith("1 ") ? `Chaque ${intervalText.slice(2)}` : `${interval % 86400000 === 0 ? "Tous les" : "Toutes les"} ${intervalText}`;
      return { trigger: { kind: "every", interval_ms: interval, phase_ms: phase }, zone, text: `${repeat}, ${phase === null ? "à partir de l’activation" : `décalage de ${cadence(phase)} depuis l’époque UTC`}` };
    }
    text = aliases[text.toLowerCase()] || text;
    const fields = text.split(" "), bounds = [[0, 59], [0, 23], [1, 31], [1, 12], [0, 7]];
    if (fields.length !== 5) throw new Error("Cron attend cinq champs : minute, heure, jour du mois, mois, jour de semaine (0 ou 7 = dimanche).");
    fields.forEach((field, i) => {
      for (const item of field.split(",")) {
        const match = /^(\*|\d+(?:-\d+)?)(?:\/(\d+))?$/.exec(item);
        if (!match || (match[2] && Number(match[2]) === 0)) throw new Error("Utilisez des nombres, *, listes, plages ou pas (*/5). Ni secondes, MON/JAN, L, W ou #.");
        if (match[1] !== "*") {
          const range = match[1].split("-").map(Number);
          if (range.some((v) => v < bounds[i][0] || v > bounds[i][1]) || (range.length === 2 && range[0] > range[1])) throw new Error("Un champ cron est hors limites.");
        }
      }
    });
    // Mirror scheduler::cron::describe_fr, including expansion of lists/steps
    // and DOM OR DOW. This remains a description, never a next-run oracle.
    const values = (field, min, max, sunday = false) => {
      const result = new Set();
      for (const item of field.split(",")) {
        const [range, stepText] = item.split("/"), step = Number(stepText || 1);
        const [from, to] = range === "*" ? [min, max] : range.includes("-") ? range.split("-").map(Number) : [Number(range), stepText ? max : Number(range)];
        for (let n = from; n <= to; n += step) result.add(sunday && n === 7 ? 0 : n);
      }
      return [...result].sort((a, b) => a - b);
    };
    const join = (items) => items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} et ${items.at(-1)}`;
    const [minute, hour, dom, month, dow] = fields;
    const minutes = values(minute, 0, 59), hours = values(hour, 0, 23), weekdays = values(dow, 0, 7, true);
    const time = minutes.length === 1 && hours.length === 1 ? `à ${String(hours[0]).padStart(2, "0")}:${String(minutes[0]).padStart(2, "0")}` : minutes.length === 60 && hours.length === 24 ? "à chaque minute" : hours.length === 24 ? `à chaque heure, aux minutes ${minutes.join(", ")}` : `aux heures ${hours.join(", ")} et aux minutes ${minutes.join(", ")}`;
    const dayNames = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"];
    const week = weekdays.join(",") === "1,2,3,4,5" ? "du lundi au vendredi" : join(weekdays.map((n) => `le ${dayNames[n]}`));
    const monthDays = `${join(values(dom, 1, 31).map((n) => `le ${n === 1 ? "1er" : n}`))} du mois`;
    const days = dom !== "*" && dow !== "*" ? `${monthDays} OU ${week}` : dom !== "*" ? monthDays : dow !== "*" ? week : "tous les jours";
    const monthNames = ["", "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    const months = month === "*" ? "" : `, en ${join(values(month, 1, 12).map((n) => monthNames[n]))}`;
    return { trigger: { kind: "cron", expression: text, time_zone: null }, zone, text: `${days[0].toUpperCase()}${days.slice(1)} ${time}${months}` };
  }
  function describe(r) {
    if (r.schedule_description) return r.schedule_description;
    try {
      if (r.trigger.kind === "once") return `Une fois : ${date(r.trigger.at, r.time_zone)} · ${r.time_zone}`;
      const t = r.trigger;
      const expression = t.kind === "cron" ? t.expression : `@every ${t.interval_ms}ms${t.phase_ms === null || t.phase_ms === undefined ? "" : `/${t.phase_ms}ms`}`;
      const parsed = schedule(expression, t.time_zone || r.time_zone);
      return `${parsed.text} (${parsed.zone})`;
    } catch { return "Horaire illisible — consultez ou corrigez cette routine."; }
  }

  let dialog, botSelect, content, status, toolbar, view = "list", current = null, returnToSettings = false;
  let settingsScrollTop = 0, settingsFocus = null;
  let busy = false, revision = 0, signature = "", historyAt = 0, historyLoading = false, reloading = false, historySignature = "", fieldId = 0;
  let historyLimit = 200, rangeLimit = 200;
  // Only compact running/nonce summaries survive a history request. The server
  // currently has no pagination; full responses are transient, not cached.
  const launches = new Map(), histories = new Map(), historyReads = new Map();
  const liveBots = () => (panels.state.bots || []).filter((b) => !b.deleted);
  const botId = () => botSelect.value;
  const items = () => (panels.state.routines?.items || []).filter((item) => item.routine.bot_id === botId()).sort((a, b) => Number(b.routine.enabled) - Number(a.routine.enabled) || a.routine.name.localeCompare(b.routine.name, "fr") || a.routine.routine_id.localeCompare(b.routine.routine_id));
  const tracked = (r) => launches.has(r.routine_id) || histories.get(r.routine_id)?.running;
  const wantsPolling = () => Boolean(dialog?.open) && (view === "history" || items().some((i) => tracked(i.routine)));
  // The host only schedules its next poll at the end of refresh(). Changes
  // caused by a panel-local history read must explicitly rearm that scheduler.
  function kick() { if (wantsPolling()) void panels.refresh().catch(() => {}); }
  const idBody = (r) => ({ bot_id: r.bot_id, routine_id: r.routine_id });
  const active = (token) => dialog.open && token === revision;
  function message(text, error = false) { status.textContent = text; status.classList.toggle("routines-error", error); }
  function lock() {
    botSelect.disabled = busy || view !== "list";
    toolbar.querySelectorAll("button").forEach((el) => { el.disabled = busy; });
    content.querySelectorAll("button, input, textarea, select").forEach((el) => { el.disabled = busy || el.dataset.readOnly === "true"; });
    dialog.setAttribute("aria-busy", String(busy));
  }
  function openThread(id) {
    if (!panels.openConversation(id)) { message("Le fil est indisponible. Actualisez puis réessayez.", true); return; }
    returnToSettings = false; // Navigating to a thread must not reopen Settings.
    dialog.close();
    for (const name of ["bot-dialog", "settings-dialog"]) document.getElementById(name)?.close();
  }
  function build() {
    if (dialog) return;
    dialog = node("dialog", "dialog routines-dialog");
    dialog.setAttribute("aria-labelledby", "routines-title");
    const header = node("div", "dialog-header"), title = node("h2", "", "Routines"); title.id = "routines-title";
    header.append(title, button("Fermer", () => dialog.close()));
    const label = node("label", "form-label", "Bot");
    botSelect = node("select", "text-input"); botSelect.name = "routines-bot"; botSelect.setAttribute("aria-label", "Bot des routines"); label.append(botSelect);
    botSelect.addEventListener("change", () => { revision++; current = null; signature = ""; message(""); renderList(); });
    toolbar = node("div", "routines-actions");
    toolbar.append(button("Nouvelle routine", () => edit(null), "primary-button"), button("Actualiser", reload));
    status = node("p", "routines-note"); status.setAttribute("role", "status");
    content = node("div", "routines-content");
    dialog.append(header, label, node("p", "routines-note", "Les routines s’exécutent quand le moteur local est ouvert. Au retour : un seul rattrapage dans les 60 dernières minutes ; les autres créneaux sont marqués manqués. Une pause ne se rattrape pas."), toolbar, status, content);
    dialog.addEventListener("close", () => {
      if (dialog.open) return;
      revision++; current = null; content.replaceChildren();
      const restore = returnToSettings; returnToSettings = false;
      const settings = document.getElementById("settings-dialog");
      if (restore && settings && !settings.open) {
        settings.showModal();
        if (settingsFocus?.isConnected) settingsFocus.focus({ preventScroll: true });
        settings.scrollTop = settingsScrollTop;
      }
      settingsFocus = null;
    });
    (document.getElementById("panels") || document.body).append(dialog);
  }
  function open(bot) {
    build();
    if (busy) { panels.notify("Une action de routine est encore en cours."); return; }
    botSelect.replaceChildren(...liveBots().map((b) => { const option = node("option", "", b.name); option.value = b.id; return option; }));
    botSelect.value = liveBots().some((b) => b.id === bot) ? bot : liveBots()[0]?.id || "";
    view = "list"; current = null; signature = ""; revision++;
    // Replace Settings, but never close the bot editor: its unsaved profile and
    // memory drafts must survive. The bot-entry layering predates D17.
    const settings = document.getElementById("settings-dialog");
    returnToSettings = Boolean(settings?.open && !document.getElementById("bot-dialog")?.open);
    settingsScrollTop = settings?.scrollTop || 0;
    settingsFocus = returnToSettings && settings.contains(document.activeElement) ? document.activeElement : null;
    settings?.close();
    message(""); dialog.showModal(); renderList();
    void reload();
  }
  async function reload() {
    if (busy || reloading) return;
    const token = revision; reloading = true;
    try {
      await panels.refresh();
      // Explicit refresh must repair cached running states too, not only /state.
      for (const { routine: r } of items()) {
        if (!active(token) || busy) break;
        if (histories.has(r.routine_id) || launches.has(r.routine_id)) {
          try { await readHistory(r); settleLaunch(r); }
          catch (error) { if (active(token)) message(`Historique à actualiser : ${error.message}`, true); }
        }
      }
      if (active(token)) { signature = ""; renderList(); if (view === "history") await showHistory(current); }
    } catch (error) { if (active(token)) message(`Actualisation impossible : ${error.message}`, true); }
    finally { reloading = false; if (active(token)) kick(); }
  }
  function renderList() {
    if (!dialog?.open || view !== "list") return;
    const list = items();
    const next = JSON.stringify([list, busy, [...launches].map(([id, job]) => [id, job, Date.now() < job.until]), [...histories]]);
    if (signature === next) return;
    signature = next; toolbar.hidden = false; content.replaceChildren();
    if (panels.state.routines?.error) content.append(node("p", "routines-error", panels.state.routines.error));
    if (!list.length) content.append(node("p", "routines-note", "Aucune routine pour ce bot."));
    for (const item of list) {
      const r = item.routine, row = node("article", "routines-row"); row.dataset.routineId = r.routine_id;
      row.append(node("h3", "", r.name), node("p", "routines-state", r.enabled ? "Active" : "En pause"), node("p", "routines-note", describe(r)));
      if (r.origin === "model" || r.model_edited_at || r.untrusted) row.append(node("p", "routines-warning", "Proposée ou modifiée par un bot · texte non vérifié."));
      if (item.error) row.append(node("p", "routines-error", `Dernier diagnostic : ${item.error}`));
      const actions = node("div", "routines-actions");
      actions.append(button("Modifier", () => edit(r)), button(r.enabled ? "Mettre en pause" : "Reprendre…", () => r.enabled ? mutate("/api/routines/pause", idBody(r), "Routine mise en pause.") : reviewResume(r)), button("Historique", () => showHistory(r)), button("Ouvrir le fil", () => openThread(r.conversation_id)));
      const job = launches.get(r.routine_id), running = histories.get(r.routine_id)?.running;
      const run = button(job?.unknown ? "Vérifier / réessayer le lancement" : "Lancer maintenant", () => runNow(r));
      run.disabled = busy || Boolean(running) || Boolean(job && Date.now() < job.until);
      actions.append(run);
      if (running) row.append(node("p", "routines-note", "Un lancement est en attente ou en cours. Les approbations éventuelles se traitent dans le chat principal."));
      if (item.confirmation_conversation_id) actions.append(button("Confirmation dans le chat principal", () => openThread(item.confirmation_conversation_id)));
      row.append(actions); content.append(row);
    }
    // Do not override the running/cooldown disabled state of the run button.
    botSelect.disabled = busy; toolbar.querySelectorAll("button").forEach((el) => { el.disabled = busy; });
    if (busy) content.querySelectorAll("button").forEach((el) => { el.disabled = true; });
  }
  async function fetchRoutine(r) {
    const data = await panels.api(`/api/routines?bot_id=${encodeURIComponent(r.bot_id)}`);
    const full = data.routines.find((item) => item.routine_id === r.routine_id);
    if (!full) throw new Error("Cette routine a été supprimée. Actualisez la liste.");
    return full;
  }
  function back() { if (busy) return; revision++; view = "list"; current = null; signature = ""; message(""); renderList(); }
  async function edit(r, resume = false) {
    if (busy) return;
    const token = ++revision; view = "edit"; current = r; toolbar.hidden = true; content.replaceChildren(); botSelect.disabled = true;
    content.append(button("Retour à la liste", back)); message(r ? "Chargement du contenu complet…" : "");
    try {
      const full = r ? await fetchRoutine(r) : null;
      let pinned = null;
      if (resume && full.body.kind === "skill_invocation") {
        const data = await panels.api(`/api/skills/versions?bot_id=${encodeURIComponent(full.bot_id)}&skill_id=${encodeURIComponent(full.body.skill_id)}`);
        pinned = data.versions.find((skill) => skill.version === full.body.version && !skill.deleted);
        if (!pinned) throw new Error("La version épinglée de cette skill est indisponible. Modifiez la routine avant de l’activer.");
      }
      if (!active(token)) return;
      message(""); editor(full, resume, pinned);
    } catch (error) { if (active(token)) message(error.message, true); }
  }
  function reviewResume(r) { void edit(r, true); }
  function editor(r, resume, pinned) {
    content.replaceChildren(button("Retour à la liste", back));
    const form = node("form", "routines-editor"), fields = node("fieldset", "routines-fields");
    function field(label, tag = "input") { const wrap = node("label", "form-label", label), input = node(tag, "text-input"); input.id = input.name = `routine-field-${++fieldId}`; wrap.append(input); fields.append(wrap); return input; }
    const name = field("Nom"); name.value = r?.name || ""; name.maxLength = 80;
    const kind = field("Contenu", "select");
    for (const [value, label] of [["inline_prompt", "Prompt libre"], ["skill_invocation", "Skill · version épinglée"]]) { const opt = node("option", "", label); opt.value = value; kind.append(opt); }
    kind.value = r?.body.kind || "inline_prompt";
    const prompt = field("Instructions de la routine", "textarea"); prompt.rows = 6; prompt.value = r?.body.prompt || "";
    const skill = field("Skill de ce bot", "select");
    const availableSkills = panels.state.skills?.by_bot?.[botId()] || [];
    skill.append(node("option", "", "Choisir une skill")); skill.firstChild.value = "";
    for (const item of availableSkills) {
      const description = [...(item.description || "")];
      const label = item.name + (item.name_truncated ? "…" : "") + (description.length ? ` — ${description.slice(0, 60).join("")}${description.length > 60 || item.description_truncated ? "…" : ""}` : "");
      const option = node("option", "", label); option.value = item.skill_id; skill.append(option);
    }
    if (r?.body.skill_id && !availableSkills.some((item) => item.skill_id === r.body.skill_id)) {
      const option = node("option", "", pinned?.content.name || "Skill épinglée indisponible"); option.value = r.body.skill_id; skill.append(option);
    }
    skill.value = r?.body.skill_id || "";
    if (!availableSkills.length) { skill.disabled = true; skill.dataset.readOnly = "true"; }
    const version = field("Version épinglée de la skill"); version.type = "number"; version.min = "1"; version.step = "1"; version.value = String(r?.body.version || 1);
    const skillHint = node("p", "routines-note", availableSkills.length ? "La version exacte est conservée, même si la skill évolue. Consultez la section Skills pour son contenu et ses versions." : "Aucune skill pour ce bot : créez-en une dans Modifier le bot › Skills."); fields.append(skillHint);
    if (pinned) fields.append(node("h4", "", `Contenu intégral de ${pinned.content.name} · version ${pinned.version}`), node("pre", "routines-skill-preview", pinned.content.body));
    const mode = field("Déclencheur", "select");
    for (const [value, label] of [["schedule", "Cron / intervalle"], ["once", "Une seule fois"]]) { const opt = node("option", "", label); opt.value = value; mode.append(opt); }
    mode.value = r?.trigger.kind === "once" ? "once" : "schedule";
    const expression = field("Cron (5 champs), alias ou @every");
    expression.value = r?.trigger.kind === "cron" ? r.trigger.expression : r?.trigger.kind === "every" ? `@every ${r.trigger.interval_ms}ms${r.trigger.phase_ms === null || r.trigger.phase_ms === undefined ? "" : `/${r.trigger.phase_ms}ms`}` : "0 9 * * *";
    const once = field("Date et heure UTC"); once.type = "datetime-local"; once.step = "0.001";
    once.value = r?.trigger.kind === "once" ? new Date(r.trigger.at).toISOString().slice(0, -1) : "";
    const zone = field("Fuseau IANA"); zone.value = r?.trigger.time_zone || r?.time_zone || localZone; zone.placeholder = "America/Montreal";
    fields.append(node("p", "routines-note", "Exemples : 0 9 * * 1-5, @daily, @every 1h30m, @every 1h/10m. Un préfixe TZ= prime sur le champ Fuseau. Pour une seule fois, saisissez l’UTC ; l’aperçu le convertit dans le fuseau choisi. Le serveur valide l’horaire ; cet aperçu ne prédit pas le prochain départ."));
    const preview = node("p", "routines-preview"); preview.setAttribute("role", "status"); fields.append(preview);
    const enabled = r ? null : field("Activer à la création");
    if (enabled) { enabled.type = "checkbox"; enabled.checked = true; enabled.className = ""; enabled.parentElement.classList.add("routines-enabled"); }
    function parsed() {
      if (mode.value === "once") {
        const at = Date.parse(`${once.value}Z`);
        if (!once.value || !Number.isFinite(at)) throw new Error("Choisissez une date UTC valide.");
        if (at <= Date.now()) throw new Error("Choisissez un instant futur (saisie en UTC, indépendamment du fuseau d’aperçu).");
        validZone(zone.value.trim());
        return { trigger: { kind: "once", at }, zone: zone.value.trim(), text: `Une fois : ${date(at, zone.value.trim())}` };
      }
      return schedule(expression.value, zone.value.trim());
    }
    function update() {
      prompt.parentElement.hidden = kind.value !== "inline_prompt";
      skill.parentElement.hidden = version.parentElement.hidden = skillHint.hidden = kind.value !== "skill_invocation";
      expression.parentElement.hidden = mode.value !== "schedule"; once.parentElement.hidden = mode.value !== "once";
      try { const p = parsed(); preview.textContent = `${p.text} (${p.zone})`; preview.classList.remove("routines-error"); }
      catch (error) { preview.textContent = error.message; preview.classList.add("routines-error"); }
    }
    const baseline = () => ({ name: name.value, kind: kind.value, prompt: prompt.value, skill: skill.value, version: version.value, mode: mode.value, expression: expression.value, once: once.value, zone: zone.value });
    const initial = baseline();
    form.addEventListener("input", update); form.addEventListener("change", update); update();
    const actions = node("div", "routines-actions");
    if (!resume) {
      const save = node("button", "primary-button", r ? "Enregistrer" : "Créer la routine"); save.type = "submit"; actions.append(save);
      form.addEventListener("submit", (event) => {
        event.preventDefault(); if (busy) return;
        try {
          const values = baseline(), p = parsed(), payload = { bot_id: botId() };
          if (!name.value.trim() || [...name.value.trim()].length > 80) throw new Error("Le nom doit contenir entre 1 et 80 caractères.");
          if (!r || values.name !== initial.name) payload.name = name.value;
          if (!r || ["kind", "prompt", "skill", "version"].some((key) => values[key] !== initial[key])) {
            if (kind.value === "inline_prompt") {
              if (!prompt.value.trim()) throw new Error("Ajoutez des instructions. La requête complète est limitée à 64 Kio, encodage JSON compris.");
              payload.body = { kind: "inline_prompt", prompt: prompt.value };
            } else {
              if (!skill.value.trim() || !Number.isSafeInteger(Number(version.value)) || Number(version.value) < 1) throw new Error("Choisissez une skill et une version entière positive.");
              payload.body = { kind: "skill_invocation", skill_id: skill.value.trim(), version: Number(version.value) };
            }
          }
          if (!r || ["mode", "expression", "once", "zone"].some((key) => values[key] !== initial[key])) { payload.trigger = p.trigger; payload.time_zone = p.zone; }
          if (r) Object.assign(payload, { routine_id: r.routine_id, modified_at: r.modified_at, schedule_version: r.schedule_version });
          else payload.enabled = enabled.checked;
          if (r && Object.keys(payload).length === 4) { back(); return; }
          if (new TextEncoder().encode(JSON.stringify(payload)).length > 65536) throw new Error("La requête complète dépasse 64 Kio (encodage JSON compris). Raccourcissez le prompt ou utilisez une skill épinglée.");
          void mutate(r ? "/api/routines/update" : "/api/routines", payload, r ? "Routine enregistrée." : payload.enabled ? "Routine créée et activée." : "Routine créée en pause. Relisez son contenu avant de la reprendre.");
        } catch (error) { message(error.message, true); }
      });
      if (r) actions.append(button("Supprimer…", () => {
        if (fields.querySelector(".routines-delete")) return;
        const confirm = node("div", "routines-delete");
        confirm.append(node("p", "routines-warning", "Supprimer cette routine ? Les prochains départs seront supprimés, mais un run déjà lancé n’est pas interrompu."), button("Confirmer la suppression", () => mutate("/api/routines/delete", idBody(r), "Routine supprimée."), "danger-button"), button("Garder la routine", () => confirm.remove())); fields.append(confirm);
      }, "danger-link"));
    } else {
      fields.querySelectorAll("input, textarea, select").forEach((el) => { el.readOnly = true; if (el.tagName === "SELECT") { el.dataset.readOnly = "true"; el.disabled = true; } });
      fields.append(node("p", "routines-warning", "Relisez tout le contenu avant d’activer. Une routine proposée par un bot se confirme normalement dans le chat principal. Ce bouton est une activation explicite par vous, sans approbation séparée."));
      actions.append(button("Activer cette routine", () => mutate("/api/routines/resume", idBody(r), "Routine activée.", r), "primary-button"));
    }
    actions.append(button("Annuler", back)); fields.append(actions); form.append(fields); content.append(form);
    if (!resume) name.focus();
  }
  async function mutate(path, payload, success, reviewed) {
    if (busy) return;
    const token = revision; busy = true; lock(); message("Enregistrement…");
    try {
      if (reviewed) {
        const latest = await fetchRoutine(reviewed);
        if (latest.modified_at !== reviewed.modified_at || latest.schedule_version !== reviewed.schedule_version) throw new Error("La routine a changé depuis votre lecture. Revenez à la liste et relisez-la avant de l’activer.");
      }
      await panels.api(path, payload);
      if (active(token)) { view = "list"; current = null; signature = ""; message(success); }
      try { await panels.refresh(); } catch { if (active(token)) message(`${success} Liste à actualiser.`, true); }
    } catch (error) {
      if (active(token)) {
        const conflict = /Stale routine edit|changé depuis votre lecture|Routine changed|409/i.test(error.message);
        const uncertain = /connexion.*interrompue|réponse illisible/i.test(error.message);
        const reason = /schedule never fires|Schedule has no future slot/i.test(error.message) ? "L’horaire n’a plus de créneau futur. Choisissez un instant futur." : error.message;
        const advice = conflict ? "Revenez à la liste, actualisez puis rouvrez l’éditeur." : uncertain ? "Vérifiez la liste avant de réessayer." : "Corrigez la saisie avant de réessayer.";
        message(`${conflict ? "Conflit d’édition" : uncertain ? "Action non confirmée" : "Action refusée par le moteur"} : ${reason} Vos saisies sont conservées. ${advice}`, true);
      }
    } finally {
      busy = false;
      if (active(token)) { if (view === "list") { signature = ""; renderList(); } else lock(); }
    }
  }
  const pending = (o) => ["pending", "running"].includes(o.status.status);
  async function readHistory(r) {
    const id = r.routine_id, request = (historyReads.get(id) || 0) + 1;
    historyReads.set(id, request);
    const h = await panels.api(`/api/routines/history?bot_id=${encodeURIComponent(r.bot_id)}&routine_id=${encodeURIComponent(id)}`);
    if (!Array.isArray(h.occurrences) || !Array.isArray(h.missed_ranges)) throw new Error("L’historique du moteur est illisible.");
    const job = launches.get(id), own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce);
    if (historyReads.get(id) === request) histories.set(id, { running: h.occurrences.some(pending), total: h.occurrences.length, own: own ? { nonce: own.manual_nonce, status: own.status.status } : null });
    return h;
  }
  function settleLaunch(r) {
    const job = launches.get(r.routine_id), own = histories.get(r.routine_id)?.own;
    if (job && own?.nonce === job.nonce && !job.unknown && Date.now() >= job.until && !["pending", "running"].includes(own.status)) launches.delete(r.routine_id);
  }
  async function showHistory(r, more = "") {
    if (busy) return;
    if (!more) { historyLimit = 200; rangeLimit = 200; }
    else if (more === "occurrences") historyLimit += 200; else rangeLimit += 200;
    const token = ++revision; view = "history"; current = r; toolbar.hidden = true; botSelect.disabled = true; historySignature = "";
    content.replaceChildren(button("Retour à la liste", back), button("Actualiser l’historique", () => showHistory(r))); message("Chargement de l’historique…");
    try { const h = await readHistory(r); settleLaunch(r); if (active(token)) { message(""); renderHistory(r, h); } }
    catch (error) { if (active(token)) message(`Historique indisponible : ${error.message}`, true); }
    finally { if (active(token)) kick(); }
  }
  function renderHistory(r, h) {
    const occurrences = h.occurrences.slice().sort((a, b) => b.scheduled_at_utc - a.scheduled_at_utc || b.occurrence_id.localeCompare(a.occurrence_id)).slice(0, historyLimit);
    const visibleRanges = h.missed_ranges.slice().sort((a, b) => b.from_inclusive - a.from_inclusive).slice(0, rangeLimit);
    const next = JSON.stringify([r.routine_id, h.occurrences.length, h.missed_ranges.length, occurrences, visibleRanges]);
    if (historySignature === next) return;
    historySignature = next;
    content.replaceChildren(button("Retour à la liste", back), button("Actualiser l’historique", () => showHistory(r)), node("h3", "", r.name));
    // Bound DOM work even when the unpaginated server returns years of history.
    const ranges = node("div", "routines-ranges"); ranges.append(node("h4", "", "Plages d’occurrences manquées"));
    if (!h.missed_ranges.length) ranges.append(node("p", "routines-note", "Aucune plage manquée."));
    for (const range of visibleRanges) ranges.append(node("p", "routines-warning", `Manquées du ${date(range.from_inclusive, range.time_zone)} inclus au ${date(range.until_exclusive, range.time_zone)} exclu · ${range.time_zone} · version ${range.schedule_version} · ${describe({ trigger: range.trigger, time_zone: range.time_zone })}`));
    if (visibleRanges.length < h.missed_ranges.length) ranges.append(node("p", "routines-note", `${visibleRanges.length} plages sur ${h.missed_ranges.length}`), button("Afficher plus de plages", () => showHistory(r, "ranges")));
    content.append(ranges, node("h4", "", `Occurrences : ${occurrences.length} sur ${h.occurrences.length} (plus récentes en premier)`));
    if (!h.occurrences.length) content.append(node("p", "routines-note", "Aucune occurrence."));
    for (const o of occurrences) {
      const row = node("article", "routines-occurrence"); row.dataset.status = o.status.status;
      const labels = { pending: "En attente", running: "En cours / en attente d’une réponse", done: "Terminée", missed: "Manquée", failed: "Échec / tâche inachevée" };
      row.append(node("strong", "", labels[o.status.status] || o.status.status), node("p", "routines-note", `${date(o.scheduled_at_utc, r.time_zone)} · ${r.time_zone} · ${o.manual_nonce ? "Manuel" : "Planifié"} · version ${o.schedule_version}`));
      if (o.status.reason) row.append(node("p", "routines-error", o.status.reason));
      if (o.status.run_id) row.append(button("Ouvrir le fil du run", () => openThread(r.conversation_id)));
      content.append(row);
    }
    if (occurrences.length < h.occurrences.length) content.append(button("Afficher plus d’occurrences", () => showHistory(r, "occurrences")));
  }
  async function runNow(r) {
    if (busy) return;
    let job = launches.get(r.routine_id);
    if (job && Date.now() < job.until) return;
    const token = revision; busy = true; signature = ""; renderList(); message("Vérification du dernier lancement…");
    try {
      const h = await readHistory(r), own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce);
      if (own && job.unknown) {
        // This click was a retry, not permission to run again after a lost reply.
        job.unknown = false; job.until = Date.now() + 3000;
        if (active(token)) message("Le lancement précédent est confirmé dans l’historique.");
        return;
      }
      if (h.occurrences.some(pending)) { if (active(token)) message("Un lancement est déjà en attente ou en cours. Consultez son historique."); return; }
      if (own) { launches.delete(r.routine_id); job = null; } // A known terminal run must not swallow a new click.
      if (!job) { job = { nonce: crypto.randomUUID(), until: 0, unknown: true }; launches.set(r.routine_id, job); }
      job.unknown = true;
      await panels.api("/api/routines/run-now", { ...idBody(r), nonce: job.nonce });
      job.unknown = false; job.until = Date.now() + 3000;
      if (active(token)) message("Lancement demandé. Son résultat apparaîtra dans l’historique.");
      await readHistory(r);
      try { await panels.refresh(); } catch { /* The page-session nonce remains available for retry. */ }
    } catch (error) { if (active(token)) message(`Lancement non confirmé : ${error.message} Réessayez avec le même identifiant ; aucun nouveau lancement ne sera créé.`, true); }
    finally { busy = false; if (active(token)) { signature = ""; renderList(); kick(); } }
  }
  async function pollHistory() {
    if (!dialog?.open || busy || reloading || historyLoading || Date.now() - historyAt < 3000) return;
    const candidates = new Map(items().map((i) => i.routine).filter(tracked).map((r) => [r.routine_id, r]));
    if (view === "history" && current) candidates.set(current.routine_id, current);
    if (!candidates.size) return;
    const token = revision; historyAt = Date.now(); historyLoading = true;
    try {
      // Never let one approval-blocked run starve other routines. A failure of
      // one history read must not prevent the remaining routines from updating.
      for (const r of candidates.values()) {
        if (!active(token) || busy || reloading) break;
        try {
          const h = await readHistory(r); settleLaunch(r);
          if (active(token) && view === "history" && current.routine_id === r.routine_id) renderHistory(r, h);
        } catch (error) { if (active(token) && view !== "edit") message(`Historique à actualiser : ${error.message}`, true); }
      }
      if (active(token)) { signature = ""; renderList(); }
    } finally { historyLoading = false; }
  }
  panels.register({
    id: "routines",
    settingsSection(container) {
      const section = node("section", "routines-section");
      section.append(node("h3", "", "Routines"), button("Gérer les routines par bot", () => open(liveBots()[0]?.id))); container.append(section);
    },
    botSection(bot, container) {
      const section = node("section", "routines-section"); section.append(node("h3", "", "Routines"));
      if (bot?.id && !bot.deleted) section.append(button("Gérer les routines de ce bot", () => open(bot.id)));
      else section.append(node("p", "routines-note", "Enregistrez le bot avant de créer ses routines."));
      container.append(section);
    },
    onState() {
      if (!dialog?.open) return;
      const bot = liveBots().find((b) => b.id === botId());
      if (!bot) { dialog.close(); panels.notify("Ce bot a été supprimé ; ses routines ne sont plus modifiables ici."); return; }
      renderList(); void pollHistory();
    },
    wantsPolling,
  });
})();
