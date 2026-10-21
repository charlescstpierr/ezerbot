"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const node = (tag, cls, text) => { const el = document.createElement(tag); el.className = cls || ""; if (text) el.textContent = text; return el; };
  const button = (text, action, cls = "secondary-button") => { const el = node("button", cls, text); el.type = "button"; el.onclick = action; return el; };
  const errors = {
    mic_denied: "Accès au microphone refusé. Autorisez Ezerbot dans Réglages Système › Confidentialité et sécurité › Microphone, puis recommencez.",
    speech_denied: "Reconnaissance vocale refusée. Autorisez Ezerbot dans Réglages Système, puis recommencez.",
    offline_unavailable: "Transcription sur ce Mac indisponible. Installez les données de Dictée en français dans Réglages Système. Aucun audio n’est envoyé à un serveur.",
    no_microphone: "Aucun microphone disponible. Branchez un microphone puis recommencez.",
    failed: "Aucune parole reconnue ou transcription échouée. Réessayez la transcription sur ce Mac, ou supprimez cette capture.",
    busy: "Une dictée ou une réunion est déjà en cours. Arrêtez-la avant de commencer.",
    microphone_busy: "Une autre capture utilise déjà le microphone. Arrêtez-la avant de continuer.",
    unavailable: "L’enregistrement de réunion est disponible dans l’application Mac Ezerbot.",
    consent_required: "Confirmez le consentement de toutes les personnes avant d’enregistrer.",
  };
  let context, startButton, bar, info, phaseLabel, meetingName, sourceLabel, clock, pause, stop, discard, retry, review, keep, save, retention, interrupted, notice, openThread, settings, emptyView, emptyHeading, emptyDescription;
  let dialog, title, consent, begin, dialogError, confirmation, confirmationTitle, confirmationText, confirmationAccept, finishConfirmation;
  let session = null, phase = "idle", pending = null, generation = 0, timer, requestBusy = false, lastActive = false, unfinished = [];
  const hasUnfinished = () => unfinished.some(item => item.id !== session?.id);
  const active = () => ["authorizing", "recording", "paused", "transcribing", "saving"].includes(phase);
  const emptyCapture = () => ["mic_denied", "speech_denied", "no_microphone", "offline_unavailable", "busy", "meeting_busy", "microphone_busy", "unavailable", "consent_required", "no_audio"].includes(phase);
  // A shared signal also updates dictation immediately, not at the next state poll.
  panels.meetingActive = active;
  function confirmDelete(heading, description, label) {
    if (finishConfirmation) return Promise.resolve(false);
    confirmationTitle.textContent = heading; confirmationText.textContent = description; confirmationAccept.textContent = label;
    return new Promise((resolve) => {
      finishConfirmation = (accepted) => {
        finishConfirmation = null; confirmation.close(); resolve(accepted);
      };
      confirmation.showModal();
    });
  }
  const bridge = (command, token = session?.id || "") => {
    const body = { command, token, purpose: "meeting", consent: true, language: "fr" };
    const native = window.webkit?.messageHandlers?.ezerbotVoice;
    if (native) return native.postMessage(body);
    // Debug-only fixture/transcriber shared with dictation. Never getUserMedia.
    if (context?.state?.meetings?.debug_fake) return panels.api("/api/voice/debug", body);
    return Promise.resolve({ status: "unavailable" });
  };
  function render() {
    if (!bar) return;
    bar.hidden = phase === "idle";
    bar.dataset.phase = phase;
    const tone = phase === "offline_unavailable" || (phase === "ready" && (pending?.too_long || pending?.partial)) ? "warning" : ["mic_denied", "speech_denied", "no_microphone", "failed", "no_audio", "save_failed"].includes(phase) ? "danger" : "";
    if (tone) bar.dataset.tone = tone; else delete bar.dataset.tone;
    startButton.disabled = !!session || hasUnfinished() || Boolean(panels.voiceActive?.() || window.EzerbotVoiceMode?.active()) || context?.conversation?.kind === "task";
    startButton.title = hasUnfinished() ? "Fermez l’avertissement de capture interrompue avant de recommencer" : window.EzerbotVoiceMode?.active() ? "Indisponible pendant le mode voix" : panels.voiceActive?.() ? "Indisponible pendant une dictée ou un mémo vocal" : session ? "Une réunion est déjà en cours" : "Enregistrer une réunion";
    phaseLabel.textContent = errors[phase] || ({authorizing:"Autorisation du microphone…",recording:"Réunion en cours",paused:"Réunion en pause",transcribing:"Transcription sur ce Mac…",ready:pending?.too_long ? "Limite de 10 minutes atteinte · Capture arrêtée" : pending?.partial ? "Transcription partielle · Capture arrêtée" : "Capture arrêtée · Transcription prête",saving:"Conservation de la réunion…",save_failed:"La réunion n’a pas pu être conservée. Réessayez sans réenregistrer."}[phase] || "Capture interrompue. Supprimez-la puis recommencez.");
    meetingName.textContent = session?.title || ""; meetingName.title = session?.title || "";
    meetingName.hidden = !session?.title;
    sourceLabel.hidden = !["recording", "paused", "transcribing"].includes(phase);
    sourceLabel.textContent = phase === "recording" ? "Microphone actif" : "Microphone arrêté";
    const ownEmpty = Boolean(session && context?.conversation?.id === session.conversation_id && !context.conversation.messages?.length && !emptyCapture() && phase !== "idle");
    emptyView.hidden = !ownEmpty;
    document.getElementById("welcome").hidden = ownEmpty || Boolean(context?.conversation?.messages?.length);
    emptyHeading.textContent = ({paused:"Réunion en pause",transcribing:"Transcription en cours",ready:"Capture terminée",failed:"Transcription à réessayer",save_failed:"Réunion à conserver"})[phase] || "Réunion en cours";
    emptyDescription.textContent = phase === "failed" ? "Réessayez la transcription ou supprimez la capture." : "Le résumé apparaîtra ici après conservation de la réunion.";
    pause.hidden = !["recording", "paused"].includes(phase); pause.textContent = phase === "paused" ? "Reprendre" : "Pause";
    stop.hidden = !["recording", "paused"].includes(phase);
    discard.hidden = phase === "saving";
    discard.textContent = emptyCapture() ? "Fermer" : "Supprimer";
    discard.setAttribute("aria-label", emptyCapture() ? "Fermer la capture" : "Supprimer la capture");
    discard.title = emptyCapture() ? "Fermer" : "Supprimer la capture";
    retry.hidden = phase !== "failed" && !(phase === "ready" && pending?.partial);
    retry.textContent = phase === "ready" ? "Réessayer les passages" : "Réessayer la transcription";
    retry.title = "Réessayer uniquement les passages non transcrits, sans refaire les passages déjà reconnus";
    settings.hidden = !["mic_denied", "speech_denied", "offline_unavailable"].includes(phase);
    openThread.hidden = !session || context?.conversation?.id === session.conversation_id;
    notice.hidden = !["authorizing", "recording", "paused"].includes(phase);
    review.hidden = !["ready", "save_failed"].includes(phase);
    clock.hidden = !["recording", "paused", "transcribing"].includes(phase) && !(phase === "ready" && (pending?.too_long || pending?.partial));
    if (active() !== lastActive) {
      lastActive = active(); window.dispatchEvent(new Event("grokbot:meeting-change"));
    }
    panels.composerChanged();
  }
  async function accept(result, rev) {
    if (rev !== generation || !session || (result.token && result.token !== session.id)) return;
    phase = result.status;
    clock.textContent = `${Math.floor((result.elapsed_ms || 0)/60000)}:${String(Math.floor((result.elapsed_ms || 0)/1000)%60).padStart(2,"0")} / 10:00`;
    if (phase === "ready") {
      pending = result;
      if (!result.data) { phase = "no_audio"; errors.no_audio = "Aucun audio enregistré. Vérifiez le microphone, puis recommencez."; }
      else if (!result.transcript?.trim()) phase = "failed";
      const missing = result.missing_segments || 1;
      review.querySelector(".meeting-cap").textContent = result.partial ? `${missing} passage${missing > 1 ? "s" : ""} non transcrit${missing > 1 ? "s" : ""} ; les autres passages sont conservables.` : result.too_long ? "La capture s’est arrêtée automatiquement. Vous pouvez conserver la transcription." : "Audio supprimé par défaut ; rétention de l’audio gardé dans Paramètres.";
      review.querySelector("pre").textContent = result.transcript || "";
    }
    render();
    if (["authorizing", "recording", "paused", "transcribing"].includes(phase)) timer = setTimeout(() => poll(rev), 250);
  }
  async function poll(rev) {
    if (rev !== generation) return;
    try { await accept(await bridge("status"), rev); }
    catch (error) { if (rev === generation) { phase = "failed"; render(); panels.notify(error.message); } }
  }
  async function command(name) {
    if (requestBusy || !session) return;
    requestBusy = true; clearTimeout(timer); const rev = ++generation;
    if (name === "stop" || name === "retry") { phase = "transcribing"; render(); }
    try { await accept(await bridge(name), rev); }
    catch (error) { phase = pending?.partial ? "ready" : "failed"; render(); panels.notify(error.message); }
    finally { requestBusy = false; }
  }
  const hasDraft = (conversation) => context?.conversation?.id === conversation ? !!panels.draft().trim() : !!localStorage.getItem(`grokbot.draft.${conversation}`)?.trim();
  async function cancelCapture() {
    if (!session) { phase = "idle"; render(); return; }
    if (!emptyCapture() && !await confirmDelete("Supprimer la capture ?", "L’audio et la transcription de cette capture seront supprimés. Une conversation créée uniquement pour elle sera retirée si elle est toujours vide.", "Supprimer la capture")) return;
    clearTimeout(timer); ++generation;
    try {
      if (!["busy", "meeting_busy", "microphone_busy"].includes(phase)) await bridge("cancel");
      await panels.api("/api/meetings/discard", { id: session.id, preserve_conversation: hasDraft(session.conversation_id) });
      session = pending = null; phase = "idle"; render(); await panels.refresh();
    } catch (error) { panels.notify(error.message); render(); }
  }
  async function start() {
    if (session || !consent.checked) return;
    if (hasUnfinished()) { panels.notify("Fermez l’avertissement de capture interrompue pour la supprimer, puis recommencez."); return; }
    if (window.EzerbotVoiceMode?.active()) { panels.notify("Quittez le mode voix avant d’enregistrer une réunion."); return; }
    if (panels.voiceActive?.()) { panels.notify("Une dictée est en cours. Arrêtez-la avant d’enregistrer une réunion."); return; }
    begin.disabled = true; dialogError.textContent = "";
    try {
      const welcome = !context.conversation, draft = panels.draft();
      session = await panels.api("/api/meetings/start", { conversation_id: context.conversation?.id || null, bot_id: context.bot?.id, title: title.value, consent: consent.checked });
      const rev = ++generation; phase = "authorizing"; keep.checked = false; pending = null;
      if (welcome) {
        if (draft) {
          localStorage.setItem(`grokbot.draft.${session.conversation_id}`, draft);
          localStorage.setItem(`grokbot.draft.${session.conversation_id}.document`, JSON.stringify(panels.draftDocument()));
        }
        await panels.refresh();
        if (!panels.openConversation(session.conversation_id)) throw new Error("Ouvrez la conversation créée avant d’enregistrer.");
        localStorage.removeItem("grokbot.draft.new"); localStorage.removeItem("grokbot.draft.new.document");
      }
      dialog.close(); render(); await accept(await bridge("start"), rev);
    } catch (error) {
      dialogError.textContent = error.message; panels.notify(error.message);
      if (session) { phase = "failed"; render(); }
    } finally { begin.disabled = !consent.checked; }
  }
  async function saveCapture() {
    if (!pending || !session || phase === "saving") return;
    phase = "saving"; render();
    try {
      const id = session.id, conversation = session.conversation_id;
      await panels.api("/api/meetings/finalize", { id, data: pending.data, transcript: pending.transcript, keep_audio: keep.checked, missing_segments: pending.missing_segments || 0 });
      await bridge("cancel"); // Wipes temporary audio regardless of retention choice.
      session = pending = null; phase = "idle"; ++generation; render();
      await panels.refresh(); panels.openConversation(conversation, id);
      try { await panels.api("/api/meetings/summarize", { id }); }
      catch (error) { panels.notify(error.message); }
      await panels.refresh();
    } catch (error) { phase = "save_failed"; render(); panels.notify(error.message); }
  }
  panels.register({
    id: "meetings",
    settingsSection(container) {
      container.className = "setting-section";
      container.append(node("h3", "", "Réunions et rétention audio"), node("p", "", "L’audio est supprimé après transcription par défaut. Si vous choisissez de le garder, il est conservé uniquement sur ce Mac pour la durée ci-dessous. Le résumé et la transcription restent dans la conversation. Ce réglage concerne les prochaines captures ; il ne supprime pas les précédentes."));
      const label = node("label", "meeting-retention", "Durée de conservation de l’audio gardé"); retention = node("select"); retention.id = "meeting-retention";
      for (const days of [1,7,30]) { const option = node("option", "", `${days} jour${days > 1 ? "s" : ""}`); option.value = String(days); retention.append(option); }
      retention.onchange = async () => { try { await panels.api("/api/meetings/settings", { audio_days: Number(retention.value) }); await panels.refresh(); panels.notify("Rétention des prochaines captures enregistrée."); } catch (error) { panels.notify(error.message); } };
      label.append(retention); container.append(label);
    },
    composer: {
      mount(_container, ctx) {
        context = ctx;
        startButton = button("● Enregistrer une réunion", () => { title.value = "Réunion"; consent.checked = false; begin.disabled = true; dialogError.textContent = ""; dialog.showModal(); }, "attachment-choose meeting-start");
        startButton.id = "meeting-start"; startButton.setAttribute("aria-label", "Enregistrer une réunion");
        // Le « + » des pièces jointes vit dans la barre d'écriture ; la réunion se range juste après.
        const attach = _container.querySelector(".attachment-choose") || document.getElementById("composer-model")?.querySelector?.(".attachment-choose");
        if (attach) attach.after(startButton); else _container.append(startButton);
        dialog = node("dialog", "dialog small-dialog meeting-dialog"); dialog.setAttribute("aria-labelledby", "meeting-consent-title");
        const heading = node("h2", "", "Enregistrer une réunion"); heading.id = "meeting-consent-title";
        const titleLabel = node("label", "", "Titre de la réunion"); title = node("input"); title.id = "meeting-title"; title.maxLength = 100; titleLabel.append(title);
        const reminder = node("p", "delete-description", "Demandez l’accord de toutes les personnes avant de commencer. Microphone uniquement (pas d’audio système). Maximum 10 minutes, avec pause et arrêt à tout moment. Transcription sur ce Mac ; seule la transcription est envoyée au bot pour le résumé, les décisions et les actions. L’audio est supprimé par défaut après transcription.");
        const consentLabel = node("label", "meeting-consent"); consent = node("input"); consent.type = "checkbox"; consent.id = "meeting-consent";
        consent.onchange = () => { begin.disabled = !consent.checked; }; consentLabel.append(consent, document.createTextNode("Toutes les personnes ont donné leur accord pour la capture et le résumé."));
        begin = button("Commencer l’enregistrement", start, "primary-button"); begin.id = "meeting-begin";
        const footer = node("div", "dialog-footer"); footer.append(button("Annuler", () => dialog.close()), begin);
        dialogError = node("p", "meeting-error"); dialogError.setAttribute("role", "alert"); dialog.append(heading,titleLabel,reminder,consentLabel,dialogError,footer); document.getElementById("panels").append(dialog);
        confirmation = node("dialog", "dialog small-dialog meeting-confirmation"); confirmation.id = "meeting-confirmation";
        confirmation.setAttribute("aria-labelledby", "meeting-confirmation-title");
        confirmationTitle = node("h2"); confirmationTitle.id = "meeting-confirmation-title";
        confirmationText = node("p", "delete-description");
        const confirmationActions = node("div", "dialog-footer");
        const confirmationCancel = button("Annuler", () => finishConfirmation?.(false)); confirmationCancel.id = "meeting-confirmation-cancel";
        confirmationAccept = button("Supprimer", () => finishConfirmation?.(true), "danger-button"); confirmationAccept.id = "meeting-confirmation-accept";
        confirmationActions.append(confirmationCancel, confirmationAccept);
        confirmation.append(confirmationTitle, confirmationText, confirmationActions);
        confirmation.addEventListener("cancel", (event) => { event.preventDefault(); finishConfirmation?.(false); });
        confirmation.addEventListener("close", () => { if (!confirmation.open) finishConfirmation?.(false); });
        document.getElementById("panels").append(confirmation);
        bar = node("section", "meeting-bar"); bar.id = "meeting-recording"; bar.setAttribute("aria-label", "Capture de réunion");
        const row = node("div", "meeting-row"); const dot = node("span", "meeting-dot"); dot.setAttribute("aria-hidden", "true");
        info = node("strong", "meeting-info"); info.setAttribute("role", "status");
        phaseLabel = node("span", "meeting-phase"); meetingName = node("span", "meeting-name"); sourceLabel = node("span", "meeting-source");
        info.append(phaseLabel,sourceLabel,meetingName); clock = node("span", "meeting-clock");
        pause = button("Pause", () => command(phase === "paused" ? "resume" : "pause")); stop = button("Arrêter", () => command("stop"));
        discard = button("Supprimer", cancelCapture); discard.id = "meeting-discard";
        retry = button("Réessayer la transcription", () => command("retry"));
        openThread = button("Ouvrir la conversation", () => panels.openConversation(session.conversation_id)); openThread.id = "meeting-open-conversation";
        settings = button("Ouvrir Réglages Système", () => bridge("settings").catch(() => panels.notify("Ouvrez Réglages Système › Confidentialité et sécurité › Microphone."))); settings.id = "meeting-system-settings";
        const actions = node("div", "meeting-actions"); actions.append(openThread,pause,stop,retry,settings,discard); row.append(dot,info,clock,actions);
        notice = node("p", "meeting-reminder", "Capture consentie · Prévenez toute nouvelle personne qui rejoint la réunion.");
        review = node("div", "meeting-review"); review.append(node("p", "meeting-cap"));
        const details = node("details"); details.append(node("summary", "", "Vérifier la transcription"),node("pre"));
        const keepLabel = node("label", "meeting-consent"); keep = node("input"); keep.type = "checkbox"; keep.id = "meeting-keep-audio"; keepLabel.append(keep,document.createTextNode("Garder l’audio sur ce Mac"));
        save = button("Conserver et produire le résumé", saveCapture, "primary-button"); save.id = "meeting-save";
        const reviewActions = node("div", "meeting-review-actions"); reviewActions.append(keepLabel,save);
        review.append(reviewActions,details);
        bar.append(row,notice,review); document.querySelector(".topbar").after(bar);
        interrupted = node("div", "meeting-interrupted"); interrupted.dataset.tone = "warning"; interrupted.setAttribute("role", "status"); document.querySelector(".topbar").after(interrupted);
        emptyView = node("section", "meeting-empty"); emptyView.id = "meeting-empty"; emptyView.setAttribute("role", "status");
        emptyHeading = node("h2"); emptyDescription = node("p"); emptyView.append(emptyHeading,emptyDescription);
        document.getElementById("welcome").before(emptyView);
        window.addEventListener("beforeunload", () => { clearTimeout(timer); if (session) bridge("cancel").catch(() => {}); });
        window.addEventListener("grokbot:voice-change", render);
        window.addEventListener("grokbot:voice-mode-change", render);
        render();
      },
      blocked: () => active(),
      blockedReason: () => phase === "transcribing" ? "Envoi suspendu pendant la transcription de la réunion. Patientez jusqu’à la fin." : phase === "saving" ? "Enregistrement de la réunion en cours. Patientez pour envoyer." : "Envoi suspendu pendant l’enregistrement de la réunion. Arrêtez la capture pour envoyer.",
      // Navigation does not end or hide capture. Only explicit stop/discard does.
    },
    onState(state, ctx) {
      context = ctx;
      unfinished = state.meetings?.interrupted || [];
      if (retention) retention.value = String(state.meetings?.audio_days || 7);
      if (interrupted) {
        interrupted.replaceChildren();
        for (const item of unfinished) {
          if (session?.id === item.id) continue;
          const warning = node("div", "meeting-row meeting-interrupted-row"); warning.append(node("span", "", `« ${item.title} » : capture interrompue (application rechargée ou redémarrée). Fermez cet avertissement pour supprimer la capture inachevée et recommencer. Aucun enregistrement ne reprend automatiquement.`), button("Fermer", async () => { try { await bridge("cancel", item.id).catch(() => {}); await panels.api("/api/meetings/discard", { id: item.id, preserve_conversation: hasDraft(item.conversation_id) }); await panels.refresh(); } catch (error) { panels.notify(error.message); } })); interrupted.append(warning);
        }
        interrupted.hidden = !interrupted.children.length;
      }
      render();
    },
    wantsPolling: () => !!session,
    async onCardAction(message, action) {
      if (message.kind !== "meeting") return false;
      const meeting = message.payload;
      if (action === "meeting-delete-audio") {
        if (!await confirmDelete("Supprimer l’audio ?", "L’audio de cette réunion sera supprimé définitivement. La transcription et le résumé disponible resteront dans la conversation.", "Supprimer l’audio")) return true;
        await panels.api("/api/meetings/delete-audio", { id: meeting.id });
      } else if (action === "meeting-summary") {
        if (meeting.summary_state === "pending") await panels.api("/api/meetings/summarize", { id: meeting.id });
        else await panels.api("/api/regenerate", { conversation_id: meeting.conversation_id, message_id: meeting.message_id });
      } else if (action === "meeting-stop-summary") {
        await panels.api("/api/stop", { conversation_id: meeting.conversation_id, message_id: meeting.message_id });
      } else return false;
      await panels.refresh(); return true;
    },
  });
})();
