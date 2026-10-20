"use strict";
(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  const el = (tag, cls, text) => { const n = document.createElement(tag); n.className = cls || ""; if (text) n.textContent = text; return n; };
  const errors = {
    mic_denied:"Accès au microphone refusé. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    speech_denied:"Reconnaissance vocale refusée. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    offline_unavailable:"La transcription hors ligne française est indisponible. Installez les données de Dictée dans Réglages Système.",
    no_microphone:"Aucun microphone disponible. Branchez un microphone, puis réessayez.",
    audio_failed:"Le microphone a été déconnecté ou le traitement audio est indisponible. Rebranchez-le, puis relancez le mode voix.",
    failed:"La transcription a échoué. Le mode voix est arrêté ; relancez-le pour réessayer.",
    tts_unavailable:"Cette voix française est indisponible sur ce Mac. Choisissez une autre voix dans Paramètres.",
    unavailable:"Le mode voix est disponible dans l’application Mac Ezerbot.",
    invalid:"La synthèse vocale a échoué. Vérifiez la voix et la vitesse dans Paramètres.",
    stale:"Cette session vocale n’est plus active. Relancez le mode voix.",
  };
  let ctx, token = "", settingsToken = "", conversationId, scopeKey, reader, epoch = 0, timer, ticking = false;
  const contextKey = context => context.conversation?.id || `new:${context.bot?.id || "grokbot"}`;
  let button, bar, label, transcript, stop, leave, systemSettings, voiceSelect, speedInput, speedLabel, notice;
  let phase = "idle", detail = "", liveText = "", lastUtterance = 0, lastBarge = 0, paused = false, sending = false;
  let voicesLoaded = false, settingsDirty = false;
  const active = () => Boolean(token);
  window.EzerbotVoiceMode = { active, stopSpeech };
  async function bridge(command, extra = {}) {
    const body = {command, token:command === "settings" ? settingsToken : token, ...extra};
    const native = window.webkit?.messageHandlers?.ezerbotVoiceSession;
    if (native) return native.postMessage(body);
    if (ctx?.state?.voice?.debug_fake) return panels.api("/api/voice/session/debug", body);
    return {status:"unavailable"};
  }
  function render() {
    if (!button) return;
    button.disabled = !active() && (!ctx?.connected || ctx?.conversation?.kind === "task");
    button.setAttribute("aria-pressed", String(active()));
    button.title = button.disabled ? "Ouvrez une conversation pour utiliser le mode voix" : "Parler et écouter les réponses sur ce Mac";
    bar.hidden = phase === "idle";
    label.textContent = detail || ({authorizing:"Autorisation du microphone…",listening:"À l’écoute · parlez pour envoyer",thinking:"Ezerbot réfléchit…",speaking:"Ezerbot parle · Échap pour interrompre",waiting:"En attente de vous · répondez dans la carte",sending:"Envoi de la transcription…"}[phase] || "");
    bar.dataset.phase = phase;
    bar.dataset.tone = detail ? "danger" : "";
    transcript.textContent = liveText || "Votre transcription apparaîtra ici. L’audio reste sur ce Mac.";
    stop.hidden = !active() || phase === "authorizing";
    leave.textContent = active() ? "Quitter" : "Fermer";
    systemSettings.hidden = !["mic_denied","speech_denied","offline_unavailable"].includes(phase);
  }
  function exit(clear = true) {
    clearTimeout(timer); epoch++;
    if (token) bridge("cancel").catch(() => {});
    token = ""; reader = null; conversationId = null; sending = false; paused = false;
    if (clear) { phase = "idle"; detail = ""; liveText = ""; scopeKey = null; }
    render(); panels.composerChanged();
  }
  function fail(status, message) {
    phase = status; detail = message || errors[status] || errors.failed;
    exit(false);
  }
  const pendingReply = message => window.GrokbotChat.isPendingReply?.(message, ctx?.conversation?.kind === "group") ?? message.state === "streaming";
  async function stopRun() {
    const conversation = ctx?.conversation;
    if (window.GrokbotChat.voiceNeedsAction(conversation?.messages || [])) return;
    const reply = conversation?.messages.find(pendingReply);
    if (reply && conversation.id === conversationId) {
      await panels.api("/api/stop", {conversation_id:conversationId,message_id:conversation.kind === "group" ? "groupe" : reply.id.replace(/^reply-/,"")});
      await panels.refresh();
    }
  }
  async function stopSpeech() {
    if (!active()) return;
    if (!window.GrokbotChat.voiceNeedsAction(ctx.conversation?.messages || [])) reader?.interrupt(ctx.conversation?.messages || []);
    const receipt = bridge("interrupt");
    phase = "listening"; render();
    await receipt;
  }
  async function interrupt() {
    if (!active()) return;
    await stopSpeech();
    await stopRun();
  }
  async function start() {
    if (active()) { exit(); return; }
    if (button.disabled) return;
    window.EzerbotDictation?.cancel();
    token = crypto.randomUUID(); settingsToken = token; conversationId = ctx.conversation?.id; scopeKey = contextKey(ctx);
    reader = window.GrokbotChat.createVoiceReader(ctx.conversation?.messages || []);
    phase = "authorizing"; detail = ""; liveText = ""; lastUtterance = 0; lastBarge = 0; paused = false;
    const rev = ++epoch;
    render(); panels.composerChanged();
    try {
      const result = await bridge("start");
      if (rev !== epoch) return;
      if (errors[result.status]) { fail(result.status); return; }
      phase = result.status; render(); schedule(rev);
    } catch (error) { if (rev === epoch) fail("failed", error.message); }
  }
  async function ensureConversation(rev) {
    if (conversationId) return;
    const {conversation} = await panels.api("/api/conversations", {bot_id:ctx.bot?.id});
    await panels.refresh();
    if (rev !== epoch) return;
    const draft = panels.draft?.();
    if (draft) {
      localStorage.setItem(`grokbot.draft.${conversation.id}`, draft);
      localStorage.setItem(`grokbot.draft.${conversation.id}.document`, JSON.stringify(panels.draftDocument()));
    }
    // Bind before openConversation publishes the new selection to panel observers.
    conversationId = conversation.id; scopeKey = conversation.id;
    if (!panels.openConversation(conversation.id)) throw new Error("Ouvrez la conversation créée pour poursuivre le mode voix.");
    try { localStorage.removeItem("grokbot.draft.new"); localStorage.removeItem("grokbot.draft.new.document"); } catch { /* The draft is already handed off. */ }
  }
  function schedule(rev) { clearTimeout(timer); if (active() && rev === epoch) timer = setTimeout(() => tick(rev), 100); }
  async function tick(rev) {
    if (!active() || rev !== epoch) return;
    if (ticking) { schedule(rev); return; }
    ticking = true;
    try {
      const result = await bridge("status");
      if (rev !== epoch) return;
      if (errors[result.status] || result.status === "idle") { fail(result.status); return; }
      if (result.status === "authorizing") { phase = "authorizing"; render(); return; }
      if (result.token !== token) { fail("stale"); return; }
      if (result.transcript) liveText = result.transcript;
      const needsAction = window.GrokbotChat.voiceNeedsAction(ctx.conversation?.messages || []);
      if (result.barged > lastBarge) {
        lastBarge = result.barged;
        // Speaking is not a decision on an approval/form/secret card.
        if (!needsAction) { reader.interrupt(ctx.conversation?.messages || []); await stopRun(); }
        if (rev !== epoch) return;
      }
      if (needsAction !== paused) {
        paused = needsAction;
        await bridge(paused ? "pause" : "resume");
        if (rev !== epoch) return;
      }
      if (result.utterance_id > lastUtterance) {
        lastUtterance = result.utterance_id;
        if (needsAction) {
          liveText = "Transcription non envoyée : répondez d’abord dans la carte.";
        } else if (result.final_text?.trim() && !sending) {
          liveText = result.final_text; phase = "sending"; sending = true; render();
          reader.nextTurn();
          try { await ensureConversation(rev); if (rev === epoch) await panels.sendVoice(result.final_text, conversationId); }
          finally { sending = false; }
          if (rev !== epoch) return;
        }
      }
      const chunks = reader.read(ctx.conversation?.messages || [], needsAction);
      const settings = ctx.state.voice?.settings || {voice:"",speed:1};
      for (const text of chunks) {
        const receipt = await bridge("speak", {text,...settings,barge:lastBarge});
        if (rev !== epoch) return;
        if (errors[receipt.status]) { fail(receipt.status); return; }
        if (receipt.barged > lastBarge) break; // Native STT already interrupted queued speech.
      }
      const thinking = (ctx.conversation?.messages || []).some(pendingReply);
      phase = needsAction ? "waiting" : result.speaking || chunks.length ? "speaking" : thinking ? "thinking" : "listening";
      render();
    } catch (error) { if (rev === epoch) fail("failed",error.message); }
    finally { ticking = false; schedule(rev); }
  }
  function observe(next) {
    if (scopeKey && scopeKey !== contextKey(next)) exit();
    ctx = next;
    if (!settingsDirty && ctx.state?.voice?.settings && voiceSelect) {
      voiceSelect.value = ctx.state.voice.settings.voice;
      speedInput.value = ctx.state.voice.settings.speed;
      speedLabel.textContent = `Vitesse · ${Number(speedInput.value).toLocaleString("fr-CA")}×`;
    }
    if (!voicesLoaded && (ctx.state?.voice?.debug_fake || window.webkit?.messageHandlers?.ezerbotVoiceSession)) loadVoices();
    render();
  }
  async function loadVoices() {
    voicesLoaded = true;
    try {
      const result = await bridge("options");
      for (const voice of result.voices || []) { const option = el("option","",`${voice.name} · ${voice.language}`); option.value = voice.id; voiceSelect.append(option); }
      voiceSelect.value = ctx.state.voice?.settings?.voice || "";
    } catch { notice.textContent = "La liste des voix est indisponible. Rouvrez l’application pour réessayer."; }
  }
  panels.register({
    id:"voice-conversation",
    settingsSection(container, context) {
      ctx = context; container.className = "setting-section voice-settings";
      container.append(el("h3","","Voix"),el("p","","Transcription et lecture françaises sur ce Mac. En mode voix, vos paroles sont envoyées automatiquement ; l’audio n’est jamais transmis au modèle."));
      const voiceLabel = el("label","","Voix française"); voiceSelect = el("select","text-input"); voiceSelect.name = "conversation-voice"; voiceSelect.setAttribute("aria-label","Voix française");
      const automatic = el("option","","Français · voix du Mac"); automatic.value = ""; voiceSelect.append(automatic); voiceLabel.append(voiceSelect);
      const speedWrapper = el("label"); speedLabel = el("span","","Vitesse · 1×"); speedInput = el("input"); speedInput.type = "range"; speedInput.name = "conversation-voice-speed"; speedInput.min = "0.5"; speedInput.max = "1.5"; speedInput.step = "0.1"; speedInput.value = "1"; speedInput.setAttribute("aria-label","Vitesse de lecture"); speedWrapper.append(speedLabel,speedInput);
      voiceSelect.onchange = () => { settingsDirty = true; };
      speedInput.oninput = () => { settingsDirty = true; speedLabel.textContent = `Vitesse · ${Number(speedInput.value).toLocaleString("fr-CA")}×`; };
      notice = el("p","voice-settings-notice"); notice.setAttribute("role","status");
      const save = el("button","secondary-button","Enregistrer la voix"); save.type = "button";
      save.onclick = async () => {
        save.disabled = true;
        try { await panels.api("/api/voice/settings", {voice:voiceSelect.value,speed:Number(speedInput.value)}); settingsDirty = false; await panels.refresh(); notice.textContent = "Voix enregistrée. Les prochaines phrases utiliseront ces réglages."; }
        catch (error) { notice.textContent = error.message; }
        finally { save.disabled = false; }
      };
      container.append(voiceLabel,speedWrapper,save,notice);
    },
    composer: {
      mount(container, context) {
        ctx = context;
        button = el("button","voice-session-toggle","Mode voix"); button.type = "button"; button.onclick = start;
        document.getElementById("send-button").before(button);
        bar = el("section","voice-session-bar"); bar.setAttribute("aria-label","Conversation vocale");
        const row = el("div","voice-session-row"); label = el("strong","voice-session-state"); label.setAttribute("role","status");
        stop = el("button","attachment-retry","Interrompre"); stop.type = "button"; stop.onclick = () => interrupt().catch(error => fail("failed",error.message));
        leave = el("button","attachment-retry","Quitter"); leave.type = "button"; leave.onclick = () => exit();
        systemSettings = el("button","attachment-retry","Ouvrir Réglages Système"); systemSettings.type = "button"; systemSettings.onclick = () => bridge("settings", {reason:phase}).catch(() => panels.notify("Ouvrez Réglages Système › Confidentialité et sécurité."));
        row.append(label,stop,leave,systemSettings);
        transcript = el("p","voice-live-transcript"); bar.append(row,transcript); container.append(bar);
        document.addEventListener("keydown", event => {
          if (event.key === "Escape" && active()) { event.preventDefault(); event.stopImmediatePropagation(); interrupt().catch(error => fail("failed",error.message)); }
        },true);
        window.addEventListener("beforeunload", () => exit());
        window.addEventListener("pagehide", () => exit());
        render();
      },
      discard() { if (active()) exit(); },
    },
    onState(state, context) { observe(context); },
    onConnection(connected) { if (!connected && active()) fail("failed","La connexion au moteur local est interrompue. Le mode voix est arrêté."); },
    wantsPolling: active,
  });
})();
