/* REA reports/cron-detail.md CR01, CR08–13, CR16–19: per-bot routines,
 * inline prompts, five-field cron (DOM OR DOW), @every phase, explicit timezone,
 * manual cooldown and conversational model confirmation. The editor, durable
 * nonce and missed-range history are local reconstruction, not server REA.
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
  function duration(text) {
    let rest = text, total = 0;
    while (rest) {
      const part = /^(\d+(?:\.\d+)?)(ms|s|m|h|d)/.exec(rest);
      if (!part) throw new Error("Durée attendue : 1h30m, 10m ou 1d (unités minuscules).");
      total += Number(part[1]) * { ms: 1, s: 1000, m: 60000, h: 3600000, d: 86400000 }[part[2]];
      rest = rest.slice(part[0].length);
    }
    if (!Number.isSafeInteger(total)) throw new Error("La durée doit être un nombre entier de millisecondes représentable.");
    return total;
  }
  function cadence(ms) {
    for (const [unit, divisor] of [["jour(s)", 86400000], ["heure(s)", 3600000], ["minute(s)", 60000], ["seconde(s)", 1000]]) {
      if (ms && ms % divisor === 0) return `${ms / divisor} ${unit}`;
    }
    return `${ms} ms`;
  }
  // This is a description/early check only, never a client-side next-run oracle.
  // In particular, never describe 1-7 DOM + Monday as "first Monday" (CR09).
  function schedule(text, zone) {
    text = text.trim().replace(/\s+/g, " ");
    const prefix = /^(?:CRON_TZ|TZ)=(\S+) (.+)$/.exec(text);
    if (prefix) { zone = prefix[1]; text = prefix[2]; }
    new Intl.DateTimeFormat("fr", { timeZone: zone }).format();
    if (text.startsWith("@every ")) {
      if (prefix) throw new Error("@every n’accepte pas de préfixe TZ ; utilisez le champ Fuseau.");
      const parts = text.slice(7).split("/");
      if (parts.length > 2 || parts.some((part) => !part)) throw new Error("Intervalle attendu : @every 1h30m ou @every 1h/10m.");
      const interval = duration(parts[0]), phase = parts.length === 2 ? duration(parts[1]) : null;
      if (interval < 60000) throw new Error("L’intervalle d’exécution minimum est d’une minute.");
      if (phase !== null && phase >= interval) throw new Error("La phase doit être inférieure à l’intervalle.");
      return { trigger: { kind: "every", interval_ms: interval, phase_ms: phase }, zone, text: `Toutes les ${cadence(interval)} · ${phase === null ? "à partir de l’activation" : `phase ${cadence(phase)} depuis l’époque UTC`}` };
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
    const [minute, hour, dom, month, dow] = fields;
    const time = /^\d+$/.test(minute) && /^\d+$/.test(hour) ? `À ${hour.padStart(2, "0")}:${minute.padStart(2, "0")}` : `Minutes : ${minute} · Heures : ${hour}`;
    const dayNames = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
    const week = dow.includes("/") ? `${dow} (0/7 = dimanche, 1 = lundi)` : dow.replace(/\d+/g, (n) => dayNames[Number(n)]);
    const days = dom !== "*" && dow !== "*" ? `jour du mois ${dom} OU ${week}` : dom !== "*" ? `jour du mois ${dom}` : dow !== "*" ? week : "tous les jours";
    return { trigger: { kind: "cron", expression: text, time_zone: null }, zone, text: `${time} · ${days} · ${month === "*" ? "tous les mois" : `mois ${month}`}` };
  }
  function describe(r) {
    try {
      if (r.trigger.kind === "once") return `Une fois : ${date(r.trigger.at, r.time_zone)} · ${r.time_zone}`;
      const t = r.trigger;
      const expression = t.kind === "cron" ? t.expression : `@every ${t.interval_ms}ms${t.phase_ms === null || t.phase_ms === undefined ? "" : `/${t.phase_ms}ms`}`;
      const parsed = schedule(expression, t.time_zone || r.time_zone);
      return `${parsed.text} · ${parsed.zone}`;
    } catch { return "Horaire illisible — consultez ou corrigez cette routine."; }
  }

  let dialog, botSelect, content, status, toolbar, view = "list", current = null;
  let busy = false, revision = 0, signature = "", historyAt = 0, historyLoading = false, historySignature = "", fieldId = 0;
  const launches = new Map(), histories = new Map();
  const liveBots = () => (panels.state.bots || []).filter((b) => !b.deleted);
  const botId = () => botSelect.value;
  const items = () => (panels.state.routines?.items || []).filter((item) => item.routine.bot_id === botId());
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
    dialog.addEventListener("close", () => { revision++; current = null; content.replaceChildren(); });
    (document.getElementById("panels") || document.body).append(dialog);
  }
  function open(bot) {
    build();
    if (busy) { panels.notify("Une action de routine est encore en cours."); return; }
    botSelect.replaceChildren(...liveBots().map((b) => { const option = node("option", "", b.name); option.value = b.id; return option; }));
    botSelect.value = liveBots().some((b) => b.id === bot) ? bot : liveBots()[0]?.id || "";
    view = "list"; current = null; signature = ""; revision++;
    message(""); dialog.showModal(); renderList();
    void reload();
  }
  async function reload() {
    if (busy) return;
    try { await panels.refresh(); if (dialog.open) { signature = ""; renderList(); if (view === "history") await showHistory(current); } }
    catch (error) { if (dialog.open) message(`Actualisation impossible : ${error.message}`, true); }
  }
  function renderList() {
    if (!dialog?.open || view !== "list") return;
    const list = items();
    const next = JSON.stringify([list, busy, [...launches], [...histories].map(([id, h]) => [id, h.occurrences])]);
    if (signature === next) return;
    signature = next; toolbar.hidden = false; content.replaceChildren();
    if (panels.state.routines?.error) content.append(node("p", "routines-error", panels.state.routines.error));
    if (!list.length) content.append(node("p", "routines-note", "Aucune routine pour ce bot."));
    for (const item of list) {
      const r = item.routine, row = node("article", "routines-row"); row.dataset.routineId = r.routine_id;
      row.append(node("h3", "", r.name), node("p", "routines-state", r.enabled ? "Active" : "En pause"), node("p", "routines-note", describe(r)), node("code", "routines-id", r.routine_id));
      if (r.origin === "model" || r.model_edited_at || r.untrusted) row.append(node("p", "routines-warning", "Proposée ou modifiée par un bot · texte non vérifié."));
      if (item.error) row.append(node("p", "routines-error", `Dernier diagnostic : ${item.error}`));
      const actions = node("div", "routines-actions");
      actions.append(button("Modifier", () => edit(r)), button(r.enabled ? "Mettre en pause" : "Reprendre…", () => r.enabled ? mutate("/api/routines/pause", idBody(r), "Routine mise en pause.") : reviewResume(r)), button("Historique", () => showHistory(r)), button("Ouvrir le fil", () => openThread(r.conversation_id)));
      const job = launches.get(r.routine_id), running = histories.get(r.routine_id)?.occurrences.some((o) => ["pending", "running"].includes(o.status.status));
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
    const name = field("Nom"); name.value = r?.name || ""; name.maxLength = 160;
    const kind = field("Contenu", "select");
    for (const [value, label] of [["inline_prompt", "Prompt libre"], ["skill_invocation", "Skill · version épinglée"]]) { const opt = node("option", "", label); opt.value = value; kind.append(opt); }
    kind.value = r?.body.kind || "inline_prompt";
    const prompt = field("Instructions de la routine", "textarea"); prompt.rows = 6; prompt.value = r?.body.prompt || "";
    const skill = field("Identifiant de la skill de ce bot"); skill.value = r?.body.skill_id || "";
    const version = field("Version épinglée de la skill"); version.type = "number"; version.min = "1"; version.step = "1"; version.value = String(r?.body.version || 1);
    const skillHint = node("p", "routines-note", "La version exacte est conservée, même si la skill évolue. Consultez la section Skills pour ses identifiants et versions."); fields.append(skillHint);
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
        new Intl.DateTimeFormat("fr", { timeZone: zone.value.trim() }).format();
        return { trigger: { kind: "once", at }, zone: zone.value.trim(), text: `Une fois : ${date(at, zone.value.trim())}` };
      }
      return schedule(expression.value, zone.value.trim());
    }
    function update() {
      prompt.parentElement.hidden = kind.value !== "inline_prompt";
      skill.parentElement.hidden = version.parentElement.hidden = skillHint.hidden = kind.value !== "skill_invocation";
      expression.parentElement.hidden = mode.value !== "schedule"; once.parentElement.hidden = mode.value !== "once";
      try { const p = parsed(); preview.textContent = `${p.text} · ${p.zone}`; preview.classList.remove("routines-error"); }
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
          if (!name.value.trim()) throw new Error("Donnez un nom à la routine.");
          if (!r || values.name !== initial.name) payload.name = name.value;
          if (!r || ["kind", "prompt", "skill", "version"].some((key) => values[key] !== initial[key])) {
            if (kind.value === "inline_prompt") {
              if (!prompt.value.trim() || new TextEncoder().encode(prompt.value).length > 256000) throw new Error("Le prompt doit contenir de 1 à 256 000 octets UTF-8.");
              payload.body = { kind: "inline_prompt", prompt: prompt.value };
            } else {
              if (!skill.value.trim() || !Number.isSafeInteger(Number(version.value)) || Number(version.value) < 1) throw new Error("Indiquez un identifiant et une version entière positive de skill.");
              payload.body = { kind: "skill_invocation", skill_id: skill.value.trim(), version: Number(version.value) };
            }
          }
          if (!r || ["mode", "expression", "once", "zone"].some((key) => values[key] !== initial[key])) { payload.trigger = p.trigger; payload.time_zone = p.zone; }
          if (r) Object.assign(payload, { routine_id: r.routine_id, modified_at: r.modified_at, schedule_version: r.schedule_version });
          else payload.enabled = enabled.checked;
          if (r && Object.keys(payload).length === 4) { back(); return; }
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
      if (active(token)) message(`Action non confirmée : ${error.message} Vos saisies sont conservées. Actualisez la liste avant de réessayer ; en cas de conflit, revenez à la liste puis rouvrez l’éditeur.`, true);
    } finally {
      busy = false;
      if (active(token)) { if (view === "list") { signature = ""; renderList(); } else lock(); }
    }
  }
  async function readHistory(r) {
    const h = await panels.api(`/api/routines/history?bot_id=${encodeURIComponent(r.bot_id)}&routine_id=${encodeURIComponent(r.routine_id)}`);
    histories.set(r.routine_id, h); return h;
  }
  async function showHistory(r) {
    if (busy) return;
    const token = ++revision; view = "history"; current = r; toolbar.hidden = true; botSelect.disabled = true; historySignature = "";
    content.replaceChildren(button("Retour à la liste", back), button("Actualiser l’historique", () => showHistory(r))); message("Chargement de l’historique…");
    try { const h = await readHistory(r); if (active(token)) { message(""); renderHistory(r, h); } }
    catch (error) { if (active(token)) message(`Historique indisponible : ${error.message}`, true); }
  }
  function renderHistory(r, h) {
    const next = JSON.stringify([r.routine_id, h]);
    if (historySignature === next) return;
    historySignature = next;
    content.replaceChildren(button("Retour à la liste", back), button("Actualiser l’historique", () => showHistory(r)), node("h3", "", r.name));
    // Ranges first: the individual retained misses may be the oldest (review r2).
    const ranges = node("div", "routines-ranges"); ranges.append(node("h4", "", "Plages d’occurrences manquées"));
    if (!h.missed_ranges.length) ranges.append(node("p", "routines-note", "Aucune plage manquée."));
    for (const range of h.missed_ranges.slice().sort((a, b) => b.from_inclusive - a.from_inclusive)) ranges.append(node("p", "routines-warning", `Manquées du ${date(range.from_inclusive, range.time_zone)} inclus au ${date(range.until_exclusive, range.time_zone)} exclu · ${range.time_zone} · version ${range.schedule_version} · ${describe({ trigger: range.trigger, time_zone: range.time_zone })}`));
    content.append(ranges, node("h4", "", "Occurrences (plus récentes en premier)"));
    if (!h.occurrences.length) content.append(node("p", "routines-note", "Aucune occurrence."));
    for (const o of h.occurrences.slice().sort((a, b) => b.scheduled_at_utc - a.scheduled_at_utc)) {
      const row = node("article", "routines-occurrence"); row.dataset.status = o.status.status;
      const labels = { pending: "En attente", running: "En cours / en attente d’une réponse", done: "Terminée", missed: "Manquée", failed: "Échec / tâche inachevée" };
      row.append(node("strong", "", labels[o.status.status] || o.status.status), node("p", "routines-note", `${date(o.scheduled_at_utc, r.time_zone)} · ${r.time_zone} · ${o.manual_nonce ? "Manuel" : "Planifié"} · version ${o.schedule_version}`), node("code", "routines-id", o.occurrence_id));
      if (o.status.reason) row.append(node("p", "routines-error", o.status.reason));
      if (o.status.run_id) row.append(node("code", "routines-id", o.status.run_id), button("Ouvrir le fil du run", () => openThread(r.conversation_id)));
      content.append(row);
    }
  }
  async function runNow(r) {
    if (busy) return;
    let job = launches.get(r.routine_id);
    if (job && Date.now() < job.until) return;
    const token = revision; busy = true; signature = ""; renderList(); message("Vérification du dernier lancement…");
    try {
      const h = await readHistory(r), own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce);
      if (own) { job.unknown = false; job.until = 0; }
      if (h.occurrences.some((o) => ["pending", "running"].includes(o.status.status))) { message("Un lancement est déjà en attente ou en cours. Consultez son historique."); return; }
      if (own) { launches.delete(r.routine_id); message("Le lancement précédent est confirmé dans l’historique. Cliquez à nouveau pour en demander un nouveau."); return; }
      if (!job) { job = { nonce: crypto.randomUUID(), until: 0, unknown: true }; launches.set(r.routine_id, job); }
      // Keep this nonce across close/reopen and lost responses. Retrying can
      // never create a second intent for the same user click (CR16 + local rule).
      job.unknown = true;
      await panels.api("/api/routines/run-now", { ...idBody(r), nonce: job.nonce });
      job.unknown = false; job.until = Date.now() + 3000;
      if (active(token)) message("Lancement demandé. Son résultat apparaîtra dans l’historique.");
      await readHistory(r);
      try { await panels.refresh(); } catch { /* The durable nonce remains. */ }
    } catch (error) { if (active(token)) message(`Lancement non confirmé : ${error.message} Réessayez avec le même identifiant ; aucun nouveau lancement ne sera créé.`, true); }
    finally { busy = false; if (active(token)) { signature = ""; renderList(); } }
  }
  async function pollHistory() {
    if (!dialog?.open || busy || historyLoading || Date.now() - historyAt < 3000) return;
    const r = view === "history" ? current : items().map((i) => i.routine).find((r) => launches.has(r.routine_id) || histories.get(r.routine_id)?.occurrences.some((o) => ["pending", "running"].includes(o.status.status)));
    if (!r) return;
    const token = revision; historyAt = Date.now(); historyLoading = true;
    try {
      const h = await readHistory(r), job = launches.get(r.routine_id);
      const own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce);
      if (own && !job.unknown && Date.now() >= job.until && !["pending", "running"].includes(own.status.status)) launches.delete(r.routine_id);
      if (active(token)) { if (view === "history") renderHistory(r, h); else { signature = ""; renderList(); } }
    }
    catch (error) { if (active(token)) message(`Historique à actualiser : ${error.message}`, true); }
    finally { historyLoading = false; }
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
    wantsPolling: () => Boolean(dialog?.open),
  });
})();
