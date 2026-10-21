"use strict";
(() => {
  const panels = window.GrokbotPanels;
  // The Mac speech bridge does not run in the phone. iOS dictation stays on the keyboard.
  if (!panels || window.EzerbotIPhone?.remote) return;
  const errors = {
    mic_denied: "Accès au microphone refusé. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    speech_denied: "Reconnaissance vocale refusée. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    no_microphone: "Aucun microphone disponible. Branchez un microphone, puis réessayez.",
    offline_unavailable: "La transcription hors ligne n’est pas disponible pour cette langue sur ce Mac. Installez les données de Dictée dans Réglages Système, puis réessayez.",
    failed: "La transcription a échoué ou aucune parole n’a été reconnue. Réessayez sans réenregistrer.",
    unavailable: "La dictée est disponible dans l’application Mac Ezerbot.",
    upload_failed: "Le mémo n’a pas pu être conservé. Réessayez l’enregistrement local du mémo.",
    busy: "Une autre dictée est déjà en cours. Arrêtez-la avant de recommencer.",
    meeting_busy: "Une réunion est en cours. Arrêtez-la avant de dicter.",
    microphone_busy: "Une autre capture utilise déjà le microphone. Arrêtez-la avant de continuer.",
  };
  const node = (tag, cls, text) => { const el = document.createElement(tag); el.className = cls || ""; if (text) el.textContent = text; return el; };
  const key = (ctx) => ctx.conversation?.id || `new:${ctx.bot?.id || "grokbot"}`;
  let context, control, memoMode, mic, region, row, statusText, meter, timer, cancel, retry, settings, list;
  let session = null, phase = "idle", revision = 0, timeout, processing = false, pendingMemo = null, memoItems = [];
  const consumed = new Set();
  // Keep successful local uploads across an older in-flight /api/state reply.
  // Send/remove receipts, not a stale polling snapshot, relinquish ownership.
  const localMemos = new Map();
  const live = () => ["authorizing", "recording", "transcribing", "saving"].includes(phase);
  window.EzerbotDictation = { active: live, refresh: render, cancel() { reset(); changed(); } };
  const tones = { mic_denied: "danger", speech_denied: "danger", no_microphone: "danger", failed: "danger", upload_failed: "danger", offline_unavailable: "warning", limited: "warning" };
  const language = () => document.documentElement.lang || "fr";
  async function bridge(command, extra = {}) {
    const body = { command, token: session?.token || "", language: language(), ...extra };
    const native = window.webkit?.messageHandlers?.ezerbotVoice;
    if (native) return native.postMessage(body);
    // This route and adapter do not exist in release builds. Never fall back to
    // getUserMedia or browser SpeechRecognition (including during QA).
    if (context?.state?.voice?.debug_fake) return panels.api("/api/voice/debug", body);
    return { status: "unavailable" };
  }
  panels.voiceActive = live;
  let lastVoiceActive = false;
  function changed() { render(); panels.composerChanged(); }
  function render() {
    if (!mic) return;
    const recording = phase === "recording";
    const voiceActive = Boolean(window.EzerbotVoiceMode?.active());
    const meetingActive = Boolean(panels.meetingActive?.());
    mic.innerHTML = recording ? '<svg class="icon" aria-hidden="true"><use href="#i-stop"/></svg>' : '<svg class="icon" aria-hidden="true" viewBox="0 0 24 24"><rect x="9" y="2" width="6" height="13" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/></svg>';
    mic.setAttribute("aria-label", voiceActive ? "Dictée indisponible pendant le mode voix" : meetingActive ? "Dictée indisponible pendant la réunion" : recording ? "Arrêter la dictée" : "Démarrer la dictée");
    mic.title = voiceActive ? "Indisponible pendant le mode voix" : meetingActive ? "Indisponible pendant l’enregistrement d’une réunion" : recording ? "Arrêter · Échap" : "Dicter un message";
    mic.setAttribute("aria-pressed", String(recording));
    mic.disabled = voiceActive || meetingActive || (live() && !recording) || context?.conversation?.kind === "task";
    memoMode.disabled = voiceActive || meetingActive || live();
    memoMode.title = voiceActive ? "Indisponible pendant le mode voix" : meetingActive ? "Indisponible pendant l’enregistrement d’une réunion" : "Joindre le mémo vocal au message";
    region.hidden = phase === "idle" && !memoItems.length;
    if (tones[phase]) region.dataset.tone = tones[phase]; else delete region.dataset.tone;
    row.hidden = phase === "idle";
    cancel.hidden = phase === "idle";
    cancel.textContent = live() ? "Annuler" : "Fermer";
    retry.hidden = !["failed", "upload_failed"].includes(phase);
    settings.hidden = !["mic_denied", "speech_denied", "offline_unavailable"].includes(phase);
    meter.hidden = !recording;
    timer.hidden = !recording;
    statusText.textContent = errors[phase] || ({authorizing:"Autorisation du microphone…",recording: "Écoute en cours · Échap pour arrêter", transcribing:"Transcription sur ce Mac…",saving:"Enregistrement du mémo…",ready:"Transcription ajoutée au brouillon. Relisez-la avant d’envoyer.",limited:"Limite de 60 secondes atteinte. L’enregistrement est arrêté ; relisez la transcription avant d’envoyer."}[phase] || "");
    const existing = new Map([...list.children].map((el) => [el.dataset.id, el]));
    for (const item of memoItems) {
      if (existing.has(item.id)) { existing.delete(item.id); continue; }
      const chip = node("div", "voice-memo-chip"); chip.dataset.id = item.id;
      const name = node("strong", "", `Mémo vocal · ${Math.ceil(item.duration_ms / 1000)} s`);
      const audio = node("audio"); audio.controls = true; audio.preload = "metadata"; audio.src = `/api/voice/audio/${encodeURIComponent(item.id)}`; audio.setAttribute("aria-label", "Écouter le mémo avant l’envoi");
      const remove = node("button", "attachment-remove", "×"); remove.type = "button"; remove.setAttribute("aria-label", "Retirer le mémo vocal");
      remove.onclick = async () => {
        remove.disabled = true;
        try { await panels.api("/api/voice/remove", { id: item.id, conversation_id: item.conversation_id }); consumed.add(item.id); localMemos.delete(item.id); memoItems = memoItems.filter((m) => m.id !== item.id); changed(); }
        catch (error) { panels.notify(error.message); remove.disabled = false; }
      };
      chip.append(name, audio, remove); list.append(chip);
    }
    for (const el of existing.values()) el.remove();
    if (live() !== lastVoiceActive) {
      lastVoiceActive = live(); window.dispatchEvent?.(new Event("grokbot:voice-change"));
    }
  }
  function reset() {
    clearTimeout(timeout); revision++; processing = false; pendingMemo = null;
    if (session) bridge("cancel").catch(() => {});
    session = null; phase = "idle";
  }
  async function ensureConversation() {
    if (context.conversation) return;
    const before = key(context);
    const { conversation } = await panels.api("/api/conversations", { bot_id: context.bot?.id });
    await panels.refresh();
    if (key(context) !== before) throw new Error("La conversation a changé. Recommencez la dictée ici.");
    const draft = panels.draft();
    if (draft) {
      localStorage.setItem(`grokbot.draft.${conversation.id}`, draft);
      localStorage.setItem(`grokbot.draft.${conversation.id}.document`, JSON.stringify(panels.draftDocument()));
    }
    // The memo session follows its own new thread instead of being cancelled by it.
    if (session) session.key = conversation.id;
    if (!panels.openConversation(conversation.id)) throw new Error("Ouvrez la conversation créée pour joindre le mémo.");
    try {
      localStorage.removeItem("grokbot.draft.new");
      localStorage.removeItem("grokbot.draft.new.document");
    } catch { /* Draft is already handed off. */ }
  }
  async function start() {
    if (live() || window.EzerbotVoiceMode?.active()) return;
    if (panels.meetingActive?.()) { panels.notify(errors.meeting_busy); return; }
    reset();
    try {
      session = { token: crypto.randomUUID(), key: key(context), memo: memoMode.checked };
      phase = "authorizing"; changed();
      const rev = revision;
      const result = await bridge("start");
      if (rev === revision) await accept(result, rev);
    } catch (error) { phase = "idle"; panels.notify(error.message || "La dictée n’a pas pu démarrer."); changed(); }
  }
  async function saveMemo(result, rev) {
    phase = "saving"; pendingMemo = result; changed();
    try {
      // A thread is created only for a memo ready to keep, never for a refusal,
      // a cancellation or a failed transcription.
      await ensureConversation();
      if (rev !== revision) return;
      result.conversation_id = context.conversation.id;
      const memo = await panels.api("/api/voice/memos", { conversation_id: context.conversation.id, upload_id: result.token, data: result.data, transcript: result.transcript, language: result.language || language() });
      if (rev !== revision) {
        await panels.api("/api/voice/remove", {id:memo.id,conversation_id:result.conversation_id}).catch(() => {});
        return;
      }
      memo.conversation_id = context.conversation.id;
      localMemos.set(memo.id, memo);
      if (!memoItems.some((m) => m.id === memo.id)) memoItems.push(memo);
      pendingMemo = null;
      finish(result, true);
    } catch (error) { if (rev === revision) { phase = "upload_failed"; panels.notify(error.message); changed(); } }
  }
  function finish(result, memo = false) {
    panels.appendDraft(result.transcript);
    // A kept memo is visible as its chip; only the duration cap still needs a note.
    phase = result.too_long ? "limited" : memo ? "idle" : "ready";
    bridge("cancel").catch(() => {}); // Wipe temporary WAV and recognition task.
    changed();
  }
  async function accept(result, rev) {
    if (rev !== revision || !session || session.key !== key(context)) return;
    if (result.token && result.token !== session.token) return;
    phase = result.status;
    timer.textContent = `${Math.floor((result.elapsed_ms || 0) / 60000)}:${String(Math.floor((result.elapsed_ms || 0) / 1000) % 60).padStart(2,"0")} / 1:00`;
    const level = Math.max(0, Math.min(1, result.level || 0));
    meter.style.setProperty("--level", String(level)); meter.setAttribute("aria-valuenow", level.toFixed(2));
    if (phase === "ready" && !processing) {
      processing = true;
      if (!result.transcript?.trim()) { phase = "failed"; processing = false; changed(); return; }
      if (session.memo) await saveMemo(result, rev);
      else finish(result);
      return;
    }
    if (["stale", "invalid"].includes(phase)) { reset(); panels.notify("Cette dictée n’est plus active. Recommencez."); }
    changed();
    if (live()) timeout = setTimeout(() => poll(rev), 120);
  }
  async function poll(rev) {
    if (rev !== revision) return;
    try { await accept(await bridge("status"), rev); }
    catch { if (rev === revision) { reset(); panels.notify("La connexion à la dictée est interrompue. Recommencez."); changed(); } }
  }
  async function stop() {
    clearTimeout(timeout);
    if (phase !== "recording") return;
    // Invalidate a status request already in flight. Its old « recording » or
    // « idle » snapshot must not overwrite the stop/transcript receipt.
    const rev = ++revision;
    phase = "transcribing"; changed();
    try { await accept(await bridge("stop"), rev); }
    catch { if (rev === revision) { phase = "failed"; changed(); } }
  }
  panels.register({
    id: "voice",
    composer: {
      mount(container, ctx) {
        context = ctx;
        control = node("div", "voice-controls");
        const label = node("label", "voice-mode"); label.title = "Mémo vocal"; label.setAttribute("aria-label", "Mémo vocal"); memoMode = node("input"); memoMode.type = "checkbox"; memoMode.setAttribute("aria-label", "Mémo vocal");
        label.append(memoMode, document.createTextNode("Mémo vocal"));
        mic = node("button", "icon-button voice-mic"); mic.id = "voice-microphone"; mic.type = "button";
        mic.onclick = () => phase === "recording" ? stop() : start();
        control.append(label, mic); window.EzerbotVoiceMode?.mountToggle?.(control);
        document.getElementById("send-button").before(control);
        region = node("section", "voice-region"); region.setAttribute("aria-label", "Dictée et mémos vocaux");
        row = node("div", "voice-status-row"); statusText = node("span", "voice-status"); statusText.setAttribute("role", "status");
        // Styled bar, not <meter>: the native gauge ignores the accent color.
        meter = node("div", "voice-level"); meter.setAttribute("role", "meter"); meter.setAttribute("aria-valuemin", "0"); meter.setAttribute("aria-valuemax", "1"); meter.setAttribute("aria-label", "Niveau du microphone"); meter.append(node("span"));
        timer = node("span", "voice-time"); timer.setAttribute("aria-label", "Durée de l’enregistrement");
        cancel = node("button", "attachment-retry", "Annuler"); cancel.type = "button"; cancel.onclick = () => { reset(); changed(); };
        retry = node("button", "attachment-retry", "Réessayer"); retry.type = "button"; retry.onclick = async () => {
          processing = false;
          if (phase === "upload_failed") return saveMemo(pendingMemo, revision);
          const rev = ++revision; phase = "transcribing"; changed();
          try { await accept(await bridge("retry"), rev); } catch { phase = "failed"; changed(); }
        };
        settings = node("button", "attachment-retry", "Ouvrir Réglages Système"); settings.type = "button"; settings.onclick = () => bridge("settings").catch(() => panels.notify("Ouvrez Réglages Système › Confidentialité et sécurité › Microphone."));
        row.append(statusText, meter, timer, cancel, retry, settings);
        list = node("div", "voice-memo-list"); region.append(row, list); container.append(region);
        document.addEventListener("keydown", (event) => { if (event.key === "Escape" && phase === "recording") { event.preventDefault(); event.stopImmediatePropagation(); stop(); } }, true);
        window.addEventListener("beforeunload", reset);
        window.addEventListener("grokbot:meeting-change", render);
        render();
      },
      blocked: () => live(),
      discard() { reset(); memoItems = []; render(); },
      onSent({extras}) { for (const id of extras.attachments || []) { consumed.add(id); localMemos.delete(id); } memoItems = memoItems.filter((m) => !consumed.has(m.id)); if (!live() && !panels.draft().trim()) phase = "idle"; render(); },
    },
    onState(state, ctx) {
      if (session && session.key !== key(ctx)) reset();
      context = ctx;
      const drafts = new Map([...(state.voice?.drafts || []), ...localMemos.values()].map((memo) => [memo.id, memo]));
      memoItems = [...drafts.values()].filter((m) => m.conversation_id === ctx.conversation?.id && !consumed.has(m.id));
      render();
    },
    sendExtras: () => ({ attachments: memoItems.map((m) => m.id) }),
  });
})();
