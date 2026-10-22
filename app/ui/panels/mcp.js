"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, text, cls) => { const n = document.createElement(tag); if (text) n.textContent = text; if (cls) n.className = cls; return n; };
  const button = (text, fn, cls = "secondary-button") => { const b = node("button", text, cls); b.type = "button"; b.addEventListener("click", fn); return b; };
  const label = (text, input) => { const n = node("label"); n.append(node("span", text), input); return n; };
  const input = (name, placeholder, type = "text") => { const n = node("input"); n.type = type; n.name = name; n.placeholder = placeholder; n.autocomplete = "off"; n.spellcheck = false; return n; };
  let section, list, catalogue, catalogueHeader, feedback, key;
  const catalogueCards = new Map();
  function rememberCard(card) {
    const details = [...card.querySelectorAll("details")];
    const inputs = [...card.querySelectorAll("input")];
    const controls = [...card.querySelectorAll("button, a, summary")];
    const focused = [...inputs, ...controls].find(n => n === document.activeElement);
    return {
      details: details.map(n => n.open),
      fields: inputs.filter(n => n.type !== "checkbox").map(n => ({name:n.name,value:n.value})),
      focus: focused && {name:focused.name,action:focused.getAttribute("data-connector-action"),label:focused.getAttribute("aria-label"),summary:details.findIndex(n => n.querySelector("summary") === focused),start:focused.selectionStart,end:focused.selectionEnd,direction:focused.selectionDirection},
    };
  }
  function restoreCard(card, view) {
    if (!view) return;
    const details = [...card.querySelectorAll("details")];
    details.forEach((n,i) => { if (i < view.details.length) n.open = view.details[i]; });
    const inputs = [...card.querySelectorAll("input")];
    for (const field of view.fields) { const target = inputs.find(n => n.name === field.name); if (target) target.value = field.value; }
    const f = view.focus;
    if (!f) return;
    const controls = [...card.querySelectorAll("button, a, summary")];
    const target = (f.name && inputs.find(n => n.name === f.name))
      || (f.action && controls.find(n => n.getAttribute("data-connector-action") === f.action))
      || (f.label && controls.find(n => n.getAttribute("aria-label") === f.label))
      || (f.summary >= 0 ? details[f.summary]?.querySelector("summary") : undefined);
    target?.focus({preventScroll:true});
    if (target && Number.isInteger(f.start) && Number.isInteger(f.end)) target.setSelectionRange?.(f.start,f.end,f.direction);
  }
  // Réunions : une clé Recall.ai, pas de client OAuth. La clé part au coffre et ne revient jamais.
  const meetingStatus = {joining:"authorizing",in_call:"connected",done:"disconnected",cancelled:"disconnected",failed:"error"};
  function recallCard(item) {
    const card = node("article", null, "mcp-server mcp-catalogue-card");
    card.setAttribute("data-connector", item.id);
    const heading = node("div", null, "mcp-heading");
    const status = node("span", null, `mcp-status mcp-status-${item.status}`);
    const dot = node("span", null, "status-dot"); dot.setAttribute("aria-hidden", "true");
    status.append(dot, document.createTextNode(item.status === "connected" ? "Connecté" : "Déconnecté"));
    heading.append(node("strong", item.name), status);
    card.append(heading, node("p", item.description), node("p", `Autorisations : ${item.permissions}`, "mcp-note"));
    const meeting = item.meeting;
    const live = Boolean(meeting?.active);
    const actions = node("div", null, "mcp-actions");
    if (item.status !== "connected") {
      const setup = node("details"); setup.append(node("summary", "Installation et autorisations détaillées"), node("p", item.setup, "mcp-note"), node("p", `${item.license} · ${item.source}`, "mcp-note"));
      const form = node("form", null, "mcp-form"); form.id = `connector-setup-${item.id}`;
      const field = input(`${item.id}-key`, "Clé API Recall.ai", "password"); field.maxLength = 256;
      form.append(label("Clé API Recall.ai — jamais réaffichée", field));
      const connect = button("Connecter", event => { event?.preventDefault(); return form.requestSubmit(); }, "primary-button");
      connect.type = "submit"; connect.setAttribute("form", form.id); connect.setAttribute("data-connector-action", "connect");
      form.addEventListener("submit", async event => {
        event.preventDefault();
        const body = {key:field.value};
        field.value = "";
        try { await act("/api/recall/connect", body, connect); }
        finally { delete body.key; }
      });
      card.append(setup, form);
      actions.append(connect);
    } else {
      const form = node("form", null, "mcp-form"); form.id = `connector-invite-${item.id}`;
      const link = input(`${item.id}-link`, "https://meet.google.com/…"); link.maxLength = 2048;
      link.setAttribute("aria-label", "Lien de la réunion");
      form.append(label("Lien Google Meet, Zoom ou Microsoft Teams", link), node("p", "Une carte de consentement s’affiche avant chaque invitation. Les participants voient le bot.", "mcp-note"));
      const invite = button("Inviter le bot", event => { event?.preventDefault(); return form.requestSubmit(); }, live ? "secondary-button" : "primary-button");
      invite.type = "submit"; invite.setAttribute("form", form.id); invite.setAttribute("data-connector-action", "invite");
      invite.disabled = live;
      form.addEventListener("submit", async event => {
        event.preventDefault();
        if (live) return;
        await act("/api/recall/join", {link:link.value}, invite);
        // Le lien d'une invitation acceptée ou en attente de carte ne reste pas dans le champ ; un lien refusé reste à corriger.
        if (!feedback.textContent) { const next = catalogueCards.get(item.id)?.node.querySelector("input"); if (next) next.value = ""; }
      });
      card.append(form);
      actions.append(invite);
      const remove = button("Déconnecter", () => act("/api/recall/disconnect", {}, remove)); remove.setAttribute("data-connector-action","disconnect");
      actions.append(remove);
    }
    if (meeting) {
      const line = node("p", null, "mcp-meeting"); line.setAttribute("data-meeting-state", meeting.state);
      const badge = node("span", null, `mcp-status mcp-status-${meetingStatus[meeting.state] || "error"}`);
      const meetingDot = node("span", null, "status-dot"); meetingDot.setAttribute("aria-hidden", "true");
      badge.append(meetingDot, document.createTextNode(meeting.label));
      line.append(node("strong", "Réunion "), badge, document.createTextNode(` · ${meeting.host} · vu par les participants comme « ${meeting.bot_name} »`));
      card.append(line);
      if (meeting.message) card.append(node("p", meeting.message, meeting.state === "failed" ? "mcp-error" : "mcp-note"));
      if (live) {
        const leave = button("Quitter la réunion", () => act("/api/recall/leave", {id:meeting.id}, leave), "primary-button"); leave.setAttribute("data-connector-action","leave");
        actions.append(leave);
      }
    }
    card.append(actions);
    return card;
  }
  // The first request names the service alone; a GitHub account other than the first is named by its slot.
  const slotBody = item => item.slot && item.slot !== item.id ? {id:item.id,account:item.slot} : {id:item.id};
  const accountStatus = {connected:"Connecté",authorizing:"En attente de votre autorisation chez GitHub",reconnect:"À reconnecter",error:"En erreur",saved:"Connexion enregistrée · à démarrer"};
  function renderCatalogue(items, bots) {
    if (!catalogueHeader && items.length) {
      catalogueHeader = node("div", null, "mcp-catalogue-header");
      catalogueHeader.append(node("h4", "Catalogue"), node("p", "Services prêts à configurer. La connexion ouvre une approbation, puis le navigateur. Les jetons restent dans le coffre local ; connecter un service n’autorise pas ses outils.", "mcp-note"));
      catalogue.append(catalogueHeader);
    }
    const ids = new Set(items.map(item => item.id));
    for (const [id, previous] of catalogueCards) { if (!ids.has(id)) { previous.node.remove(); catalogueCards.delete(id); } }
    for (const item of items) {
      const cardKey = JSON.stringify([item,bots.map(b => [b.id,b.name])]);
      const previous = catalogueCards.get(item.id);
      // Do not even detach an unchanged card: detaching an active password field
      // loses browser focus/selection despite keeping the node and its value.
      if (previous?.key === cardKey) continue;
      const view = previous && rememberCard(previous.node);
      if (item.kind === "recall") {
        const recall = recallCard(item);
        if (previous) previous.node.replaceWith(recall); else catalogue.append(recall);
        catalogueCards.set(item.id,{key:cardKey,node:recall});
        restoreCard(recall,view);
        continue;
      }
      const card = node("article", null, "mcp-server mcp-catalogue-card");
      card.setAttribute("data-connector", item.id);
      const heading = node("div", null, "mcp-heading");
      const status = node("span", null, `mcp-status mcp-status-${item.status}`);
      const dot = node("span", null, "status-dot"); dot.setAttribute("aria-hidden", "true");
      status.append(dot, document.createTextNode(({connected:"Connecté",saved:"Connexion enregistrée · à démarrer",authorizing:item.device_flow ? "En attente de votre autorisation chez GitHub" : "Connexion dans le navigateur",reconnect:"À reconnecter",error:"En erreur",disconnected:"Déconnecté"})[item.status] || "Déconnecté"));
      heading.append(node("strong", item.name), status);
      card.append(heading, node("p", item.description), node("p", `Autorisations : ${item.permissions}`, "mcp-note"));
      if (item.account) card.append(node("p", `Compte connecté : ${item.account}`, "mcp-note"));
      if (item.message && !(item.device_flow && item.device)) card.append(node("p", item.message, item.warning ? "mcp-warning" : ["error","reconnect"].includes(item.status) ? "mcp-error" : "mcp-note"));
      const setup = node("details"); setup.append(node("summary", "Installation et autorisations détaillées"), node("p", item.setup, "mcp-note"));
      const copyButton = (caption, value, name = item.name) => {
        let resetCopy;
        const copy = button("Copier", async () => {
          clearTimeout(resetCopy);
          try { await panels.copyText(value); copy.textContent = "Copié"; resetCopy = setTimeout(() => { copy.textContent = "Copier"; },1500); }
          catch { feedback.textContent = "Sélectionnez le texte pour le copier. La copie automatique n’est pas disponible."; feedback.scrollIntoView?.({block:"nearest"}); }
        });
        copy.setAttribute("aria-label", `Copier ${caption.toLowerCase()} — ${name}`);
        return copy;
      };
      const copyable = (caption, value) => {
        setup.append(node("p", caption, "mcp-note"));
        const row = node("div", null, "mcp-copy-row");
        row.append(node("code", value), copyButton(caption, value)); setup.append(row);
      };
      if (item.redirect_uri) copyable("URL de retour OAuth", item.redirect_uri);
      if (item.id === "notion") setup.append(node("p", "Autorisations choisies pendant le consentement Notion.", "mcp-note"));
      else if (item.device_flow) setup.append(node("p", `${item.scopes_label} · aucune application OAuth à créer, aucun secret à saisir.`, "mcp-note"));
      else copyable("Portées OAuth", item.scopes_label || item.scopes);
      setup.append(node("p", `${item.license} · ${item.source}`, "mcp-note"));
      let form, client, secret;
      if (item.needs_client && item.has_tokens) setup.append(node("p", "Pour changer d’application OAuth, déconnectez d’abord le service.", "mcp-note"));
      if (item.needs_client && !item.has_tokens && item.status !== "authorizing") {
        form = node("form", null, "mcp-form"); form.id = `connector-setup-${item.id}`;
        client = input(`${item.id}-client`, "Identifiant de l’application OAuth"); client.maxLength = 512;
        secret = input(`${item.id}-secret`, "Secret de l’application OAuth", "password"); secret.maxLength = 1024;
        client.required = secret.required = !item.has_client;
        form.append(label("Identifiant OAuth", client), label("Secret OAuth — jamais réaffiché", secret));
        if (item.has_client) form.append(node("p", "Laissez les champs vides pour réutiliser la configuration enregistrée.", "mcp-note"));
        form.append(node("p", "Ne saisissez pas le mot de passe de votre compte.", "mcp-note"));
        setup.append(form);
      }
      card.append(setup);
      if (item.device_flow && !item.configured) card.append(node("p", "L’application GitHub d’Ezerbot n’est pas configurée dans cette version. Il n’y a ni identifiant ni secret à saisir ici : Connecter restera indisponible tant qu’elle n’est pas préparée.", "mcp-warning"));
      const deviceBlock = (code, name, action = "authorize") => {
        const device = node("div", null, "mcp-device"); device.setAttribute("role", "group"); device.setAttribute("aria-label", "Code de connexion GitHub");
        const until = new Date(code.expires_at).toLocaleTimeString("fr-FR", {hour:"2-digit",minute:"2-digit"});
        const row = node("div", null, "mcp-copy-row");
        row.append(node("code", code.user_code, "mcp-device-code"), copyButton("le code de connexion", code.user_code, name));
        device.append(node("p", "Entrez ce code sur github.com/login/device :", "mcp-note"), row, node("p", `Le code est valable jusqu’à ${until}. Ezerbot termine la connexion dès que vous avez autorisé l’application chez GitHub.`, "mcp-note"));
        const page = node("a", "Ouvrir github.com/login/device", "secondary-button");
        page.href = code.verification_uri; page.target = "_blank"; page.rel = "noopener noreferrer"; page.setAttribute("data-connector-action", action);
        device.append(page);
        return device;
      };
      if (item.device_flow && item.device) card.append(deviceBlock(item.device, item.name));
      const actions = node("div", null, "mcp-actions");
      const connect = button(item.status === "disconnected" ? "Connecter" : "Reconnecter", event => {
        event?.preventDefault();
        if (form && !item.has_client && !setup.open) { setup.open = true; client.focus?.(); }
        else if (form && setup.open) return form.requestSubmit();
        else return act("/api/connectors/connect", slotBody(item), connect);
      }, ["disconnected","reconnect","error"].includes(item.status) ? "primary-button" : "secondary-button");
      connect.setAttribute("data-connector-action", "connect");
      if (form) {
        // One visible CTA, also the form's default submit button (Enter works).
        connect.type = "submit"; connect.setAttribute("form", form.id);
        form.addEventListener("submit", async event => {
          event.preventDefault();
          const body = {...slotBody(item),client_id:client.value,client_secret:secret.value};
          client.value = ""; secret.value = "";
          try { await act("/api/connectors/connect", body, connect); }
          finally { delete body.client_secret; delete body.client_id; }
        });
      }
      if (item.status !== "authorizing") actions.append(connect);
      if (item.device_flow && item.status === "authorizing") {
        const cancel = button("Annuler", () => act("/api/connectors/cancel", slotBody(item), cancel)); cancel.setAttribute("data-connector-action", "cancel"); actions.append(cancel);
      }
      if (item.authorization_url && item.status === "authorizing" && !item.device) {
        const link = node("a", "Ouvrir la page de connexion", "secondary-button");
        link.href = item.authorization_url; link.target = "_blank"; link.rel = "noopener noreferrer"; link.setAttribute("data-connector-action","authorize");
        actions.append(link);
      }
      if (item.server) {
        const remove = button("Déconnecter", () => act("/api/connectors/disconnect", slotBody(item), remove)); remove.setAttribute("data-connector-action","disconnect");
        if ((item.accounts || []).length > 1 && item.account) remove.setAttribute("aria-label", `Déconnecter ${item.account}`); if (!(item.device_flow && item.status === "authorizing")) actions.append(remove);
        if (item.status === "connected" || item.server.tools.length) {
          const choices = node("fieldset", null, "mcp-bots"); choices.append(node("legend", "Activer les outils pour"));
          for (const bot of bots) {
            const check = input(`catalogue-${item.id}-${bot.id}`, "", "checkbox"); check.checked = item.server.enabled_bots.includes(bot.id);
            check.addEventListener("change", () => act("/api/mcp/select", {id:item.server.id,bot_id:bot.id,enabled:check.checked}, check));
            choices.append(label(bot.name, check));
          }
          card.append(choices);
          if (item.status === "authorizing") card.append(node("p", "Une nouvelle identification réinitialise l’activation par bot.", "mcp-note"));
          const tools = node("details"); tools.append(node("summary", `Outils (${item.server.tools.length})${item.status === "connected" ? "" : " · dernier catalogue connu"}`));
          for (const tool of item.server.tools) tools.append(node("p", `${tool.name} — ${tool.description || "Sans description"}`, "mcp-note"));
          card.append(tools);
        } else {
          card.append(node("p", "Les outils et leur activation par bot seront disponibles après la connexion.", "mcp-note"));
        }
      }
      const others = (item.accounts || []).filter(account => account.id !== item.slot);
      if (item.device_flow && item.configured && item.status === "connected" && !(item.accounts || []).some(account => account.status === "authorizing")) {
        const another = button("Connecter un autre compte", () => act("/api/connectors/connect", {id:item.id,another:true}, another));
        another.setAttribute("data-connector-action", "add-account"); actions.append(another);
      }
      card.append(actions);
      for (const account of others) card.append(accountBlock(item, account, copyButton, deviceBlock, bots));
      if (previous) previous.node.replaceWith(card); else catalogue.append(card);
      catalogueCards.set(item.id,{key:cardKey,node:card});
      restoreCard(card,view);
    }
  }
  // A GitHub account beside the first one: its login, its own code wait, disconnect and bot switches.
  function accountBlock(item, account, copyButton, deviceBlock, bots) {
    const block = node("section", null, "mcp-account"); block.setAttribute("data-account", account.id);
    const who = account.account || "nouveau compte";
    block.append(node("p", account.status === "connected" ? `Compte connecté : ${account.account}` : `Compte GitHub · ${who} · ${accountStatus[account.status] || "Déconnecté"}`, "mcp-note"));
    if (account.message && !account.device) block.append(node("p", account.message, account.warning ? "mcp-warning" : ["error","reconnect"].includes(account.status) ? "mcp-error" : "mcp-note"));
    if (account.device) block.append(deviceBlock(account.device, `${item.name} (${who})`, "authorize-account"));
    const actions = node("div", null, "mcp-actions");
    const target = {id:item.id,account:account.id};
    if (["reconnect","error"].includes(account.status)) {
      const connect = button("Reconnecter", () => act("/api/connectors/connect", target, connect), "primary-button");
      connect.setAttribute("data-connector-action", "reconnect-account"); connect.setAttribute("aria-label", `Reconnecter ${who}`); actions.append(connect);
    }
    if (account.status === "authorizing") {
      const cancel = button("Annuler", () => act("/api/connectors/cancel", target, cancel));
      cancel.setAttribute("data-connector-action", "cancel-account"); cancel.setAttribute("aria-label", `Annuler la connexion de ${who}`); actions.append(cancel);
    } else {
      const remove = button("Déconnecter", () => act("/api/connectors/disconnect", target, remove));
      remove.setAttribute("data-connector-action", "disconnect-account"); remove.setAttribute("aria-label", `Déconnecter ${who}`); actions.append(remove);
    }
    block.append(actions);
    if (account.status === "connected" && account.server) {
      const choices = node("fieldset", null, "mcp-bots"); choices.append(node("legend", `Activer les outils pour (${who})`));
      for (const bot of bots) {
        const check = input(`catalogue-${item.id}-${account.id}-${bot.id}`, "", "checkbox"); check.checked = account.server.enabled_bots.includes(bot.id);
        check.addEventListener("change", () => act("/api/mcp/select", {id:account.server.id,bot_id:bot.id,enabled:check.checked}, check));
        choices.append(label(bot.name, check));
      }
      block.append(choices);
    }
    return block;
  }
  async function act(path, body, b) {
    b.disabled = true;
    feedback.textContent = "";
    feedback.className = "mcp-error";
    try {
      const result = await panels.api(path, body);
      await panels.refresh();
      if (result.conversation_id) {
        document.getElementById("settings-dialog").close();
        panels.openConversation(result.conversation_id);
      }
    } catch (e) {
      feedback.textContent = e.message || "L’action du connecteur a échoué.";
      // Restore checkboxes from confirmed state after a refusal or lost reply.
      key = undefined;
      for (const cached of catalogueCards.values()) { if (cached.node.contains(b)) cached.key = null; }
      await panels.refresh().catch(() => {});
      render(panels.state);
      feedback.scrollIntoView?.({block:"nearest"});
    }
    finally { b.disabled = false; }
  }
  function render(state) {
    if (!list) return;
    const items = state.mcp?.catalogue || [];
    const catalogueIds = new Set(items.flatMap(i => [i.server?.id, ...(i.accounts || []).map(a => a.server?.id), ...(i.slot_servers || [])]).filter(Boolean));
    const servers = (state.mcp?.servers || []).filter(s => !catalogueIds.has(s.id));
    const bots = (state.bots || []).filter(b => !b.deleted);
    // Catalogue updates are keyed independently of the custom-server list.
    // Keep existing drafts if an unavailable vault temporarily omits catalogue.
    if (Array.isArray(state.mcp?.catalogue)) renderCatalogue(items, bots);
    const next = JSON.stringify([servers, bots.map(b => [b.id,b.name]),state.mcp?.error]);
    if (next === key) return;
    key = next;
    list.replaceChildren();
    if (state.mcp?.error) list.append(node("p", state.mcp.error, "mcp-error"));
    if (!servers.length) list.append(node("p", "Aucun connecteur ajouté.", "mcp-note"));
    for (const server of servers) {
      const card = node("article", null, "mcp-server");
      const heading = node("div", null, "mcp-heading");
      const status = node("span", null, `mcp-status mcp-status-${server.status}`);
      const dot = node("span", null, "status-dot"); dot.setAttribute("aria-hidden", "true");
      status.append(dot, document.createTextNode(({connected:"Connecté",busy:"Occupé",starting:"Démarrage en cours",disconnected:"Déconnecté",crashed:"Serveur arrêté",error:"En erreur"})[server.status] || "État inconnu"));
      heading.append(node("strong", server.name), status);
      card.append(heading);
      const config = node("details"); config.append(node("summary", "Configuration locale"));
      config.append(node("pre", [server.command, ...server.arguments].join("\n")));
      config.append(node("p", server.environment_names.length ? `Variables privées : ${server.environment_names.join(", ")} · valeurs masquées` : "Aucune variable privée.", "mcp-note"));
      card.append(config);
      if (server.error) card.append(node("p", server.error, "mcp-error"));
      const choices = node("fieldset", null, "mcp-bots"); choices.append(node("legend", "Activer les outils pour"));
      for (const bot of bots) {
        const check = input(`mcp-${server.id}-${bot.id}`, "", "checkbox");
        check.checked = server.enabled_bots.includes(bot.id);
        check.addEventListener("change", () => act("/api/mcp/select", {id:server.id,bot_id:bot.id,enabled:check.checked}, check));
        choices.append(label(bot.name,check));
      }
      card.append(choices);
      const tools = node("details"); tools.append(node("summary", `Outils (${server.tools.length})${["connected","busy"].includes(server.status) ? "" : " · dernier catalogue connu"}`));
      if (!server.tools.length) tools.append(node("p", "Démarrez le connecteur pour découvrir ses outils.", "mcp-note"));
      for (const tool of server.tools) { const item = node("p"); item.append(node("strong",tool.name),node("span",` — ${tool.description || "Sans description"}`)); tools.append(item); }
      card.append(tools);
      const actions = node("div", null, "mcp-actions");
      const restart = button(["connected","busy","starting"].includes(server.status) ? "Redémarrer…" : "Démarrer…", () => act("/api/mcp/restart",{id:server.id},restart));
      const remove = button("Supprimer…", () => act("/api/mcp/remove",{id:server.id},remove));
      actions.append(restart,remove); card.append(actions); list.append(card);
    }
  }
  panels.register({
    id: "mcp",
    settingsSection(container) {
      section = container; section.className = "setting-section mcp-section";
      section.append(node("h3", "Connecteurs"),node("p", "Choisissez un service du catalogue ou ajoutez votre propre serveur MCP. Les actions de gestion sont regroupées dans la conversation « Approbations ». « Toujours » peut autoriser un outil de ce serveur pour un bot."),node("p", "La désactivation retire les outils de ce bot sans arrêter le serveur partagé.", "mcp-note"));
      catalogueCards.clear(); catalogueHeader = undefined;
      catalogue = node("div", null, "mcp-catalogue mcp-list"); section.append(catalogue);
      section.append(node("h4", "Serveurs personnalisés"));
      list = node("div", null, "mcp-list"); section.append(list);
      const details = node("details", null, "mcp-add"); details.append(node("summary", "Ajouter un serveur MCP"));
      const form = node("form", null, "mcp-form");
      const name = input("name", "Par exemple : Atelier local"); name.required = true; name.maxLength = 60;
      const command = input("command", "/chemin/vers/le/serveur"); command.required = true;
      const args = node("textarea"); args.name = "arguments"; args.rows = 3; args.placeholder = "/chemin/serveur.py\n--option"; args.spellcheck = false;
      form.append(label("Nom du connecteur",name),label("Commande (sans interpréteur de shell)",command),label("Arguments — un par ligne",args));
      const env = node("div",null,"mcp-environment");
      const addEnv = () => {
        const row = node("div",null,"mcp-env-row");
        const k = input("env-name", "CLE_API"); k.setAttribute("aria-label","Nom de la variable privée");
        const v = input("env-value", "Valeur privée", "password"); v.setAttribute("aria-label","Valeur privée de la variable");
        row.append(k,v,button("Retirer",()=> {v.value="";row.remove();})); env.append(row);
      };
      form.append(node("p","Variables d’environnement privées : conservées sur ce Mac (trousseau macOS ou fichier local, selon Paramètres), jamais réaffichées. Le serveur reçoit uniquement ces variables et un environnement système minimal. Les valeurs de 8 caractères ou plus sont masquées dans tous les résultats, même si ce connecteur est inactif. Pour les valeurs plus courtes, seuls les champs texte identiques de ce serveur sont masqués ; les nombres et booléens restent inchangés.","mcp-note"),env,button("Ajouter une variable privée",addEnv));
      const submit = node("button","Enregistrer le connecteur","primary-button"); submit.type = "submit";
      form.append(submit); details.append(form); section.append(details);
      feedback = node("p",null,"mcp-error"); feedback.setAttribute("role","status"); section.append(feedback);
      form.addEventListener("submit",async event => {
        event.preventDefault(); submit.disabled=true; feedback.textContent=""; feedback.className="mcp-error";
        const environment = Object.create(null);
        let invalid = false;
        for (const row of env.children) {
          const [k,v] = row.querySelectorAll("input");
          if (!k.value.trim() || Object.hasOwn(environment,k.value.trim())) invalid=true;
          environment[k.value.trim()] = v.value;
        }
        if (invalid) { feedback.textContent="Chaque variable privée doit avoir un nom unique et non vide."; submit.disabled=false; return; }
        try {
          await panels.api("/api/mcp/add",{name:name.value,command:command.value,arguments:args.value.replace(/(\r?\n)+$/, "") ? args.value.replace(/(\r?\n)+$/, "").split(/\r?\n/) : [],environment});
          form.reset(); env.replaceChildren(); details.open=false;
          await panels.refresh();
          feedback.className="mcp-note";
          feedback.textContent="Connecteur enregistré. Activez-le pour un bot, puis démarrez-le.";
        } catch (e) { feedback.textContent=e.message || "Le connecteur n’a pas pu être enregistré."; }
        finally { for (const k of Object.keys(environment)) delete environment[k]; for (const v of env.querySelectorAll('input[type="password"]')) v.value=""; submit.disabled=false; }
      });
      render(panels.state);
    },
    onState: render,
    wantsPolling({state}) { return document.getElementById("settings-dialog")?.open && Boolean(state.mcp?.servers?.length || state.mcp?.catalogue?.length); },
  });
})();
