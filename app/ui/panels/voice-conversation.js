"use strict";
(() => {
  const panels = window.GrokbotPanels;
  // Voice mode captures and speaks on the Mac. Hide its composer control on iPhone.
  if (!panels || window.EzerbotIPhone?.remote) return;
  const el = (tag, cls, text) => { const n = document.createElement(tag); n.className = cls || ""; if (text) n.textContent = text; return n; };
  const errors = {
    mic_denied:"Accès au microphone refusé. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    speech_denied:"Reconnaissance vocale refusée. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    offline_unavailable:"La transcription hors ligne française est indisponible. Installez les données de Dictée dans Réglages Système.",
    no_microphone:"Aucun microphone disponible. Branchez un microphone, puis réessayez.",
    audio_failed:"Le microphone a été déconnecté ou le traitement audio est indisponible. Rebranchez-le, puis relancez le mode voix.",
    failed:"La transcription a échoué. Le mode voix est arrêté ; relancez-le pour réessayer.",
    tts_unavailable:"La voix de ce bot est indisponible sur ce Mac. Choisissez une autre voix dans les réglages du bot.",
    unavailable:"Le mode voix est disponible dans l’application Mac Ezerbot.",
    invalid:"La synthèse vocale a échoué. Vérifiez la voix et la vitesse dans les réglages du bot.",
    stale:"Cette session vocale n’est plus active. Relancez le mode voix.",
  };
  let ctx, token = "", settingsToken = "", conversationId, scopeKey, reader, epoch = 0, timer, ticking = false;
  const contextKey = context => context.conversation?.id || `new:${context.bot?.id || "grokbot"}`;
  let button, bar, label, transcript, heldNotice, stop, mute, leave, systemSettings;
  let phase = "idle", detail = "", liveText = "", lastUtterance = 0, lastBarge = 0, paused = false, sending = false;
  // Mirror of the bridge `muted`, read from its answers: the click sends the command, the screen follows the reply.
  let muted = false;
  let inputHeld = false, followResumedReply = false;
  let voicesLoaded = false;
  const botViews = new Set();
  const noVoice = {voice:"",speed:1,language:"",chosen:false};
  // Each bot speaks with its own voice, speed and language, whichever conversation it is in.
  const botVoice = botId => ctx?.state?.voice?.bots?.[botId || ctx.bot?.id] || noVoice;
  const active = () => Boolean(token);
  let lastActive = false;
  errors.microphone_busy = "Une autre capture utilise déjà le microphone. Arrêtez-la avant de continuer.";
  window.EzerbotVoiceMode = { active, stopSpeech, mountToggle(container) { if (button) container.prepend(button); } };
  async function bridge(command, extra = {}) {
    const body = {command, token:command === "settings" ? settingsToken : token, ...extra};
    const native = window.webkit?.messageHandlers?.ezerbotVoiceSession;
    if (native) return native.postMessage(body);
    if (ctx?.state?.voice?.debug_fake) return panels.api("/api/voice/session/debug", body);
    return {status:"unavailable"};
  }
  function render() {
    if (!button) return;
    button.disabled = !active() && (Boolean(panels.meetingActive?.() || window.EzerbotVoiceCall?.active?.()) || !ctx?.connected || ctx?.conversation?.kind === "task");
    button.setAttribute("aria-pressed", String(active()));
    button.title = window.EzerbotVoiceCall?.active?.() && !active() ? "Indisponible pendant l’appel vocal" : panels.meetingActive?.() && !active() ? "Indisponible pendant l’enregistrement d’une réunion" : button.disabled ? "Ouvrez une conversation pour utiliser le mode voix" : "Mode voix · parler et écouter les réponses sur ce Mac";
    bar.hidden = phase === "idle";
    // A muted microphone wins over the listening and speaking labels: neither invites the user to speak.
    const shown = muted && ["listening","speaking"].includes(phase) ? "muted" : phase;
    label.textContent = detail || ({authorizing:"Autorisation du microphone…",muted:"Micro coupé · le bot ne vous entend pas",listening:"À l’écoute · parlez pour envoyer",thinking:"Ezerbot réfléchit…",speaking:"Ezerbot parle · Échap pour interrompre",waiting:"En attente de vous · répondez dans la carte",sending:"Envoi de la transcription…"}[shown] || "");
    bar.dataset.phase = shown;
    bar.dataset.tone = detail ? "danger" : "";
    transcript.textContent = muted ? "" : liveText || "Votre transcription apparaîtra ici. L’audio reste sur ce Mac.";
    transcript.hidden = muted || Boolean(detail) && !liveText;
    transcript.scrollTop = transcript.scrollHeight;
    heldNotice.hidden = !inputHeld || !window.GrokbotChat.voiceNeedsAction(ctx.conversation?.messages || []);
    mute.hidden = !active();
    mute.disabled = phase === "authorizing";
    mute.setAttribute("aria-pressed", String(muted));
    const muteName = muted ? "Réactiver le micro" : "Couper le micro";
    mute.setAttribute("aria-label", muteName); mute.title = `${muteName} (⌘⇧M)`;
    stop.hidden = !active() || !["speaking","thinking","sending"].includes(phase);
    leave.textContent = active() ? "Quitter" : "Fermer";
    systemSettings.hidden = !["mic_denied","speech_denied","offline_unavailable"].includes(phase);
    window.EzerbotDictation?.refresh();
    if (active() !== lastActive) { lastActive = active(); window.dispatchEvent?.(new Event("grokbot:voice-mode-change")); }
  }
  function exit(clear = true) {
    clearTimeout(timer); epoch++;
    if (token) bridge("cancel").catch(() => {});
    token = ""; reader = null; conversationId = null; sending = false; paused = false;
    inputHeld = false; followResumedReply = false; muted = false;
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
  async function toggleMute() {
    if (!active() || phase === "authorizing") return;
    const rev = epoch, receipt = await bridge(muted ? "unmute" : "mute");
    if (rev !== epoch) return;
    if (errors[receipt.status]) { fail(receipt.status); return; }
    muted = Boolean(receipt.muted); if (muted) liveText = "";
    // Unmuting listens again to the chosen microphone, which may have gone while muted.
    if (!muted && receipt.microphone_fallback) window.EzerbotMicrophone?.fellBack(window.EzerbotMicrophone.current());
    render();
  }
  const muteToggled = () => toggleMute().catch(error => panels.notify(error.message));
  async function interrupt() {
    if (!active()) return;
    await stopSpeech();
    await stopRun();
  }
  async function start() {
    if (active()) { exit(); return; }
    if (button.disabled) return;
    if (panels.meetingActive?.() || window.EzerbotVoiceCall?.active?.()) { panels.notify(errors.microphone_busy); return; }
    window.EzerbotDictation?.cancel();
    token = crypto.randomUUID(); settingsToken = token; conversationId = ctx.conversation?.id; scopeKey = contextKey(ctx);
    reader = window.GrokbotChat.createVoiceReader(ctx.conversation?.messages || []);
    phase = "authorizing"; detail = ""; liveText = ""; lastUtterance = 0; lastBarge = 0; paused = false;
    inputHeld = false; followResumedReply = false; muted = false;
    const rev = ++epoch;
    render(); panels.composerChanged();
    try {
      const requested = window.EzerbotMicrophone?.current() ?? "";
      const result = await bridge("start", {microphone:requested});
      if (rev !== epoch) return;
      if (result.microphone_fallback) window.EzerbotMicrophone?.fellBack(requested);
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
      muted = Boolean(result.muted);
      if (muted) liveText = ""; else if (result.transcript) liveText = result.transcript;
      const needsAction = window.GrokbotChat.voiceNeedsAction(ctx.conversation?.messages || []);
      if (result.barged > lastBarge) {
        lastBarge = result.barged;
        // Speaking is not a decision on an approval/form/secret card.
        if (!needsAction) { reader.interrupt(ctx.conversation?.messages || []); await stopRun(); }
        if (rev !== epoch) return;
      }
      if (!needsAction) inputHeld = false;
      if (needsAction !== paused) {
        followResumedReply = !needsAction;
        paused = needsAction;
        await bridge(paused ? "pause" : "resume");
        if (rev !== epoch) return;
      }
      if (result.utterance_id > lastUtterance) {
        lastUtterance = result.utterance_id;
        if (needsAction) {
          inputHeld = true;
        } else if (result.final_text?.trim() && !sending) {
          liveText = result.final_text; phase = "sending"; sending = true; render();
          reader.nextTurn();
          try { await ensureConversation(rev); if (rev === epoch) await panels.sendVoice(result.final_text, conversationId); }
          finally { sending = false; }
          if (rev !== epoch) return;
        }
      }
      const chunks = reader.readByAuthor(ctx.conversation?.messages || [], needsAction);
      for (const {text, botId} of chunks) {
        const settings = botVoice(botId);
        const receipt = await bridge("speak", {text,voice:settings.voice,speed:settings.speed,language:settings.language,barge:lastBarge});
        if (rev !== epoch) return;
        if (errors[receipt.status]) { fail(receipt.status); return; }
        if (receipt.barged > lastBarge) break; // Native STT already interrupted queued speech.
      }
      if (chunks.length && followResumedReply) {
        followResumedReply = false;
        panels.followVoiceReply?.(conversationId);
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
    if (!voicesLoaded && (ctx.state?.voice?.debug_fake || window.webkit?.messageHandlers?.ezerbotVoiceSession)) loadVoices();
    for (const view of botViews) view.render();
    render();
  }
  // The Mac says which voices it can pronounce; the engine hands one to each bot without a choice.
  async function loadVoices() {
    voicesLoaded = true;
    try {
      const result = await bridge("options");
      await panels.api("/api/voice/voices", {voices:result.voices || []});
      await panels.refresh();
    } catch { voicesLoaded = false; for (const view of botViews) view.say("La liste des voix est indisponible. Rouvrez l’application pour réessayer."); }
  }
  const languageName = code => { try { return new Intl.DisplayNames(["fr"], {type:"language"}).of(code) || code; } catch { return code; } };
  const languageOf = voice => String(voice.language || "").split("-")[0].toLowerCase();
  const speedText = speed => `${Number(speed).toLocaleString("fr-CA")}×`;
  function botSection(bot, container, context) {
    ctx = ctx || context;
    container.className = "setting-section voice-settings voice-bot-settings";
    container.append(el("h3","","Voix"),el("p","","Le mode voix de ce bot parle avec cette voix, cette vitesse et cette langue. Transcription et lecture restent sur ce Mac ; l’audio n’est jamais transmis au modèle."));
    if (!bot?.id || bot.deleted) { container.append(el("p","voice-settings-notice","Enregistrez d’abord ce bot : une voix lui sera attribuée.")); return; }
    const current = el("p","voice-bot-current"); current.setAttribute("role","status");
    const voiceLabel = el("label","","Voix"); const voiceSelect = el("select","text-input"); voiceSelect.name = "bot-voice"; voiceSelect.setAttribute("aria-label","Voix du bot"); voiceLabel.append(voiceSelect);
    const languageLabel = el("label","","Langue de la voix"); const languageSelect = el("select","text-input"); languageSelect.name = "bot-voice-language"; languageSelect.setAttribute("aria-label","Langue de la voix"); languageLabel.append(languageSelect);
    const speedLabel = el("label","","Vitesse · de 0,5 à 2"); const speedInput = el("input","text-input"); speedInput.type = "number"; speedInput.name = "bot-voice-speed"; speedInput.min = "0.5"; speedInput.max = "2"; speedInput.step = "0.1"; speedInput.setAttribute("aria-label","Vitesse de la voix"); speedLabel.append(speedInput);
    const hint = el("p","voice-bot-hint");
    const notice = el("p","voice-settings-notice"); notice.setAttribute("role","status");
    const save = el("button","secondary-button","Enregistrer la voix"); save.type = "button";
    let dirty = false;
    const saved = () => ctx.state?.voice?.bots?.[bot.id] || noVoice;
    const voices = () => ctx.state?.voice?.voices || [];
    const fill = (select, entries, value) => {
      select.replaceChildren(...entries.map(([id, text]) => { const option = el("option","",text); option.value = id; return option; }));
      select.value = entries.some(([id]) => id === value) ? value : entries[0][0];
    };
    const fillVoices = (language, value) => fill(voiceSelect, [["","Voix du Mac par défaut"],...voices().filter(v => !language || languageOf(v) === language).map(v => [v.id,`${v.name} · ${v.language}`])], value);
    function render() {
      const setting = saved(), list = voices();
      const codes = [...new Set(list.map(languageOf))].sort((a,b) => (b === "fr") - (a === "fr") || languageName(a).localeCompare(languageName(b),"fr"));
      if (!dirty) {
        fill(languageSelect, [["","Auto"],...codes.map(code => [code,languageName(code)])], setting.language);
        fillVoices(languageSelect.value, setting.voice);
        speedInput.value = setting.speed;
      }
      const shown = list.find(v => v.id === setting.voice);
      current.textContent = !list.length ? "Les voix sont disponibles dans l’application Mac Ezerbot."
        : `Voix de ${bot.name || "ce bot"} : ${shown ? shown.name : "voix du Mac par défaut"} · ${speedText(setting.speed)} · ${setting.language ? languageName(setting.language) : "langue Auto"}`;
      hint.textContent = !list.length ? ""
        : list.length === 1 ? "Une seule voix est installée sur ce Mac : tous les bots la partagent."
        : setting.chosen ? "" : "Voix attribuée automatiquement, différente de celle des autres bots tant que c’est possible.";
      voiceSelect.disabled = languageSelect.disabled = !list.length; save.disabled = !list.length;
    }
    languageSelect.onchange = () => { dirty = true; fillVoices(languageSelect.value, voiceSelect.value); };
    voiceSelect.onchange = () => { dirty = true; };
    speedInput.oninput = () => { dirty = true; };
    save.onclick = async () => {
      save.disabled = true; notice.textContent = "";
      try {
        await panels.api("/api/voice/bot", {bot_id:bot.id,voice:voiceSelect.value,speed:Number(speedInput.value),language:languageSelect.value});
        dirty = false; await panels.refresh(); notice.textContent = "Voix enregistrée. Le mode voix de ce bot l’utilisera dès la prochaine phrase.";
      } catch (error) { notice.textContent = error.message; }
      finally { save.disabled = false; }
    };
    container.append(current,hint,voiceLabel,languageLabel,speedLabel,save,notice);
    const view = {render, say(text) { notice.textContent = text; }};
    botViews.add(view); render();
    return () => botViews.delete(view);
  }
  panels.register({
    id:"voice-conversation",
    botSection,
    composer: {
      mount(container, context) {
        ctx = context;
        button = el("button","voice-session-toggle"); button.type = "button"; button.onclick = start;
        button.setAttribute("aria-label","Mode voix");
        const icon = el("span","voice-session-icon"); icon.setAttribute("aria-hidden","true");
        icon.innerHTML = '<svg class="icon" viewBox="0 0 24 24"><path d="M3 18v-6a9 9 0 0 1 18 0v6"/><path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3z"/><path d="M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"/></svg>';
        button.append(icon,el("span","voice-session-label","Mode voix"));
        const controls = document.querySelector?.(".voice-controls");
        if (controls) controls.prepend(button);
        else document.getElementById("send-button").before(button); // Dictation mounts this group later in the panel load order.
        bar = el("section","voice-session-bar"); bar.setAttribute("aria-label","Conversation vocale");
        const row = el("div","voice-session-row"); label = el("strong","voice-session-state"); label.setAttribute("role","status");
        stop = el("button","attachment-retry","Interrompre"); stop.type = "button"; stop.onclick = () => interrupt().catch(error => fail("failed",error.message));
        mute = el("button","icon-button voice-mute-toggle"); mute.type = "button"; mute.setAttribute("aria-keyshortcuts","Meta+Shift+M");
        mute.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="2" width="6" height="13" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/><path class="voice-mute-slash" d="M3 3l18 18"/></svg>';
        mute.onclick = muteToggled;
        leave = el("button","attachment-retry","Quitter"); leave.type = "button"; leave.onclick = () => exit();
        systemSettings = el("button","attachment-retry","Ouvrir Réglages Système"); systemSettings.type = "button"; systemSettings.onclick = () => bridge("settings", {reason:phase}).catch(() => panels.notify("Ouvrez Réglages Système › Confidentialité et sécurité."));
        row.append(label,mute,stop,leave,systemSettings);
        transcript = el("p","voice-live-transcript");
        heldNotice = el("p","voice-held-notice","Transcription non envoyée : répondez d’abord dans la carte."); heldNotice.setAttribute("role","status");
        bar.append(row,transcript,heldNotice); container.append(bar);
        document.addEventListener("keydown", event => {
          if (event.key === "Escape" && active()) { event.preventDefault(); event.stopImmediatePropagation(); interrupt().catch(error => fail("failed",error.message)); }
          // ⌘⇧M: free in the app (app.js takes N, F, K, ⇧W; the dictation takes D; the native menu has no M).
          if (event.metaKey && event.shiftKey && !event.ctrlKey && !event.altKey && !event.repeat && event.key?.toLowerCase() === "m" && active()) {
            event.preventDefault(); event.stopImmediatePropagation(); muteToggled();
          }
        },true);
        window.addEventListener("beforeunload", () => exit());
        window.addEventListener("pagehide", () => exit());
        window.addEventListener("grokbot:meeting-change", render);
        window.addEventListener("grokbot:voice-call-change", render);
        render();
      },
      discard() { if (active()) exit(); },
    },
    onState(state, context) { observe(context); },
    onConnection(connected) { if (!connected && active()) fail("failed","La connexion au moteur local est interrompue. Le mode voix est arrêté."); },
    wantsPolling: active,
  });
})();
