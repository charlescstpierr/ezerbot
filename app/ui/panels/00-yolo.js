"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let settings, key = "", error = "", saving = false;
  const SHOWN = 20;
  const node = (tag, text, cls) => { const e = document.createElement(tag); if (text) e.textContent = text; if (cls) e.className = cls; return e; };
  const when = (at) => { try { return new Date(at).toLocaleString("fr-CA", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }); } catch { return ""; } };
  async function save(path, body) {
    if (saving) return;
    saving = true; error = "";
    try { await panels.api(path, body); await panels.refresh(); }
    catch (e) { error = e.message || "Modification non confirmée."; }
    finally { saving = false; key = ""; render(panels.state); }
  }
  function modeButton(mode, current, title, detail) {
    const button = node("button"); button.type = "button"; button.dataset.yoloMode = mode;
    button.setAttribute("aria-pressed", String(current === mode));
    button.append(node("strong", title), node("span", detail));
    button.addEventListener("click", () => (current === mode ? undefined : save("/api/yolo/mode", { mode })));
    return button;
  }
  function render(state) {
    if (!settings) return;
    const yolo = state.yolo;
    const next = JSON.stringify([yolo, state.bots?.map(b => [b.id, b.name, b.deleted]), error]);
    if (next === key) return;
    key = next;
    settings.className = "setting-section yolo-settings";
    settings.replaceChildren(node("h3", "Mode YOLO (tous les droits)"));
    if (!yolo || yolo.error) { settings.append(node("p", yolo?.error || "Mode YOLO indisponible.")); return; }
    const on = yolo.mode === "yolo";
    settings.append(node("p", on
      ? "Activé : tous les outils sont ouverts (terminal et fichiers dans votre dossier personnel, connecteurs, écran du Mac, documents, délégation, routines, mémoire) et les approbations partent seules. Chaque action reste visible dans le fil (« Fait automatiquement ») et dans le journal ci-dessous."
      : "Désactivé : chaque action protégée affiche une carte d’approbation, et les outils puissants suivent leurs réglages un à un. Le Mode YOLO les ouvre tous d’un seul clic."));
    const modes = node("div", null, "yolo-modes"); modes.setAttribute("role", "group"); modes.setAttribute("aria-label", "Mode d’autorisation");
    modes.append(
      modeButton("yolo", yolo.mode, "Mode YOLO (tous les droits)", "Rien ne demande de carte."),
      modeButton("ask", yolo.mode, "Demander avant d’agir", "Une carte avant chaque action protégée."),
    );
    settings.append(modes);
    const problem = node("p", error, "yolo-error"); problem.setAttribute("role", "alert"); problem.hidden = !error;
    settings.append(problem);

    settings.append(node("h4", "Garde-fous qui restent toujours"));
    const guards = node("ul", null, "yolo-guards");
    for (const [title, detail] of [
      ["Les données d’Ezerbot restent illisibles.", " Son dossier de données, son coffre, ses jetons et le trousseau sont refusés aux outils, même en Mode YOLO."],
      ["macOS demande ses propres permissions.", " Accessibilité, Enregistrement de l’écran et dossiers protégés : la première fois, c’est le système qui vous interroge."],
      ["Échap et Arrêter arrêtent toujours le bot,", " immédiatement, y compris une commande en cours ou une tâche de fond."],
      ["Quelques cartes restent des cartes :", " brouillons d’envoi à relire (courriel, message, commentaire), propositions de skill, connexion ou retrait d’un connecteur, consentement Recall, publication de webhooks et agent de routines. Le contrôle de l’écran exige toujours une session démarrée par vous, et un « Toujours refuser » que vous avez écrit reste respecté."],
    ]) { const li = node("li"); li.append(node("strong", title), document.createTextNode(detail)); guards.append(li); }
    settings.append(guards);

    const bots = (state.bots || []).filter(b => !b.deleted);
    if (bots.length) {
      settings.append(node("h4", "Par bot"), node("p", "Un bot peut suivre le réglage global, toujours agir sans demander ou toujours demander."));
      for (const bot of bots) {
        const current = yolo.bots?.find(b => b.bot_id === bot.id) || { mode: "inherit", effective: yolo.mode };
        const row = node("div", null, "yolo-bot-row"); row.append(node("strong", bot.name));
        const select = node("select"); select.name = `yolo-${bot.id}`; select.setAttribute("aria-label", `Mode d’autorisation de ${bot.name}`);
        for (const [value, label] of [["inherit", "Suivre le réglage global"], ["yolo", "Mode YOLO"], ["ask", "Demander avant d’agir"]]) {
          const option = node("option", label); option.value = value; option.selected = value === current.mode; select.append(option);
        }
        select.value = current.mode;
        select.addEventListener("change", () => save("/api/yolo/bot", { bot_id: bot.id, mode: select.value }));
        const badge = node("span", current.effective === "yolo" ? "Sans demander" : "Avec cartes", "yolo-effective"); badge.dataset.mode = current.effective;
        row.append(select, badge); settings.append(row);
      }
    }

    settings.append(node("h4", "Journal d’activité"));
    const log = yolo.log || [];
    if (!log.length) settings.append(node("p", "Aucune action faite automatiquement pour l’instant."));
    else {
      const list = node("ol", null, "yolo-log");
      for (const entry of log.slice(0, SHOWN)) {
        const bot = bots.find(b => b.id === entry.bot_id)?.name || entry.bot_id;
        const item = node("li"); item.dataset.tool = entry.tool;
        const head = node("span", `${when(entry.at)} · ${bot} · ${entry.source === "bot" ? "réglage du bot" : "réglage global"}`);
        item.append(head, node("code", entry.summary)); list.append(item);
      }
      settings.append(list);
      if (log.length > SHOWN) settings.append(node("p", `Les ${SHOWN} dernières actions sur ${log.length} gardées.`));
    }
  }
  panels.register({
    id: "yolo",
    settingsSection(container) { settings = container; render(panels.state); },
    onState(state) { render(state); },
  });
})();
