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
    if (receipt && JSON.stringify(previous?.receipt) !== JSON.stringify(receipt)) {
      activity.set(request.entry_id, { ...previous, receipt, error: null });
      changed();
    }
  }

  // Standing permissions are reversible (REA goal-permissions-secrets.md PS03).
  // Removing one affects future reviews, not an action already admitted.
  let settings;
  let rulesKey;
  function renderRules(state) {
    if (!settings) return;
    const rules = state.forms?.rules || [];
    const key = JSON.stringify([rules, Boolean(state.forms?.error)]);
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
      description.textContent = `${rule.route.requesting_bot_id} · ${rule.tool} · ${rule.allow ? "Toujours autoriser" : "Toujours refuser"} · ${rule.scope === "global" ? "Toutes les actions de cet outil" : "Arguments identiques"} `;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Retirer la règle";
      remove.addEventListener("click", async () => {
        remove.disabled = true;
        try {
          await panels.api("/api/approvals/rules/revoke", {
            requesting_bot_id: rule.route.requesting_bot_id,
            target_bot_id: rule.route.target_bot_id,
            tool: rule.tool, args_digest: rule.args_digest, scope: rule.scope,
          });
          await panels.refresh();
        } catch { panels.notify("Impossible de retirer la règle. Actualisez puis réessayez."); }
        finally { remove.disabled = false; }
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
    },
    wantsPolling({ state }) {
      // SR01/SR02: accepted is not proof of resumption. /api/state projects the
      // current receipt without re-sending form fields or any secret value.
      return [...activity.values()].some((item) => item.busy || item.receipt?.wake?.status === "pending")
        || (state.forms?.requests || []).some((request) => request.status?.status === "pending");
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
      // A stable, per-entry ID makes an uncertain network retry idempotent even
      // after a renderer reload. The server's first resolution always wins.
      const body = {
        entry_id: request.entry_id, submission_id: `ui:${request.entry_id}`,
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
      } catch {
        // Do not echo an exception or the submitted data (especially a secret).
        activity.set(request.entry_id, { ...previous, busy: false,
          error: "Envoi non confirmé. Actualisez l’état avant de réessayer." });
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
