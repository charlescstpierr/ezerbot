"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const kinds = new Set(["form", "approval", "secret"]);
  const activity = new Map();
  const changed = () => window.dispatchEvent(new Event("grokbot:forms-change"));
  // Only non-sensitive delivery metadata lives here. Never cache values, arguments,
  // secrets or request bodies in storage, logs, notifications or the transcript.
  window.GrokbotForms = { view: (id) => activity.get(id) };

  function observe(request) {
    if (!request?.entry_id) return;
    const previous = activity.get(request.entry_id);
    const receipt = request.receipt;
    // Overlapping refreshes must not turn a proven delivery back into “pending”.
    if (previous?.receipt?.wake?.status === "delivered" && receipt?.wake?.status === "pending"
        && previous.receipt.resolution_id === receipt.resolution_id) return;
    if ((receipt && JSON.stringify(previous?.receipt) !== JSON.stringify(receipt))
        || (previous?.error && request.status?.status !== "pending")) {
      activity.set(request.entry_id, { ...previous, ...(receipt ? { receipt } : {}), error: null });
      changed();
    }
  }

  // Standing permissions are reversible (REA goal-permissions-secrets.md PS03).
  // Removing one affects future reviews, not an action already admitted.
  let settings;
  let rulesKey;
  let expiryTimer;
  const pendingApproval = (request) => request.status?.status === "pending"
    && request.kind?.kind === "approval" && Number.isFinite(request.kind.expires_at);
  function scheduleExpiry(state) {
    window.clearTimeout(expiryTimer);
    const deadlines = (state.forms?.requests || []).filter(pendingApproval).map((request) => request.kind.expires_at);
    if (!deadlines.length) return;
    // No polling while a person is thinking. A one-shot check after the next
    // approval deadline also works when no other shell refresh is scheduled.
    const delay = Math.min(2147483647, Math.max(1000, Math.min(...deadlines) - Date.now() + 1000));
    expiryTimer = window.setTimeout(() => { panels.refresh().catch(() => {}); }, delay);
  }
  function renderRules(state) {
    if (!settings) return;
    const rules = state.forms?.rules || [];
    const key = JSON.stringify([rules, state.bots?.map((bot) => [bot.id, bot.name]), Boolean(state.forms?.error)]);
    if (key === rulesKey) return;
    rulesKey = key;
    settings.replaceChildren();
    const heading = document.createElement("h3");
    heading.textContent = "Autorisations permanentes";
    settings.append(heading);
    const note = document.createElement("p");
    note.textContent = "Les règles servent aux revues futures. Les retirer n’annule pas une action déjà autorisée.";
    settings.append(note);
    if (!rules.length) {
      const empty = document.createElement("p");
      empty.textContent = state.forms?.error ? "Les règles sont momentanément indisponibles." : "Aucune règle enregistrée.";
      settings.append(empty);
    }
    for (const rule of rules) {
      const row = document.createElement("p");
      const description = document.createElement("span");
      const botName = state.bots?.find((bot) => bot.id === rule.route.requesting_bot_id)?.name || rule.route.requesting_bot_id;
      description.textContent = `${botName} · ${rule.tool} · ${rule.allow ? "Toujours autoriser" : "Toujours refuser"} · ${rule.scope === "global" ? "Toutes les actions de cet outil" : `Arguments identiques · ${rule.args_digest.slice(0, 8)}`} `;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Retirer la règle";
      remove.addEventListener("click", async () => {
        remove.disabled = true;
        try {
          const response = await panels.api("/api/approvals/rules/revoke", {
            requesting_bot_id: rule.route.requesting_bot_id,
            target_bot_id: rule.route.target_bot_id,
            tool: rule.tool, args_digest: rule.args_digest, scope: rule.scope,
          });
          renderRules({ ...panels.state, forms: { ...panels.state.forms, rules: response.rules } });
        } catch {
          panels.notify("Retrait non confirmé. Actualisez puis réessayez.");
          return;
        } finally { remove.disabled = false; }
        // A failed follow-up read cannot undo an acknowledged rule retirement.
        await panels.refresh().catch(() => panels.notify("Règle retirée. L’actualisation de l’état reste à faire."));
      });
      row.append(description, remove);
      settings.append(row);
    }
  }

  panels.register({
    id: "forms",
    settingsSection(container) { settings = container; renderRules(panels.state); },
    onState(state) {
      for (const request of state.forms?.requests || []) observe(request);
      // Older entries can fall outside the bounded state.forms history.
      for (const conversation of state.conversations || []) {
        for (const message of conversation.messages || []) {
          if (kinds.has(message.kind)) observe(message.payload);
        }
      }
      renderRules(state);
      scheduleExpiry(state);
    },
    wantsPolling({ state }) {
      // SR01/SR02: accepted is not proof of resumption. /api/state projects the
      // current receipt without re-sending form fields or any secret value.
      return [...activity.values()].some((item) => item.busy || item.receipt?.wake?.status === "pending")
        || (state.forms?.requests || []).some((request) => request.receipt?.wake?.status === "pending"
          || (pendingApproval(request) && request.kind.expires_at <= Date.now() + 1000));
    },
    async onCardAction(message, action, detail, context) {
      if (!kinds.has(message.kind) || !action.startsWith("forms.")) return false;
      if (action === "forms.refresh") { await panels.refresh(); return true; }
      const request = message.payload;
      if (!request?.entry_id || context.conversation?.id !== request.route?.target_conversation_id) {
        throw new Error("Ouvrez la conversation où cette demande est affichée pour répondre.");
      }
      const previous = activity.get(request.entry_id);
      if (previous?.busy) return true;
      // Retries of one intent stay idempotent, even after a renderer reload.
      // Opposite decisions and cancel-vs-submit must never replay one another.
      // Form/secret values are intentionally NOT part of this non-sensitive ID.
      const intent = action === "forms.decide" ? `${action}:${JSON.stringify(detail?.decision)}` : action;
      const body = {
        entry_id: request.entry_id, submission_id: `ui:${request.entry_id}:${intent}`,
        conversation_id: context.conversation.id,
      };
      let path;
      if (action === "forms.cancel") path = "/api/forms/cancel";
      else if (action === "forms.submit" && message.kind === "form") {
        path = "/api/forms/submit"; body.values = detail?.values;
      } else if (action === "forms.secret" && message.kind === "secret") {
        path = "/api/forms/submit"; body.value = detail?.value;
      } else if (action === "forms.decide" && message.kind === "approval") {
        path = "/api/approvals/resolve"; body.decision = detail?.decision;
        // Do not echo the displayed arguments: they may be redacted. The server
        // binds entry_id to the original action digest (wrapper review r2 §2).
      } else return false;
      activity.set(request.entry_id, { ...previous, busy: true, error: null });
      changed();
      try {
        const response = await panels.api(path, body);
        if (!response.accepted || !response.receipt?.wake) throw new Error("Missing receipt");
        activity.set(request.entry_id, { busy: false, receipt: response.receipt });
      } catch (error) {
        // api() wraps transport/decoding failures in these fixed shell messages;
        // other rejections are HTTP errors. Never echo the exception or data.
        const uncertain = error instanceof TypeError || ["Missing receipt",
          "La connexion au moteur local est interrompue.", "Le moteur local a renvoyé une réponse illisible.",
        ].includes(error?.message);
        activity.set(request.entry_id, { ...previous, busy: false,
          error: uncertain ? "Envoi non confirmé. Actualisez l’état avant de réessayer."
            : "Le moteur a refusé la réponse. Vérifiez les champs ou actualisez pour voir la décision déjà enregistrée." });
      } finally {
        delete body.value;
        changed();
      }
      // Reconcile lost responses and decisions made in another window. A refresh
      // failure must not discard a receipt already acknowledged by the server.
      try { await panels.refresh(); } catch { /* The shell displays connection state. */ }
      return true;
    },
  });
})();
