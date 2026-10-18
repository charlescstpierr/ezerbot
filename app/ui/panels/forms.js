"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const kinds = new Set(["form", "approval", "secret"]);
  const activity = new Map();
  const changed = () => window.dispatchEvent(new Event("grokbot:forms-change"));
  // Only non-sensitive delivery metadata lives here. Never cache values, arguments,
  // secrets or request bodies in storage, logs, notifications or the transcript.
  let storageStatus;
  let storageFeedback;
  let storageCheckBusy = false;
  let storageWarning;
  window.GrokbotForms = { view: (id) => activity.get(id), storage: () => storageStatus };

  function observeStorage(state) {
    const next = state.forms?.storage;
    if (JSON.stringify(next) !== JSON.stringify(storageStatus)) {
      if (!storageCheckBusy) storageFeedback = undefined;
      storageStatus = next;
      changed();
    }
    const button = document.getElementById?.("settings-button");
    if (!button) return;
    if (!storageWarning) {
      storageWarning = document.createElement("span");
      storageWarning.className = "storage-warning-marker";
      storageWarning.textContent = "!";
      storageWarning.setAttribute("role", "img");
      storageWarning.setAttribute("aria-label", "Attention : les nouveaux secrets sont enregistrés dans un fichier de secours non chiffré. Vérifiez le stockage dans Paramètres.");
      button.append(storageWarning);
    }
    storageWarning.hidden = next?.backend !== "file_fallback";
  }

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
  let reviewDraft;
  let reviewNotice;
  let reviewStatus;
  let reviewErrors = {};
  let reviewBusy = false;
  let reviewControls = [];
  function setReviewNotice(text, error = false) {
    reviewNotice = text ? { text, error } : undefined;
    if (reviewStatus) {
      reviewStatus.textContent = text || "";
      reviewStatus.className = `reviewer-notice${error ? " reviewer-error" : ""}`;
      reviewStatus.setAttribute("role", error ? "alert" : "status");
    }
  }
  function setReviewBusy(busy) {
    reviewBusy = busy;
    for (const control of reviewControls) control.disabled = busy;
  }
  function reviewEditor(state) {
    const saved = state.forms?.reviewer;
    if (!saved) return;
    const draft = reviewDraft || { enabled: saved.config.enabled,
      allow: saved.config.allow.join("\n"), block: saved.config.block.join("\n"), version: saved.version };
    const form = document.createElement("form");
    form.className = "reviewer-settings";
    const title = document.createElement("h4");
    title.textContent = "Règles en langage naturel";
    const toggle = document.createElement("label");
    toggle.className = "setting-toggle";
    const copy = document.createElement("span");
    const toggleTitle = document.createElement("strong");
    toggleTitle.textContent = "Activer le relecteur d’approbations";
    const note = document.createElement("small");
    note.textContent = "Après les règles « Toujours », un modèle relit vos demandes. Au moindre doute ou en cas d’erreur, vous décidez. Aucun secret ne lui est transmis. Les commandes et données sensibles nécessitent votre confirmation.";
    copy.append(toggleTitle, note);
    const enabled = document.createElement("input");
    enabled.type = "checkbox"; enabled.name = "reviewer-enabled"; enabled.checked = draft.enabled;
    enabled.setAttribute("role", "switch");
    const track = document.createElement("span");
    track.className = "toggle-track"; track.setAttribute("aria-hidden", "true");
    toggle.append(copy, enabled, track);
    form.append(title, toggle);
    const fields = {};
    const errors = {};
    for (const [key, label, placeholder] of [
      ["allow", "Autoriser ou demander confirmation", "Autorise toujours la lecture dans ~/Documents/projets, mais demande-moi avant toute suppression."],
      ["block", "Refuser systématiquement", "Refuse toute suppression dans ~/Documents/archives."],
    ]) {
      const wrapper = document.createElement("label");
      wrapper.textContent = label;
      const field = document.createElement("textarea");
      field.className = "text-input"; field.name = `reviewer-${key}`; field.rows = 3; field.value = draft[key];
      field.placeholder = placeholder; field.setAttribute("aria-label", label);
      const error = document.createElement("p");
      error.id = `reviewer-${key}-error`; error.className = "reviewer-error";
      error.setAttribute("role", "alert"); error.textContent = reviewErrors[key] || "";
      field.setAttribute("aria-describedby", error.id);
      field.setAttribute("aria-invalid", String(Boolean(reviewErrors[key])));
      fields[key] = field; errors[key] = error;
      wrapper.append(field); form.append(wrapper, error);
    }
    const limits = document.createElement("p");
    limits.textContent = "Une règle par ligne · 20 règles par liste · 1 000 caractères par règle. Les refus et demandes de confirmation priment. N’incluez aucun secret. Modifier ou désactiver ces règles agit sur les appels futurs, pas sur une action déjà autorisée. Cette revue utilise votre modèle et votre abonnement ChatGPT.";
    reviewStatus = document.createElement("p");
    setReviewNotice(reviewNotice?.text, reviewNotice?.error);
    const save = document.createElement("button"); save.type = "submit";
    save.className = "primary-button"; save.textContent = "Enregistrer les règles";
    const capture = () => { reviewDraft = { enabled: enabled.checked,
      allow: fields.allow.value, block: fields.block.value, version: draft.version }; };
    function clearFeedback() {
      reviewErrors = {};
      for (const key of Object.keys(fields)) {
        fields[key].setAttribute("aria-invalid", "false"); errors[key].textContent = "";
      }
      setReviewNotice("");
    }
    form.addEventListener("input", () => { capture(); clearFeedback(); });
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      if (reviewBusy) return;
      capture(); clearFeedback();
      const entries = (text) => text.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
      const config = { enabled: reviewDraft.enabled, allow: entries(reviewDraft.allow), block: entries(reviewDraft.block) };
      for (const key of Object.keys(fields)) {
        if (config[key].length > 20 || config[key].some((line) => [...line].length > 1000)) {
          reviewErrors[key] = "Limite dépassée : 20 règles par liste, 1 000 caractères par règle.";
          fields[key].setAttribute("aria-invalid", "true"); errors[key].textContent = reviewErrors[key];
        }
      }
      const invalid = Object.keys(reviewErrors)[0];
      if (invalid) { fields[invalid].focus(); return; }
      setReviewBusy(true);
      try {
        await panels.api("/api/approvals/reviewer", { config, version: reviewDraft.version });
        reviewDraft = undefined; rulesKey = undefined;
        setReviewNotice("Règles enregistrées. Elles s’appliqueront aux prochaines demandes.");
        try { await panels.refresh(); }
        catch { setReviewNotice("Règles enregistrées. L’actualisation de l’état reste à faire."); }
      } catch {
        setReviewNotice("Enregistrement non confirmé. Vérifiez que vos règles ne contiennent aucun secret. Si elles ont changé dans une autre fenêtre, rechargez-les avant de réessayer.", true);
      } finally { setReviewBusy(false); }
    });
    const reload = document.createElement("button"); reload.type = "button";
    reload.className = "secondary-button"; reload.textContent = "Recharger les règles enregistrées";
    reload.addEventListener("click", async () => {
      if (reviewBusy) return;
      setReviewBusy(true);
      try {
        await panels.refresh(); reviewDraft = undefined; reviewErrors = {}; rulesKey = undefined;
        setReviewNotice("Règles rechargées."); renderRules(panels.state);
      } catch { setReviewNotice("Les règles ne peuvent pas être rechargées pour le moment.", true); }
      finally { setReviewBusy(false); }
    });
    const actions = document.createElement("div"); actions.className = "reviewer-actions";
    actions.append(save, reload);
    reviewControls = [enabled, ...Object.values(fields), save, reload];
    setReviewBusy(reviewBusy);
    form.append(limits, reviewStatus, actions); settings.append(form);
  }
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
    const key = JSON.stringify([rules, state.forms?.reviewer, state.forms?.storage, storageFeedback, storageCheckBusy, state.bots?.map((bot) => [bot.id, bot.name]), Boolean(state.forms?.error)]);
    if (key === rulesKey) return;
    rulesKey = key;
    settings.replaceChildren();
    settings.className = "setting-section";
    const storageBlock = document.createElement("div");
    storageBlock.className = "storage-settings";
    const storageTitle = document.createElement("h3");
    storageTitle.textContent = "Stockage des secrets";
    const storage = document.createElement("p");
    storage.textContent = state.forms?.storage?.label || "Stockage momentanément indisponible";
    storage.className = `storage-backend${state.forms?.storage?.backend === "file_fallback" ? " storage-backend-fallback" : ""}`;
    const storageNote = document.createElement("p");
    storageNote.textContent = "Secrets des formulaires et variables privées des connecteurs MCP. Les identifiants ChatGPT sont gérés séparément.";
    storageBlock.append(storageTitle, storage, storageNote);
    if (state.forms?.storage?.notice) {
      const notice = document.createElement("p");
      const information = state.forms.storage.notice_kind === "info";
      notice.className = `storage-notice${information ? " storage-info" : ""}`;
      notice.setAttribute("role", information ? "status" : "alert");
      const title = document.createElement("strong");
      title.textContent = information ? "Activation du trousseau en attente" : "Secrets non chiffrés pour le moment.";
      const detail = document.createElement("span");
      detail.textContent = state.forms.storage.notice;
      notice.append(title, detail);
      storageBlock.append(notice);
      if (!information) {
      const retry = document.createElement("button");
      retry.type = "button";
      retry.className = "secondary-button";
      retry.textContent = "Vérifier le stockage";
      retry.disabled = storageCheckBusy;
      retry.addEventListener("click", async () => {
        if (storageCheckBusy) return;
        storageCheckBusy = true;
        storageFeedback = "Vérification du stockage en cours…";
        renderRules(panels.state);
        try {
          await panels.refresh();
          storageFeedback = panels.state.forms?.storage?.backend === "keychain"
            ? "Trousseau macOS rétabli : les secrets du fichier de secours y ont été transférés."
            : "Le trousseau est toujours inaccessible. Les nouveaux secrets restent dans le fichier de secours.";
        } catch { storageFeedback = "Le stockage ne peut pas être vérifié pour le moment. Réessayez lorsque le moteur local est disponible."; }
        finally { storageCheckBusy = false; renderRules(panels.state); }
      });
      const actions = document.createElement("p");
      actions.append(retry);
      storageBlock.append(actions);
      }
    }
    if (storageFeedback) {
      const feedback = document.createElement("p");
      feedback.className = "storage-feedback";
      feedback.setAttribute("role", "status");
      feedback.textContent = storageFeedback;
      storageBlock.append(feedback);
    }
    settings.append(storageBlock);
    const heading = document.createElement("h3");
    heading.textContent = "Autorisations permanentes";
    settings.append(heading);
    const note = document.createElement("p");
    note.textContent = "Les règles servent aux revues futures. Les retirer n’annule pas une action déjà autorisée.";
    settings.append(note);
    if (!rules.length) {
      const empty = document.createElement("p");
      empty.textContent = state.forms?.error ? "Les règles sont momentanément indisponibles." : "Aucune règle « Toujours » enregistrée.";
      settings.append(empty);
    }
    for (const rule of rules) {
      const row = document.createElement("p");
      const description = document.createElement("span");
      const botName = state.bots?.find((bot) => bot.id === rule.route.requesting_bot_id)?.name || rule.route.requesting_bot_id;
      const server = state.mcp?.servers?.find((s) => s.tools.some((t) => t.namespace === rule.tool) || [`mcp_start_${s.id}`, `mcp_remove_${s.id}`].includes(rule.tool));
      const tool = server?.tools.find((t) => t.namespace === rule.tool)?.name || (rule.tool.startsWith("mcp_start_") ? "Démarrer / redémarrer" : "Supprimer le connecteur");
      const toolName = server ? `${server.name} · ${tool}` : window.GrokbotLocalUi?.toolLabel(rule.tool) || rule.tool;
      description.textContent = `${botName} · ${toolName} · ${rule.allow ? "Toujours autoriser" : "Toujours refuser"} · ${rule.scope === "global" ? "Toutes les actions de cet outil" : `Arguments identiques · ${rule.args_digest.slice(0, 8)}`} `;
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
      if (toolLabel !== rule.tool) {
        const details = document.createElement("details");
        const summary = document.createElement("summary"); summary.textContent = "Détails techniques";
        const code = document.createElement("code"); code.textContent = `${rule.tool} · ${rule.args_digest}`;
        details.append(summary, code); settings.append(details);
      }
    }
    reviewEditor(state);
  }

  panels.register({
    id: "forms",
    settingsSection(container) { settings = container; renderRules(panels.state); },
    onState(state) {
      observeStorage(state);
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
      return (state.forms?.requests || []).some((request) => request.review?.pending && request.review.started_at + 13000 > Date.now())
        || [...activity.values()].some((item) => item.busy || item.receipt?.wake?.status === "pending")
        || (state.forms?.requests || []).some((request) => request.receipt?.wake?.status === "pending"
          || (pendingApproval(request) && request.kind.expires_at <= Date.now() + 1000));
    },
    async onCardAction(message, action, detail, context) {
      if (!kinds.has(message.kind) || !action.startsWith("forms.")) return false;
      if (action === "forms.refresh") { await panels.refresh(); return true; }
      if (action === "forms.rules") {
        document.getElementById("settings-button")?.click();
        settings?.scrollIntoView({ block: "start" });
        settings?.querySelector("textarea")?.focus({ preventScroll: true });
        return true;
      }
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
