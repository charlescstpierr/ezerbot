/* REA reports/cron-detail.md CR01, CR08–13, CR16–19: per-bot routines,
 * inline prompts, five-field cron (DOM OR DOW), @every phase, explicit timezone,
 * manual cooldown and conversational model confirmation. The editor, durable
 * nonce aliases, snapshot pagination and run-output links are local choices;
 * the original server's guarantees and run/conversation join are unknown.
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
      return { trigger: { kind: "every", interval_ms: interval, phase_ms: phase }, zone, text: `${repeat}, ${phase === null ? "à partir de l’activation" : `décalé de ${cadence(phase)} sur l’horloge UTC`}` };
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
    const calendarDays = values(dom, 1, 31), ranges = [], ordinal = (n) => n === 1 ? "1er" : String(n);
    for (let i = 0; i < calendarDays.length; i++) {
      let end = i;
      while (calendarDays[end + 1] === calendarDays[end] + 1) end++;
      if (end - i >= 2) ranges.push(`du ${ordinal(calendarDays[i])} au ${ordinal(calendarDays[end])}`);
      else for (let j = i; j <= end; j++) ranges.push(`le ${ordinal(calendarDays[j])}`);
      i = end;
    }
    const monthDays = `${join(ranges)} du mois`;
    const days = dom !== "*" && dow !== "*" ? `${monthDays} OU ${week}` : dom !== "*" ? monthDays : dow !== "*" ? week : "tous les jours";
    const monthNames = ["", "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    const months = month === "*" ? "" : `, en ${join(values(month, 1, 12).map((n) => monthNames[n]))}`;
    return { trigger: { kind: "cron", expression: text, time_zone: null }, zone, text: `${days[0].toUpperCase()}${days.slice(1)} ${time}${months}` };
  }
  const scheduleLabel = (parsed) => parsed.trigger.kind === "every" ? parsed.text : `${parsed.text} (${parsed.zone})`;
  function describe(r) {
    if (r.schedule_description) return r.schedule_description;
    try {
      if (r.trigger.kind === "once") return `Une fois : ${date(r.trigger.at, r.time_zone)} · ${r.time_zone}`;
      const t = r.trigger;
      const expression = t.kind === "cron" ? t.expression : `@every ${t.interval_ms}ms${t.phase_ms === null || t.phase_ms === undefined ? "" : `/${t.phase_ms}ms`}`;
      const parsed = schedule(expression, t.time_zone || r.time_zone);
      return scheduleLabel(parsed);
    } catch { return "Horaire illisible — consultez ou corrigez cette routine."; }
  }

  let dialog, botSelect, content, status, toolbar, view = "list", current = null;
  let covered = [], restoreCovered = true;
  let busy = false, revision = 0, signature = "", historyAt = 0, historyLoading = false, reloading = false, historySignature = "", fieldId = 0;
  let historyView = null, uncertainResume = null, notices, hostOnline = true;
  // Funnel is asked live (never trusted from an old poll); a Bearer token is kept
  // in memory only while it is shown, right after its creation or regeneration.
  let funnel = null, funnelLoading = false, funnelBusy = false, regenerating = null;
  const revealed = new Map();
  // Retain the intent across a page reload, without storing prompts or outputs.
  let savedLaunches = [];
  try { savedLaunches = JSON.parse(sessionStorage.getItem("ezerbot.routine-launches") || "[]"); } catch { /* Optional browser storage. */ }
  const launches = new Map(Array.isArray(savedLaunches) ? savedLaunches : []), histories = new Map(), historyReads = new Map();
  function saveLaunches() { try { sessionStorage.setItem("ezerbot.routine-launches", JSON.stringify([...launches])); } catch { /* Server aliases remain authoritative. */ } }
  const liveBots = () => (panels.state.bots || []).filter((b) => !b.deleted);
  const botId = () => botSelect.value;
  const items = () => (panels.state.routines?.items || []).filter((item) => item.routine.bot_id === botId()).sort((a, b) => Number(b.routine.enabled) - Number(a.routine.enabled) || a.routine.name.localeCompare(b.routine.name, "fr") || a.routine.routine_id.localeCompare(b.routine.routine_id));
  const tracked = (r) => launches.has(r.routine_id) || histories.get(r.routine_id)?.running || items().some((i) => i.routine.routine_id === r.routine_id && i.running);
  const wantsPolling = () => Boolean(dialog?.open);
  // The host only schedules its next poll at the end of refresh(). Changes
  // caused by a panel-local history read must explicitly rearm that scheduler.
  function kick() { if (wantsPolling()) void panels.refresh().catch(() => {}); }
  const idBody = (r) => ({ bot_id: r.bot_id, routine_id: r.routine_id });
  const active = (token) => dialog.open && token === revision;
  function message(text, error = false, retry, retryLabel = "Réessayer") {
    delete status.dataset.launchResult;
    status.replaceChildren(node("span", "", text)); status.classList.toggle("routines-error", error);
    if (retry) status.append(button(retryLabel, retry));
    status.setAttribute("role", error ? "alert" : "status");
    // Only a long editor needs to reveal its error. History errors must never
    // move the reader away from the page or control they were using.
    if (error && view !== "history" && status.dataset.message !== text) status.scrollIntoView?.({ block: "center" });
    status.dataset.message = text;
  }
  function historyPosition() {
    const position = { scrollTop: dialog.scrollTop };
    if (view !== "history" || dialog.scrollTop < 120 || !dialog.getBoundingClientRect) return position;
    const bounds = dialog.getBoundingClientRect();
    const row = [...content.querySelectorAll("[data-history-focus]")].find((el) => (el.tagName === "ARTICLE" || el.tagName === "P") && el.getBoundingClientRect().bottom > bounds.top && el.getBoundingClientRect().top < bounds.bottom);
    if (row) { position.key = row.dataset.historyFocus; position.offset = row.getBoundingClientRect().top - bounds.top; }
    return position;
  }
  function restoreHistoryPosition(position) {
    const row = position.key && [...content.querySelectorAll("[data-history-focus]")].find((el) => el.dataset.historyFocus === position.key);
    if (row) dialog.scrollTop += row.getBoundingClientRect().top - dialog.getBoundingClientRect().top - position.offset;
    else dialog.scrollTop = position.scrollTop;
  }
  function renderNotices() {
    if (!notices || !dialog?.open) return;
    const next = JSON.stringify([hostOnline, view === "history" && historyView?.newer, view === "history" && historyView?.failures.poll && !historyView.failures.initial && !historyView.failures.occurrences && !historyView.failures.ranges]);
    if (notices.dataset.signature === next) return;
    notices.dataset.signature = next;
    const position = historyPosition();
    notices.replaceChildren();
    if (!hostOnline) {
      const warning = node("div", "routines-warning routines-history-notice");
      warning.setAttribute("role", "alert");
      warning.append(node("span", "", "Moteur local injoignable — reconnexion automatique en cours. Vos données affichées sont conservées."), button("Réessayer", () => { void panels.refresh().catch(() => {}); }));
      notices.append(warning);
    }
    if (view === "history" && historyView?.newer) {
      const news = node("div", "routines-warning routines-history-notice");
      news.append(node("span", "", "De nouvelles occurrences sont disponibles. Votre lecture reste dans le même ordre."), button("Voir les plus récentes", () => showHistory(current)));
      notices.append(news);
    }
    if (view === "history" && historyView?.failures.poll && !historyView.failures.initial && !historyView.failures.occurrences && !historyView.failures.ranges) {
      const warning = node("div", "routines-warning routines-history-notice"); warning.setAttribute("role", "alert");
      warning.append(node("span", "", `Mise à jour de l’historique indisponible : ${historyView.failures.poll} Les pages affichées sont conservées.`), button("Réessayer", () => retryLoadedPages(current)));
      notices.append(warning);
    }
    restoreHistoryPosition(position);
  }
  function frenchError(error, diagnostic = false) {
    const text = error?.message || "";
    const known = { "Routine run cancelled": "L’exécution a été arrêtée.", "Routine run failed": "L’exécution a échoué. Consultez son résultat pour le détail.", "Routine turn ended without complete_task": "Le bot a répondu sans confirmer la fin de la tâche.", "Routine paused or edited": "La routine a été mise en pause ou modifiée avant le départ.", "Occurrence no longer runnable": "Cette occurrence ne peut plus être lancée : la routine a changé." };
    if (known[text]) return known[text];
    if (/schedule never fires|no future slot|schedule exhausted/i.test(text)) return "L’horaire n’a plus de créneau futur. Choisissez un nouvel horaire.";
    if (/unknown routine/i.test(text)) return "La routine est introuvable ou a été supprimée.";
    if (/time.?zone|time zone|zoneinfo|failed to find.*zone/i.test(text)) return "Le fuseau horaire de la routine est invalide. Ouvrez-la pour le corriger.";
    if (/invalid schedule|invalid cron|interval.*invalid/i.test(text)) return "L’horaire de la routine est invalide. Ouvrez-la pour le corriger.";
    if (/changé|Conflit|créneau|introuvable|indisponible|connexion|illisible|Curseur|horaire|Réessayez/i.test(text)) return text;
    return diagnostic ? "Diagnostic technique non traduit. Vérifiez l’horaire et le contenu de cette routine avant de la relancer." : "Le moteur local n’a pas pu terminer cette action. Vos données sont conservées.";
  }
  function lock() {
    botSelect.disabled = busy || view !== "list";
    toolbar.querySelectorAll("button").forEach((el) => { el.disabled = busy; });
    content.querySelectorAll("button, input, textarea, select").forEach((el) => { el.disabled = busy || el.dataset.readOnly === "true"; });
    dialog.setAttribute("aria-busy", String(busy));
  }
  function openThread(id, messageId) {
    if (!panels.openConversation(id, messageId)) { message("Le fil n’est pas encore disponible.", true, async () => { try { await panels.refresh(); openThread(id, messageId); } catch (error) { message(frenchError(error), true, () => openThread(id, messageId)); } }); return; }
    restoreCovered = false; // Thread navigation closes the entry dialogs too.
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
    content = node("div", "routines-content"); notices = node("div", "routines-live-notices");
    dialog.append(header, label, node("p", "routines-note", "Les routines s’exécutent quand le moteur local est ouvert, ou quand l’agent en arrière-plan est activé dans Paramètres. Au retour : un seul rattrapage dans les 60 dernières minutes ; les autres créneaux sont marqués manqués. Une pause ne se rattrape pas."), toolbar, notices, status, content);
    dialog.addEventListener("close", () => {
      if (dialog.open) return;
      revision++; current = null; content.replaceChildren(); revealed.clear(); regenerating = null;
      for (const { entry, scrollTop, focus } of covered.splice(0)) {
        entry.classList.remove("routines-covered");
        if (restoreCovered && entry.open) {
          if (focus?.isConnected) focus.focus({ preventScroll: true });
          entry.scrollTop = scrollTop;
        }
      }
    });
    (document.getElementById("panels") || document.body).append(dialog);
  }
  function open(bot) {
    build();
    if (busy) { panels.notify("Une action de routine est encore en cours."); return; }
    botSelect.replaceChildren(...liveBots().map((b) => { const option = node("option", "", b.name); option.value = b.id; return option; }));
    botSelect.value = liveBots().some((b) => b.id === bot) ? bot : liveBots()[0]?.id || "";
    view = "list"; current = null; signature = ""; revision++;
    // Keep entry dialogs mounted (including unsaved bot/memory/skill drafts),
    // but hide their surfaces and backdrops while Routines is the visible layer.
    restoreCovered = true;
    covered = ["settings-dialog", "bot-dialog"].map((name) => document.getElementById(name)).filter((entry) => entry?.open).map((entry) => {
      const snapshot = { entry, scrollTop: entry.scrollTop, focus: entry.contains(document.activeElement) ? document.activeElement : null };
      entry.classList.add("routines-covered");
      return snapshot;
    });
    message(""); dialog.showModal(); hostOnline = panels.connected !== false; renderNotices(); renderList();
    void reload(); void loadFunnel();
  }
  const webhookItems = () => panels.state.routine_webhooks?.items || {};
  // A publication stays managed after its last webhook is gone: it must stay withdrawable.
  const publicationOn = () => Boolean(panels.state.routine_webhooks?.funnel?.enabled);
  async function loadFunnel() {
    if (funnelLoading || !(Object.keys(webhookItems()).length || publicationOn())) return;
    const token = revision; funnelLoading = true;
    try { funnel = await panels.api("/api/routines/funnel"); }
    catch (error) { funnel = { state: "unconfirmed", reason: `Tailscale n’a pas pu être vérifié : ${frenchError(error)}`, public_base: null }; }
    finally { funnelLoading = false; if (active(token)) { signature = ""; renderList(); } }
  }
  async function reload() {
    if (busy || reloading) return;
    const token = revision; reloading = true;
    try {
      await panels.refresh(); void loadFunnel();
      // Explicit refresh must repair cached running states too, not only /state.
      for (const { routine: r } of items()) {
        if (!active(token) || busy) break;
        if (histories.has(r.routine_id) || launches.has(r.routine_id)) {
          try { await readHistory(r); settleLaunch(r); }
          catch (error) { if (active(token)) message(`Historique à actualiser : ${frenchError(error)}`, true, reload); }
        }
      }
      if (active(token)) { signature = ""; renderList(); if (view === "history") await showHistory(current); }
    } catch (error) { if (active(token)) message(`Actualisation impossible : ${frenchError(error)}`, true, reload); }
    finally { reloading = false; if (active(token)) kick(); }
  }
  async function copy(text, done) {
    try { await panels.copyText(text); panels.notify(done); }
    catch { panels.notify("La copie a échoué. Sélectionnez le texte et copiez-le à la main."); }
  }
  function funnelPanel() {
    const box = node("section", "routines-funnel"), listener = panels.state.routine_webhooks?.listener;
    box.append(node("h3", "", "Publication sur Internet · Tailscale Funnel"), node("p", "routines-note", "Seul le chemin des webhooks devient public, sur son propre port HTTPS. L’adresse privée de l’iPhone (Tailscale Serve) ne change pas, et chaque appel exige son jeton Bearer. L’écoute vit tant qu’Ezerbot est ouvert : un appel reçu pendant que l’app est fermée est perdu."));
    if (listener?.error) { const warning = node("p", "routines-error", listener.error); warning.setAttribute("role", "alert"); box.append(warning); }
    const state = node("p", funnel?.state === "published" ? "routines-note" : "routines-warning", !funnel ? "Vérification de Tailscale en cours…" : funnel.state === "published" ? "Publié par Tailscale Funnel." : funnel.reason || "Publication non confirmée.");
    state.setAttribute("role", "status"); state.dataset.funnelState = funnel?.state || "loading"; box.append(state);
    const actions = node("div", "routines-actions");
    if (funnel && ["published", "withdraw_failed"].includes(funnel.state)) actions.append(button("Retirer la publication…", () => publishFunnel(false)));
    else if (funnel && ["off", "refused", "unconfirmed"].includes(funnel.state)) actions.append(button("Publier avec Tailscale Funnel…", () => publishFunnel(true), "primary-button"));
    actions.append(button("Vérifier Tailscale", () => { funnel = null; signature = ""; renderList(); void loadFunnel(); }));
    if (funnelBusy) actions.querySelectorAll("button").forEach((el) => { el.disabled = true; });
    box.append(actions); return box;
  }
  async function publishFunnel(enabled) {
    if (funnelBusy) return;
    funnelBusy = true; signature = ""; renderList();
    try {
      const result = await panels.api("/api/routines/funnel", { enabled });
      if (result.conversation_id) { await panels.refresh(); openThread(result.conversation_id, result.approval_message_id); panels.notify("Confirmez l’action dans la carte d’approbation."); }
    } catch (error) { message(`Publication impossible : ${error.message}`, true); }
    finally { funnelBusy = false; signature = ""; renderList(); }
  }
  function webhookBlock(r) {
    const info = webhookItems()[r.routine_id], box = node("div", "routines-webhook");
    box.append(node("h4", "", "Webhook"));
    const url = funnel?.state === "published" && funnel.public_base && info ? `${funnel.public_base}/${r.routine_id}/${info.trigger_id}` : "";
    if (url) {
      const line = node("p", "routines-webhook-url"); line.append(node("code", "", url));
      box.append(node("p", "routines-note", "URL publique (HTTPS)"), line, button("Copier l’URL", () => copy(url, "URL copiée.")));
    } else box.append(node("p", "routines-warning", funnel ? `Pas d’URL publique : ${funnel.reason || "publication non confirmée."}` : "Aucune URL publique n’est affichée avant la confirmation de Tailscale."));
    box.append(node("p", "routines-note", "Appelez l’URL en POST avec l’en-tête Authorization: Bearer <jeton>. Le jeton n’est pas dans l’URL. Le corps JSON est transmis au bot ; une clé Idempotency-Key évite un second départ lors d’un nouvel essai."));
    const secret = revealed.get(r.routine_id), token = node("p", "routines-webhook-token");
    token.append(node("span", "", "Jeton Bearer : "), node("code", "", secret || "••••••••••••••••"));
    box.append(token);
    if (secret) box.append(node("p", "routines-warning", "Copiez ce jeton maintenant. Il sera masqué ensuite, mais il restera copiable."));
    const actions = node("div", "routines-actions");
    actions.append(button("Copier le jeton", async () => {
      try { const reply = await panels.api("/api/routines/webhook/token", idBody(r)); await copy(reply.token, "Jeton copié."); }
      catch (error) { message(`Copie impossible : ${error.message}`, true); }
    }));
    if (secret) actions.append(button("Masquer le jeton", () => { revealed.delete(r.routine_id); signature = ""; renderList(); }));
    actions.append(button("Régénérer le jeton…", () => { regenerating = r.routine_id; signature = ""; renderList(); }));
    box.append(actions);
    if (regenerating === r.routine_id) {
      const confirm = node("div", "routines-delete");
      confirm.append(node("p", "routines-warning", "Régénérer le jeton ? L’ancien jeton cesse de fonctionner tout de suite : mettez à jour l’appelant."), button("Régénérer", async () => {
        try { const reply = await panels.api("/api/routines/webhook/regenerate", idBody(r)); revealed.set(r.routine_id, reply.token); regenerating = null; message("Jeton régénéré. L’ancien jeton est refusé."); }
        catch (error) { message(`Régénération impossible : ${error.message}`, true); }
        signature = ""; renderList();
      }, "danger-button"), button("Garder le jeton", () => { regenerating = null; signature = ""; renderList(); }));
      box.append(confirm);
    }
    return box;
  }
  function renderList() {
    if (!dialog?.open || view !== "list") return;
    const list = items();
    const next = JSON.stringify([list, busy, [...launches].map(([id, job]) => [id, job, Date.now() < job.until]), [...histories], funnel, funnelBusy, [...revealed.keys()], regenerating, webhookItems(), panels.state.routine_webhooks?.listener, panels.state.routine_webhooks?.funnel?.enabled]);
    if (signature === next) return;
    signature = next; toolbar.hidden = false; content.replaceChildren();
    if (panels.state.routines?.error) content.append(node("p", "routines-error", panels.state.routines.error));
    if (!list.length) content.append(node("p", "routines-note", "Aucune routine pour ce bot."));
    if (list.some((item) => item.routine.trigger?.kind === "webhook") || publicationOn()) content.append(funnelPanel());
    for (const item of list) {
      const r = item.routine, row = node("article", "routines-row"); row.dataset.routineId = r.routine_id;
      row.append(node("h3", "", r.name), node("p", "routines-state", r.enabled ? "Active" : "En pause"), node("p", "routines-note", describe(r)));
      if (r.review_stamp) row.append(node("p", "routines-note", r.review_stamp));
      if (r.trigger?.kind === "webhook") row.append(webhookBlock(r));
      if (r.origin === "model" || r.model_edited_at || r.untrusted) row.append(node("p", "routines-warning", "Proposée ou modifiée par un bot · texte non vérifié."));
      if (item.error) row.append(node("p", "routines-error", `Dernier diagnostic : ${frenchError({ message: item.error }, true)}`));
      const actions = node("div", "routines-actions");
      actions.append(button("Modifier", () => edit(r)), button(r.enabled ? "Mettre en pause" : "Reprendre…", () => r.enabled ? mutate("/api/routines/pause", idBody(r), "Routine mise en pause.") : reviewResume(r)), button("Historique", () => showHistory(r)), button("Ouvrir le fil", () => openThread(r.conversation_id)));
      const job = launches.get(r.routine_id), running = item.running || histories.get(r.routine_id)?.running;
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
  function back() { if (busy) return; uncertainResume = null; revision++; view = "list"; current = null; signature = ""; message(""); renderNotices(); renderList(); }
  async function edit(r, resume = false) {
    if (busy) return;
    const token = ++revision; view = "edit"; current = r; toolbar.hidden = true; content.replaceChildren(); botSelect.disabled = true;
    content.append(button("Retour à la liste", back)); message(r ? "Chargement du contenu complet…" : ""); renderNotices();
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
    } catch (error) { if (active(token)) message(`Confirmation indisponible : ${frenchError(error)}`, true, () => edit(r, resume)); }
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
    for (const [value, label] of [["schedule", "Cron / intervalle"], ["once", "Une seule fois"], ["webhook", "Webhook · appel HTTPS avec jeton Bearer"]]) { const opt = node("option", "", label); opt.value = value; mode.append(opt); }
    mode.value = r?.trigger.kind === "once" ? "once" : r?.trigger.kind === "webhook" ? "webhook" : "schedule";
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
      if (mode.value === "webhook") return { trigger: { kind: "webhook" }, zone: validZone(zone.value.trim() || localZone), text: "Webhook : démarre à chaque appel authentifié par le jeton Bearer. Le jeton et l’URL s’affichent après l’enregistrement." };
      return schedule(expression.value, zone.value.trim());
    }
    function update() {
      prompt.parentElement.hidden = kind.value !== "inline_prompt";
      skill.parentElement.hidden = version.parentElement.hidden = skillHint.hidden = kind.value !== "skill_invocation";
      expression.parentElement.hidden = mode.value !== "schedule"; once.parentElement.hidden = mode.value !== "once"; zone.parentElement.hidden = mode.value === "webhook";
      try { const p = parsed(); preview.textContent = scheduleLabel(p); preview.classList.remove("routines-error"); }
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
      actions.append(button("Activer cette routine", () => mutate("/api/routines/resume", { ...idBody(r), modified_at: r.modified_at, schedule_version: r.schedule_version }, "Routine activée.", r), "primary-button"));
    }
    actions.append(button("Annuler", back)); fields.append(actions); form.append(fields); content.append(form);
    if (!resume) name.focus();
  }
  async function mutate(path, payload, success, reviewed) {
    if (busy) return;
    const token = revision; uncertainResume = null; busy = true; lock(); message("Enregistrement…");
    let sent = false;
    try {
      if (reviewed) {
        const latest = await fetchRoutine(reviewed);
        if (latest.modified_at !== reviewed.modified_at || latest.schedule_version !== reviewed.schedule_version) throw new Error("La routine a changé depuis votre lecture. Revenez à la liste et relisez-la avant de l’activer.");
      }
      sent = true;
      const reply = await panels.api(path, payload);
      if (reply?.webhook?.token && reply.routine) revealed.set(reply.routine.routine_id, reply.webhook.token);
      if (active(token)) { view = "list"; current = null; signature = ""; message(success); }
      try { await panels.refresh(); } catch { if (active(token)) message(`${success} Liste à actualiser.`, true); }
      if (reply?.webhook) void loadFunnel();
    } catch (error) {
      if (active(token)) {
        const conflict = /Stale routine edit|changé depuis votre lecture|Routine changed|409/i.test(error.message);
        const disconnected = /connexion.*interrompue|réponse illisible/i.test(error.message);
        const uncertain = sent && disconnected;
        const reason = /schedule never fires|Schedule has no future slot/i.test(error.message) ? "L’horaire n’a plus de créneau futur. Choisissez un instant futur." : error.message;
        if (uncertain && reviewed) uncertainResume = reviewed.routine_id;
        const advice = conflict ? "Revenez à la liste, actualisez puis rouvrez l’éditeur." : uncertain ? reviewed ? "L’état sera vérifié automatiquement à la reconnexion." : "Vérifiez l’état après reconnexion avant de réessayer." : /indisponible/i.test(reason) ? "Réessayez dans un instant." : "Vérifiez la saisie avant de réessayer.";
        if (reviewed && disconnected && !sent) {
          message(`Activation non envoyée : ${frenchError(error)}`, true, () => mutate(path, payload, success, reviewed));
        } else if (conflict && reviewed) {
          message("Conflit d’activation : la routine a changé depuis votre lecture. Relisez la nouvelle version avant de l’activer.", true, () => reviewResume(reviewed), "Relire la routine");
        } else {
          message(`${conflict ? "Conflit d’édition" : uncertain ? "Action non confirmée" : "Action refusée par le moteur"} : ${frenchError({ message: reason })}${reviewed ? "" : " Vos saisies sont conservées."} ${advice}`, true, () => mutate(path, payload, success, reviewed));
        }
      }
    } finally {
      busy = false;
      if (active(token)) { renderNotices(); if (view === "list") { signature = ""; renderList(); } else lock(); }
    }
  }
  async function readHistory(r, cursor, rangesCursor) {
    const id = r.routine_id, request = (historyReads.get(id) || 0) + 1;
    historyReads.set(id, request);
    const h = await panels.api(`/api/routines/history?bot_id=${encodeURIComponent(r.bot_id)}&routine_id=${encodeURIComponent(id)}${cursor ? `&cursor=${cursor}` : ""}${rangesCursor ? `&ranges_cursor=${rangesCursor}` : ""}`);
    if (!Array.isArray(h.occurrences) || !Array.isArray(h.missed_ranges)) throw new Error("L’historique du moteur est illisible.");
    const job = launches.get(id), own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce || o.occurrence_id === job.occurrenceId);
    if (historyReads.get(id) === request) histories.set(id, { running: h.running, own: own ? { nonce: job.nonce, status: own.status.status } : null });
    return h;
  }
  function settleLaunch(r) {
    const job = launches.get(r.routine_id), own = histories.get(r.routine_id)?.own;
    if (job && !job.unknown && Date.now() >= job.until && ((own?.nonce === job.nonce && !["pending", "running"].includes(own.status)) || !histories.get(r.routine_id)?.running)) { launches.delete(r.routine_id); saveLaunches(); }
  }
  function mergePage(h, remember = true) {
    for (const o of h.occurrences) historyView.occurrences.set(o.occurrence_id, o);
    for (const range of h.missed_ranges) historyView.ranges.set(`${range.schedule_version}.${range.from_inclusive}`, range);
    if (remember) historyView.pages.set(`${h.cursor}/${h.ranges_cursor}`, [h.cursor, h.ranges_cursor]);
  }
  function emptyHistory() {
    return { occurrences: new Map(), ranges: new Map(), pages: new Map(), next: null, nextRanges: null, newer: false, rangesOpen: false, failures: { initial: null, poll: null, occurrences: null, ranges: null }, loading: "", focusTarget: null };
  }
  function installHistory(h) {
    const rangesOpen = historyView?.rangesOpen || false;
    historyView = emptyHistory(); historyView.rangesOpen = rangesOpen;
    mergePage(h); historyView.next = h.next_cursor; historyView.nextRanges = h.next_ranges_cursor;
  }
  async function showHistory(r, more = "") {
    if (busy || (more && historyView?.loading)) return;
    const token = ++revision; view = "history"; current = r; toolbar.hidden = true; botSelect.disabled = true;
    if (!more) {
      historyView = emptyHistory(); historySignature = "";
      content.replaceChildren(button("Retour à la liste", back)); message("Chargement de l’historique…");
    } else {
      if (content.contains(document.activeElement)) historyView.focusTarget = document.activeElement.dataset.historyFocus;
      historyView.loading = more;
      // Do not replace the DOM while waiting: the loading state belongs to the
      // control at the reader's position, not to a status above sixty cards.
      for (const control of content.querySelectorAll("button")) {
        if (control.dataset.loadMore || control.dataset.historyFocus?.startsWith("retry-")) {
          control.disabled = true;
          if (control.dataset.loadMore === more || control.dataset.historyFocus === `retry-${more}`) control.textContent = "Chargement…";
        }
      }
    }
    historyLoading = token; renderNotices();
    const cursor = more ? (more === "occurrences" ? historyView.next : [...historyView.pages.values()][0][0]) : null;
    const rangesCursor = more ? (more === "ranges" ? historyView.nextRanges : [...historyView.pages.values()][0][1]) : null;
    try {
      const h = await readHistory(r, cursor, rangesCursor); settleLaunch(r);
      if (active(token)) {
        const newItems = more === "occurrences" ? h.occurrences.filter((o) => !historyView.occurrences.has(o.occurrence_id)) : h.missed_ranges.filter((range) => !historyView.ranges.has(`${range.schedule_version}.${range.from_inclusive}`));
        mergePage(h);
        if (!more || more === "occurrences") historyView.next = h.next_cursor;
        if (!more || more === "ranges") historyView.nextRanges = h.next_ranges_cursor;
        historyView.failures[more || "initial"] = null;
        if (historyView.focusTarget && newItems.length) historyView.focusTarget = more === "occurrences" ? `occurrence-${newItems[0].occurrence_id}` : `range-${newItems[0].schedule_version}.${newItems[0].from_inclusive}`;
        if (!more) message("");
      }
    } catch (error) {
      if (active(token)) {
        historyView.failures[more || "initial"] = frenchError(error);
        if (!more) message(`Historique indisponible : ${frenchError(error)}`, true, () => retryLoadedPages(r));
      }
    } finally {
      if (historyLoading === token) historyLoading = false;
      if (active(token)) { historyView.loading = ""; historySignature = ""; renderHistory(r); kick(); }
    }
  }
  // Revalidation never starts a new traversal: preserve membership, cursor,
  // explicit page errors and position. Only a reader at the top of one page
  // can safely adopt fresh arrivals automatically.
  async function refreshLoadedPages(r, token) {
    const h = await readHistory(r); settleLaunch(r);
    if (!active(token)) return;
    const newer = h.occurrences.some((o) => !historyView.occurrences.has(o.occurrence_id));
    if (!historyView.pages.size || (newer && historyView.pages.size === 1 && dialog.scrollTop < 120 && !historyView.failures.occurrences && !historyView.failures.ranges)) {
      installHistory(h); message("");
    } else {
      if (newer) historyView.newer = true;
      for (const [cursor, rangesCursor] of [...historyView.pages.values()]) {
        const page = await readHistory(r, cursor, rangesCursor);
        if (!active(token)) return;
        mergePage(page, false);
      }
      historyView.failures.poll = null;
      if (historyView.failures.initial) { historyView.failures.initial = null; message(""); }
    }
    renderHistory(r);
  }
  async function retryLoadedPages(r) {
    if (busy || historyView?.loading || !r) return;
    const token = ++revision; historyLoading = token;
    try { await refreshLoadedPages(r, token); }
    catch (error) { if (active(token)) { historyView.failures.poll = frenchError(error); renderHistory(r); } }
    finally { if (historyLoading === token) historyLoading = false; if (active(token)) kick(); }
  }
  function pageControls(parent, r, kind) {
    if (!(kind === "occurrences" ? historyView.next : historyView.nextRanges)) return;
    const controls = node("div", "routines-pagination");
    const load = button(kind === "occurrences" ? "Charger plus" : "Charger plus de plages", () => showHistory(r, kind), "secondary-button routines-load-more");
    load.dataset.loadMore = kind; load.dataset.historyFocus = `load-${kind}`; controls.append(load);
    if (historyView.failures[kind]) {
      const error = node("div", "routines-error routines-page-error"); error.setAttribute("role", "alert");
      const retry = button("Réessayer", () => showHistory(r, kind)); retry.dataset.historyFocus = `retry-${kind}`;
      error.append(node("span", "", `Impossible de charger la suite : ${historyView.failures[kind]}`), retry); controls.append(error);
    }
    parent.append(controls);
  }
  function renderHistory(r) {
    renderNotices();
    const occurrences = [...historyView.occurrences.values()].sort((a, b) => b.scheduled_at_utc - a.scheduled_at_utc || (a.occurrence_id < b.occurrence_id ? 1 : -1));
    const visibleRanges = [...historyView.ranges.values()].sort((a, b) => b.from_inclusive - a.from_inclusive || b.schedule_version - a.schedule_version);
    const next = JSON.stringify([r.routine_id, occurrences, visibleRanges, historyView.next, historyView.nextRanges, historyView.failures]);
    if (historySignature === next) return;
    historySignature = next;
    const position = historyPosition(), focused = document.activeElement;
    const focusKey = focused?.disabled && historyView.focusTarget ? historyView.focusTarget : content.contains(focused) ? focused.dataset.historyFocus : (!focused || focused === document.body) ? historyView.focusTarget : null;
    const focusText = content.contains(focused) ? focused.textContent : null;
    content.replaceChildren(button("Retour à la liste", back), button("Actualiser l’historique", () => retryLoadedPages(r)), node("h3", "", r.name));
    const ranges = node("details", "routines-ranges");
    ranges.open = historyView.rangesOpen;
    ranges.append(node("summary", "", `Plages d’occurrences manquées (${visibleRanges.length}${historyView.nextRanges ? "+" : ""})`));
    ranges.addEventListener("toggle", () => { if (ranges.isConnected) historyView.rangesOpen = ranges.open; });
    if (!visibleRanges.length) ranges.append(node("p", "routines-note", "Aucune plage manquée."));
    for (const range of visibleRanges) {
      const row = node("p", "routines-warning", `Manquées du ${date(range.from_inclusive, range.time_zone)} inclus au ${date(range.until_exclusive, range.time_zone)} exclu · ${range.time_zone} · version ${range.schedule_version} · ${describe({ trigger: range.trigger, time_zone: range.time_zone })}`);
      row.tabIndex = -1; row.dataset.historyFocus = `range-${range.schedule_version}.${range.from_inclusive}`; ranges.append(row);
    }
    pageControls(ranges, r, "ranges");
    content.append(ranges, node("h4", "", `${occurrences.length} ${occurrences.length > 1 ? "occurrences chargées" : "occurrence chargée"}${historyView.next ? " · d’autres plus anciennes disponibles" : ""} · plus récentes en premier`));
    if (!occurrences.length) content.append(node("p", "routines-note", "Aucune occurrence."));
    for (const o of occurrences) {
      const row = node("article", "routines-occurrence"); row.dataset.status = o.status.status; row.dataset.occurrenceId = o.occurrence_id;
      row.tabIndex = -1; row.dataset.historyFocus = `occurrence-${o.occurrence_id}`;
      const labels = { pending: "En attente", running: "En cours / en attente d’une réponse", done: "Terminée", missed: "Manquée", failed: "Échec / tâche inachevée" };
      row.append(node("strong", "", labels[o.status.status] || o.status.status), node("p", "routines-note", o.trigger?.kind === "webhook" ? `${o.trigger.label} · reçu le ${date(o.trigger.received_at, r.time_zone)} · ${r.time_zone}` : `${date(o.scheduled_at_utc, r.time_zone)} · ${r.time_zone} · ${o.manual_nonce ? "Manuel" : "Planifié"} · version ${o.schedule_version}`));
      if (o.status.reason) row.append(node("p", "routines-error", frenchError({ message: o.status.reason }, true)));
      if (o.conversation_id) {
        const result = button("Voir le résultat de cette exécution", () => openThread(o.conversation_id, o.message_id));
        result.dataset.historyFocus = `result-${o.occurrence_id}`; row.append(result);
      }
      content.append(row);
    }
    pageControls(content, r, "occurrences");
    const focus = [...content.querySelectorAll("[data-history-focus]")].find((el) => el.dataset.historyFocus === focusKey) || [...content.querySelectorAll("button")].find((el) => focusText && el.textContent === focusText);
    focus?.focus({ preventScroll: true }); historyView.focusTarget = null;
    restoreHistoryPosition(position);
  }
  async function runNow(r) {
    if (busy) return;
    let job = launches.get(r.routine_id);
    if (job && Date.now() < job.until) return;
    const token = revision; let confirmed = false;
    busy = true; signature = ""; renderList(); message("Vérification du dernier lancement…");
    try {
      const h = await readHistory(r), own = job && h.occurrences.find((o) => o.manual_nonce === job.nonce || o.occurrence_id === job.occurrenceId);
      if (own && job.unknown) {
        // This click was a retry, not permission to run again after a lost reply.
        job.unknown = false; job.until = Date.now() + 3000;
        if (active(token)) message("Le lancement précédent est confirmé dans l’historique.");
        return;
      }
      if (h.running) { if (active(token)) message("Un lancement est déjà en attente ou en cours. Consultez son historique."); return; }
      if (own || (job && !job.unknown)) { launches.delete(r.routine_id); job = null; } // A known terminal run must not swallow a new click.
      if (!job) { job = { nonce: crypto.randomUUID(), until: 0, unknown: true }; launches.set(r.routine_id, job); }
      job.unknown = true; saveLaunches();
      const reply = await panels.api("/api/routines/run-now", { ...idBody(r), nonce: job.nonce });
      job.occurrenceId = reply.occurrence.occurrence_id;
      confirmed = true; job.unknown = false; job.until = Date.now() + 3000;
      if (active(token)) message("Lancement demandé. Son résultat apparaîtra dans l’historique.");
      await readHistory(r);
      try { await panels.refresh(); } catch { /* The page-session nonce remains available for retry. */ }
    } catch (error) {
      if (active(token)) {
        const text = confirmed ? `Lancement enregistré. Son historique est temporairement indisponible : ${frenchError(error)} Utilisez « Historique » ci-dessous pour réessayer.` : launches.get(r.routine_id)?.unknown ? `Lancement non confirmé : ${frenchError(error)} Le même identifiant sera réutilisé pour éviter un doublon. Utilisez « Vérifier / réessayer le lancement » ci-dessous.` : `Lancement non envoyé : ${frenchError(error)} Utilisez « Lancer maintenant » ci-dessous pour réessayer.`;
        message(text, true); status.dataset.launchResult = "true";
      }
    }
    finally { busy = false; saveLaunches(); if (active(token)) { signature = ""; renderList(); kick(); } }
  }
  async function pollHistory() {
    if (!dialog?.open || busy || reloading || historyLoading || Date.now() - historyAt < 3000) return;
    const candidates = new Map(items().map((i) => i.routine).filter(tracked).map((r) => [r.routine_id, r]));
    if (view === "history" && current) candidates.set(current.routine_id, current);
    if (!candidates.size) return;
    const token = revision; historyAt = Date.now(); historyLoading = token;
    try {
      // Never let one approval-blocked run starve other routines. A failure of
      // one history read must not prevent the remaining routines from updating.
      for (const r of candidates.values()) {
        if (!active(token) || busy || reloading) break;
        try {
          if (view === "history" && current.routine_id === r.routine_id) await refreshLoadedPages(r, token);
          else { await readHistory(r); settleLaunch(r); }
        } catch (error) {
          if (active(token) && view === "history" && current.routine_id === r.routine_id) {
            historyView.failures.poll = frenchError(error); renderHistory(r);
          } else if (active(token) && view !== "edit" && status.dataset.launchResult !== "true") message(`Historique temporairement indisponible : ${frenchError(error)}`, true, reload);
        }
      }
      if (active(token)) { signature = ""; renderList(); }
    } finally { if (historyLoading === token) historyLoading = false; }
  }
  panels.register({
    id: "routines",
    async openLink({ id, agent }) {
      if (busy) return false;
      const routine = (panels.state.routines?.items || []).find((item) => item.routine.routine_id === id && item.routine.bot_id === agent)?.routine;
      if (!routine) return false;
      open(agent);
      await edit(routine); // Read/editor only; never run, resume or confirm.
      return true;
    },
    settingsSection(container) {
      const section = node("section", "setting-section");
      section.append(node("h3", "", "Routines"), button("Gérer les routines par bot", () => open(liveBots()[0]?.id))); container.append(section);
    },
    botSection(bot, container) {
      const section = node("section", "routines-section"); section.append(node("h3", "", "Routines"));
      if (bot?.id && !bot.deleted) section.append(button("Gérer les routines de ce bot", () => open(bot.id)));
      else section.append(node("p", "routines-note", "Enregistrez le bot avant de créer ses routines."));
      container.append(section);
    },
    onConnection(online) { hostOnline = online; renderNotices(); },
    onState(_state, context) {
      if (!dialog?.open) return;
      hostOnline = context?.connected ?? panels.connected ?? hostOnline;
      const bot = liveBots().find((b) => b.id === botId());
      if (!bot) { dialog.close(); panels.notify("Ce bot a été supprimé ; ses routines ne sont plus modifiables ici."); return; }
      if (!busy && uncertainResume && view === "edit" && current?.routine_id === uncertainResume) {
        const observed = items().find((i) => i.routine.routine_id === uncertainResume);
        if (observed) {
          uncertainResume = null;
          if (observed.routine.enabled) {
            revision++; view = "list"; current = null; signature = "";
            message("La routine est active. État récupéré automatiquement après reconnexion.");
          } else {
            message("L’activation n’a pas été enregistrée : la routine est toujours en pause. Relisez-la puis réessayez.", true, () => reviewResume(current), "Relire la routine");
          }
        }
      }
      renderNotices(); renderList(); void pollHistory();
    },
    wantsPolling,
  });
})();
