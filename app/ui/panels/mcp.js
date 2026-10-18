"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, text, cls) => { const n = document.createElement(tag); if (text) n.textContent = text; if (cls) n.className = cls; return n; };
  const button = (text, fn, cls = "secondary-button") => { const b = node("button", text, cls); b.type = "button"; b.addEventListener("click", fn); return b; };
  const label = (text, input) => { const n = node("label"); n.append(node("span", text), input); return n; };
  const input = (name, placeholder, type = "text") => { const n = node("input"); n.type = type; n.name = name; n.placeholder = placeholder; n.autocomplete = "off"; n.spellcheck = false; return n; };
  let section, list, feedback, key;
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
      await panels.refresh().catch(() => {});
      render(panels.state);
    }
    finally { b.disabled = false; }
  }
  function render(state) {
    if (!list) return;
    const servers = state.mcp?.servers || [];
    const bots = (state.bots || []).filter(b => !b.deleted);
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
      section.append(node("h3", "Connecteurs"),node("p", "Ajoutez un serveur MCP local (stdio). Démarrage et appels exigent votre accord. Les actions de gestion sont regroupées dans la conversation « Approbations ». « Toujours » peut autoriser un outil de ce serveur pour un bot."),node("p", "La désactivation retire les outils de ce bot sans arrêter le serveur partagé.", "mcp-note"));
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
    wantsPolling({state}) { return document.getElementById("settings-dialog")?.open && Boolean(state.mcp?.servers?.length); },
  });
})();
