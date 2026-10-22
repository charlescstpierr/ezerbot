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
  const button = (text, cls, action) => {
    const el = node("button", cls, text);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  };
  const count = (n) => new Intl.NumberFormat("fr-FR").format(n);
  const percent = (n) => new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 }).format(n);
  const date = (n) => new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(n * 1000));
  const labels = { chat: "Chat", routine: "Routines", group: "Groupes", delegation: "Délégation", bot_to_bot: "Entre bots" };
  const sourceRank = (source) => { const rank = Object.keys(labels).indexOf(source); return rank < 0 ? Infinity : rank; };
  const missingBot = "Bot supprimé";
  let dialog, section, quotaArea, localArea, notice, refresh, warning, warningText;
  let generalTab, usageTab, quotaState, localData, view = "bot", loading = false, queued = false, queuedForce = false, request = 0;
  let providerKey, quotaPoll, quotaShown, localShown;
  const isUsageOpen = () => dialog?.open && dialog.classList.contains("usage-focus");

  function chooseUsage(active) {
    dialog.classList.toggle("usage-focus", active);
    generalTab.setAttribute("aria-pressed", String(!active));
    usageTab.setAttribute("aria-pressed", String(active));
    dialog.scrollTop = 0;
    if (active) void load(false);
  }
  function openUsage() {
    if (!dialog.open) dialog.showModal();
    chooseUsage(true);
  }
  function accountSettings() {
    chooseUsage(false);
    document.getElementById("provider-section").scrollIntoView({ block: "start" });
  }
  function windows(data) {
    return [data?.quota?.primary, data?.quota?.secondary].filter(Boolean);
  }
  function windowName(w) {
    if (w.limit_window_seconds === 604800) return "Quota hebdomadaire";
    const hours = w.limit_window_seconds / 3600;
    const [amount, unit] = hours >= 24 ? [hours / 24, "jour"] : [hours, "heure"];
    return `Quota sur ${percent(amount)} ${unit}${amount < 2 ? "" : "s"}`;
  }
  function renderQuota(data) {
    quotaState = data;
    if (!quotaArea) return;
    clearTimeout(quotaPoll);
    if (data?.status === "loading" || data?.refreshing) quotaPoll = setTimeout(() => { void load(false); }, 800);
    // An unchanged reading keeps its nodes: repainting identical content makes the panel flash.
    const shown = JSON.stringify(data ?? null);
    if (shown === quotaShown) return;
    quotaShown = shown;
    quotaArea.replaceChildren();
    const ready = data?.status === "ready";
    const high = ready && windows(data).filter((w) => w.used_percent >= 80).sort((a, b) => b.used_percent - a.used_percent)[0];
    warning.hidden = !high;
    if (high) warningText.textContent = `${percent(high.used_percent)} % du ${windowName(high).toLowerCase()} Codex utilisés. ${percent(high.remaining_percent)} % restants.`;
    if (!ready) {
      const text = {
        signin: "Connectez votre compte ChatGPT dans Ezerbot pour lire votre quota Codex.",
        unsupported: "Le quota de votre abonnement est indisponible. Reconnectez votre compte ChatGPT.",
        loading: "Lecture du quota Codex…",
      }[data?.status] || data?.message || "Quota Codex indisponible.";
      const empty = node("div", "usage-empty");
      empty.append(node("strong", "", data?.status === "loading" ? "Un instant…" : "Quota non disponible"), node("p", "", text));
      if (data?.status === "signin" || data?.status === "unsupported" || data?.kind === "signin") empty.append(button("Réglages du compte", "secondary-button", accountSettings));
      quotaArea.append(empty);
      return;
    }
    const grid = node("div", "usage-quota-grid");
    for (const w of windows(data)) {
      const card = node("article", `usage-quota-card${w.used_percent >= 80 ? " usage-high" : ""}`);
      card.append(node("h4", "", windowName(w)));
      const amount = node("div", "usage-remaining");
      amount.append(node("strong", "", `${percent(w.remaining_percent)} %`), node("span", "", "restants"));
      const meter = node("progress", "usage-meter");
      meter.max = 100; meter.value = w.used_percent;
      meter.setAttribute("aria-label", `${windowName(w)} utilisé`);
      const reset = node("p", "usage-reset", w.reset_at ? `Réinitialisation le ${date(w.reset_at)}` : "Date de réinitialisation non fournie");
      if (w.reset_at) reset.title = new Date(w.reset_at * 1000).toString();
      card.append(amount, meter, node("span", "usage-used", `${percent(w.used_percent)} % utilisés`), reset);
      grid.append(card);
    }
    quotaArea.append(grid);
    if (high) {
      const alert = node("p", "usage-alert", `Seuil de 80 % atteint — ${percent(high.used_percent)} % du ${windowName(high).toLowerCase()} utilisés. Les autres applications Codex partagent ce forfait.`);
      alert.setAttribute("role", "status");
      quotaArea.append(alert);
    }
    quotaArea.append(node("p", "usage-sync", `Forfait ${data.quota.plan || "Codex"} · Lu chez OpenAI le ${date(data.quota.checked_at)}${data.refreshing ? " · Actualisation…" : ""}. Heures locales.`));
  }
  function renderLocal() {
    if (!localArea || !localData) return;
    const shown = JSON.stringify([localData, view]);
    if (shown === localShown) return;
    localShown = shown;
    localArea.replaceChildren();
    const total = localData.total;
    const heading = node("div", "usage-local-heading");
    const title = node("div");
    title.append(node("h3", "", "Tokens dans Ezerbot"), node("p", "", "Historique local · Ezerbot"));
    heading.append(title, node("strong", "usage-total", `${count(total.input_tokens + total.output_tokens)} tokens`));
    const tabs = node("div", "usage-breakdown-tabs");
    tabs.setAttribute("role", "group"); tabs.setAttribute("aria-label", "Ventilation des tokens");
    for (const [id, label] of [["bot", "Par bot"], ["source", "Par source"]]) {
      const tab = button(label, "usage-tab", () => {
        view = id; renderLocal();
        localArea.querySelector(`[data-view="${id}"]`).focus();
      });
      tab.dataset.view = id; tab.setAttribute("aria-pressed", String(view === id)); tabs.append(tab);
    }
    const table = node("table", "usage-table");
    const caption = node("caption", "usage-sr-only", `Tokens ${view === "bot" ? "par bot" : "par source"}`);
    const head = node("thead"), header = node("tr");
    for (const label of [view === "bot" ? "Bot" : "Source", "Entrée", "Sortie", "Total"]) {
      const th = node("th", "", label); th.scope = "col"; header.append(th);
    }
    head.append(header);
    const body = node("tbody");
    const rows = view === "bot" ? localData.by_bot : [...localData.by_source].sort((a, b) => sourceRank(a.source) - sourceRank(b.source));
    for (const row of rows) {
      const t = row.tokens, tr = node("tr"), label = node("th"); label.scope = "row";
      label.append(node("span", "", view === "bot" ? `${row.name}${row.deleted && row.name !== missingBot ? " (supprimé)" : ""}` : labels[row.source] || row.source));
      if (t.unknown_runs) label.append(node("small", "", `${count(t.unknown_runs)} exécution(s) sans mesure`));
      tr.append(label);
      for (const n of [t.input_tokens, t.output_tokens, t.input_tokens + t.output_tokens]) tr.append(node("td", "", !t.measured_runs && t.unknown_runs ? "—" : count(n)));
      body.append(tr);
    }
    table.append(caption, head, body);
    localArea.append(heading, tabs);
    if (!rows.length) localArea.append(node("p", "usage-empty", "Aucune exécution enregistrée. Les tokens apparaîtront après une réponse du bot."));
    else localArea.append(table);
    localArea.append(node("p", "usage-sync", `${count(total.cached_input_tokens)} tokens d’entrée en cache, déjà inclus. ${count(total.measured_runs)} exécution(s) mesurée(s).`));
    if (total.unknown_runs) localArea.append(node("p", "usage-unknown", `${count(total.unknown_runs)} exécution(s) sans télémétrie : total partiel, jamais estimé.`));
    localArea.append(node("p", "usage-footnote", "Réponses principales du moteur, depuis le début de l’historique local. Hors titres, mémoire automatique, vision et résumés. Ces tokens ne permettent pas de calculer le quota du forfait."));
  }
  async function load(force) {
    if (!section) return;
    if (loading) {
      // Refresh stays pressable: the request waits for the read in flight, then runs once.
      queued = true; queuedForce ||= force;
      return;
    }
    const revision = ++request;
    loading = true; notice.textContent = "";
    section.setAttribute("aria-busy", "true");
    try {
      const data = await panels.api(force ? "/api/usage/refresh" : "/api/usage", force ? {} : undefined);
      if (revision !== request) return;
      localData = data.local; renderQuota(data.codex); renderLocal();
    } catch (error) {
      if (revision === request) { notice.textContent = error.message; notice.setAttribute("role", "alert"); }
    } finally {
      loading = false; section.setAttribute("aria-busy", "false");
      if (queued) {
        const again = queuedForce;
        queued = queuedForce = false;
        void load(again);
      }
    }
  }
  // Background read of the usage alone; it neither disables the panel nor repaints the workspace.
  async function poll() {
    if (document.visibilityState === "hidden" || panels.busy || loading) return;
    const revision = request;
    try {
      const data = await panels.api("/api/usage");
      if (revision !== request) return;
      localData = data.local; renderQuota(data.codex); renderLocal();
    } catch { /* The next tick retries; only an explicit load reports the failure. */ }
  }
  panels.register({
    id: "usage",
    onState(state) {
      const key = `${state.provider?.backend}:${state.provider?.connected}:${state.provider?.email || ""}`;
      if (providerKey !== undefined && key !== providerKey) {
        // Invalidate an in-flight response on sign-out or backend switch.
        request++; localData = null; localShown = undefined; localArea?.replaceChildren();
        if (isUsageOpen()) void load(false);
      }
      providerKey = key;
      if (state.usage?.codex) renderQuota(state.usage.codex);
    },
    onSettingsLink(id) {
      chooseUsage(id === "usage");
    },
    settingsSection(container) {
      section = container; section.className = "setting-section usage-section";
      dialog = document.getElementById("settings-dialog");
      const nav = node("nav", "usage-settings-nav"); nav.setAttribute("aria-label", "Sections des paramètres");
      generalTab = button("Général", "usage-tab", () => chooseUsage(false)); generalTab.setAttribute("aria-pressed", "true");
      usageTab = button("Usage", "usage-tab", () => chooseUsage(true)); usageTab.setAttribute("aria-pressed", "false");
      nav.append(generalTab, usageTab); dialog.querySelector(".dialog-header").after(nav);
      const heading = node("div", "usage-heading");
      const title = node("div"); title.append(node("h3", "", "Usage et quotas"), node("p", "", "Votre forfait ChatGPT, sans surcoût Ezerbot."));
      refresh = button("Actualiser", "secondary-button", () => load(true));
      heading.append(title, refresh);
      quotaArea = node("div", "usage-quota");
      const manage = node("a", "usage-manage", "Gérer le quota de votre abonnement ↗");
      manage.href = "https://chatgpt.com/codex/settings/usage"; manage.target = "_blank"; manage.rel = "noreferrer";
      localArea = node("div", "usage-local");
      notice = node("p", "usage-error"); notice.setAttribute("role", "status");
      section.append(heading, quotaArea, manage, localArea, notice);
      warning = node("div", "usage-global-warning"); warning.hidden = true; warning.setAttribute("role", "status");
      warningText = node("span"); warning.append(warningText, button("Voir l’usage", "", openUsage));
      document.querySelector(".topbar").after(warning);
      renderQuota(quotaState || { status: "loading" });
      new MutationObserver(() => { if (isUsageOpen()) void load(false); }).observe(dialog, { attributes: true, attributeFilter: ["open"] });
      // Keep the threshold warning current even when the app has no active run.
      setInterval(() => { void poll(); }, 60_000);
    },
  });
})();
