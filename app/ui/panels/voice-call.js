"use strict";
(() => {
  const panels = window.GrokbotPanels;
  // The call starts from the Mac. The phone only reads its transcript once it is in the thread.
  if (!panels || window.EzerbotIPhone?.remote) return;
  const el = (tag, cls, text) => { const n = document.createElement(tag); n.className = cls || ""; if (text) n.textContent = text; return n; };
  const disclosure = "L’audio de cet appel est transmis au fournisseur de voix le temps de l’appel. Il n’est pas gardé sur le Mac.";
  const errors = {
    microphone_busy: "Une autre capture utilise déjà le microphone. Arrêtez-la avant de continuer.",
    mic_denied: "Accès au microphone refusé. Autorisez Ezerbot dans Réglages Système, puis réessayez.",
    no_microphone: "Aucun microphone disponible. Branchez un microphone, puis réessayez.",
    secret_refused: "Le fournisseur de voix a refusé d’ouvrir l’appel. Relancez l’appel pour réessayer.",
    unavailable: "L’appel vocal est disponible dans l’application Mac Ezerbot.",
    failed: "L’appel vocal a été interrompu. Relancez-le pour réessayer.",
    stale: "Cet appel vocal n’est plus actif. Relancez-le.",
  };
  const states = {
    authorizing: "Ouverture de l’appel…",
    muted: "Micro coupé · le bot ne vous entend pas",
    listening: "Appel en cours · parlez, le bot vous écoute",
    speaking: "Le bot parle · parlez ou Échap pour couper sa voix",
    waiting: "En attente de vous · répondez dans la carte",
    ending: "Fin de l’appel…",
  };
  let ctx, callId = "", scopeKey = "", conversationId = "", epoch = 0, timer, ticking = false;
  let button, menuItem, bar, label, notice, transcriptToggle, mute, interrupt, leave, transcript;
  let phase = "idle", detail = "", lines = [], partial = "", written = 0, shown = false, paused = false, speaking = false;
  // Mirror of the bridge `muted`, read from its answers: the click sends the command, the screen follows the reply.
  let muted = false;
  let lastActive = false, drawn = "";
  // Calls this view already ended: a state read older than the end must not end them twice.
  const finished = new Set();
  const active = () => Boolean(callId);
  const contextKey = context => context.conversation?.id || "";
  const available = () => Boolean(ctx?.state?.voice_call?.available);
  // One microphone: the call waits for the voice mode, the dictation and a local capture, and they wait for it.
  const busy = () => Boolean(window.EzerbotVoiceMode?.active?.() || window.EzerbotDictation?.active?.() || panels.meetingActive?.());
  const botVoice = () => ctx?.state?.voice?.bots?.[ctx.bot?.id] || {voice: "", speed: 1, language: ""};
  window.EzerbotVoiceCall = { active };
  panels.callActive = active;
  async function bridge(command, extra = {}) {
    const body = {command, token: callId, ...extra};
    const native = window.webkit?.messageHandlers?.ezerbotVoiceCall;
    if (native) return native.postMessage(body);
    if (ctx?.state?.voice_call?.debug_fake) return panels.api("/api/voice/call/debug", body);
    return {status: "unavailable"};
  }
  function render() {
    if (!button) return;
    const blocked = !active() && busy();
    button.disabled = !active() && (!ctx?.connected || !ctx?.conversation || ctx.conversation.kind === "task" || !available() || blocked);
    button.setAttribute("aria-pressed", String(active()));
    button.title = !available() && !active() ? "L’appel vocal n’est pas disponible : aucun fournisseur de voix n’est configuré sur ce Mac."
      : blocked ? "Indisponible pendant le mode voix, la dictée ou l’enregistrement d’une réunion"
      : button.disabled ? "Ouvrez une conversation pour lancer un appel vocal"
      : "Appel vocal · conversation en direct, l’audio est transmis au fournisseur de voix";
    menuItem.disabled = button.disabled;
    menuItem.setAttribute("aria-pressed", String(active()));
    menuItem.title = button.title;
    bar.hidden = phase === "idle";
    // A muted microphone wins over the listening and speaking labels: neither invites the user to speak.
    const key = muted && ["listening", "speaking"].includes(phase) ? "muted" : phase;
    bar.dataset.phase = key;
    bar.dataset.tone = detail ? "danger" : "";
    label.textContent = detail || states[key] || "";
    notice.hidden = Boolean(detail) && !active();
    transcriptToggle.hidden = !active();
    transcriptToggle.textContent = shown ? "Masquer le transcript" : "Afficher le transcript";
    transcriptToggle.setAttribute("aria-expanded", String(shown));
    mute.hidden = !active();
    mute.disabled = ["authorizing", "ending"].includes(phase);
    mute.setAttribute("aria-pressed", String(muted));
    const muteName = muted ? "Réactiver le micro" : "Couper le micro";
    mute.setAttribute("aria-label", muteName); mute.title = `${muteName} (⌘⇧M)`;
    interrupt.hidden = !active() || !speaking;
    leave.textContent = active() ? "Quitter" : "Fermer";
    transcript.hidden = !active() || !shown;
    // The transcript is rebuilt only when it changed, so scrolling back through it is not undone every tick.
    const shape = JSON.stringify([lines, partial, ctx?.bot?.name]);
    if (shape !== drawn) {
      drawn = shape;
      transcript.replaceChildren(...lines.map(line => {
        const row = el("p", "voice-call-line"); row.dataset.role = line.role;
        row.append(el("strong", "", line.role === "user" ? "Vous" : ctx?.bot?.name || "Le bot"), el("span", "", line.text));
        return row;
      }));
      if (partial) { const row = el("p", "voice-call-line voice-call-partial", partial); row.dataset.role = "user"; transcript.append(row); }
      if (!lines.length && !partial) transcript.append(el("p", "voice-call-empty", "Le transcript se remplira pendant l’appel."));
      transcript.scrollTop = transcript.scrollHeight;
    }
    window.EzerbotDictation?.refresh?.();
    if (active() !== lastActive) { lastActive = active(); window.dispatchEvent?.(new Event("grokbot:voice-call-change")); }
  }
  // Lines go to the Mac as they are heard, so closing the app only loses the sentence being spoken.
  // One writer at a time: a tick and Quit share `written`, and a line moves it on only once it is stored.
  let writing = Promise.resolve();
  function persist(id, spoken) {
    writing = writing.catch(() => {}).then(async () => {
      for (; written < spoken.length; written++) {
        const line = spoken[written];
        await panels.api("/api/voice/call/line", {id, seq: written, role: line.role, text: line.text});
      }
    });
    return writing;
  }
  function reset() {
    clearTimeout(timer); epoch++;
    callId = ""; conversationId = ""; scopeKey = ""; lines = []; partial = ""; written = 0; shown = false; paused = false; speaking = false; ticking = false; muted = false;
  }
  async function quit() {
    const id = callId;
    if (!id) { phase = "idle"; detail = ""; render(); return; }
    clearTimeout(timer); epoch++;
    // Leaving while the call is still opening: start() closes the call it just opened.
    if (id === "pending") { reset(); phase = "idle"; detail = ""; render(); panels.composerChanged(); return; }
    phase = "ending"; render();
    try { const last = await bridge("status"); lines = last.lines || lines; await persist(id, lines); } catch { /* The lines already written stay in the thread. */ }
    try { await bridge("cancel"); } catch { /* The session is gone either way. */ }
    finished.add(id);
    try { await panels.api("/api/voice/call/end", {id}); } catch (error) { panels.notify(error.message); }
    reset(); phase = "idle"; detail = "";
    render(); panels.composerChanged();
    await panels.refresh();
  }
  async function fail(status, message) {
    const id = callId;
    clearTimeout(timer); epoch++;
    if (id) {
      try { await bridge("cancel"); } catch { /* Already closed. */ }
      finished.add(id);
      try { await panels.api("/api/voice/call/end", {id}); } catch { /* The next start closes it. */ }
    }
    reset(); phase = "failed"; detail = message || errors[status] || errors.failed;
    render(); panels.composerChanged();
    await panels.refresh();
  }
  async function start() {
    if (active()) { await quit(); return; }
    if (button.disabled) return;
    if (busy()) { panels.notify(errors.microphone_busy); return; }
    const rev = ++epoch;
    conversationId = ctx.conversation.id; scopeKey = contextKey(ctx);
    phase = "authorizing"; detail = ""; lines = []; partial = ""; written = 0; shown = false; paused = false; speaking = false; muted = false;
    callId = "pending";
    render(); panels.composerChanged();
    let opened;
    try { opened = await panels.api("/api/voice/call/start", {conversation_id: conversationId}); }
    catch (error) { callId = ""; await fail("failed", error.message); return; }
    if (rev !== epoch) { try { await panels.api("/api/voice/call/end", {id: opened.call.id}); } catch { /* Closed at the next start. */ } return; }
    callId = opened.call.id;
    try {
      const requested = window.EzerbotMicrophone?.current() ?? "";
      const voice = botVoice();
      // The secret goes to the provider once. It is never shown, stored or kept past this call.
      const result = await bridge("start", {secret: opened.secret, microphone: requested, voice: voice.voice, speed: voice.speed, language: voice.language});
      opened = null;
      if (rev !== epoch) return;
      if (result.microphone_fallback) window.EzerbotMicrophone?.fellBack?.(requested);
      if (errors[result.status]) { await fail(result.status); return; }
      phase = "listening"; render(); schedule(rev);
    } catch (error) { if (rev === epoch) await fail("failed", error.message); }
  }
  function schedule(rev) { clearTimeout(timer); if (active() && rev === epoch) timer = setTimeout(() => tick(rev), 100); }
  async function tick(rev) {
    if (!active() || rev !== epoch) return;
    if (ticking) { schedule(rev); return; }
    ticking = true;
    try {
      const result = await bridge("status");
      if (rev !== epoch) return;
      if (errors[result.status] || result.status === "idle") { await fail(result.status); return; }
      muted = Boolean(result.muted);
      lines = result.lines || lines; partial = muted ? "" : result.transcript || ""; speaking = Boolean(result.speaking);
      // A card waiting for a click holds the voice. Speaking is not a decision on it.
      const needsAction = window.GrokbotChat.voiceNeedsAction(ctx.conversation?.messages || []);
      if (needsAction !== paused) {
        paused = needsAction;
        await bridge(paused ? "pause" : "resume");
        if (rev !== epoch) return;
      }
      try { await persist(callId, lines); } catch { /* Written at the next tick, or when leaving. */ }
      if (rev !== epoch) return;
      phase = needsAction ? "waiting" : speaking ? "speaking" : "listening";
      render();
    } catch (error) { if (rev === epoch) await fail("failed", error.message); }
    finally { ticking = false; schedule(rev); }
  }
  // A reload during a call stops the session but leaves the call open on the Mac: this view closes it,
  // which also puts the lines already written into the thread.
  async function reconcile(next) {
    const open = next.state?.voice_call?.active;
    if (!open || callId || finished.has(open.id)) return;
    finished.add(open.id);
    try { await panels.api("/api/voice/call/end", {id: open.id}); await panels.refresh(); }
    catch { /* Not retried on every state read: the Mac closes it at its next start. */ }
  }
  function observe(next) {
    reconcile(next);
    if (scopeKey && scopeKey !== contextKey(next)) { ctx = next; quit(); return; }
    ctx = next;
    render();
  }
  function cutVoice() {
    if (!active()) return;
    speaking = false; render();
    bridge("interrupt").catch(error => panels.notify(error.message));
  }
  async function toggleMute() {
    if (!active() || ["authorizing", "ending"].includes(phase)) return;
    const rev = epoch, receipt = await bridge(muted ? "unmute" : "mute");
    if (rev !== epoch) return;
    if (errors[receipt.status]) { await fail(receipt.status); return; }
    muted = Boolean(receipt.muted); if (muted) partial = "";
    // Unmuting listens again to the chosen microphone, which may have gone while muted.
    if (!muted && receipt.microphone_fallback) window.EzerbotMicrophone?.fellBack?.(window.EzerbotMicrophone.current());
    render();
  }
  const muteToggled = () => toggleMute().catch(error => panels.notify(error.message));
  panels.register({
    id: "voice-call",
    composer: {
      mount(container, context) {
        ctx = context;
        button = el("button", "icon-button voice-call-toggle"); button.type = "button"; button.onclick = () => start().catch(error => panels.notify(error.message));
        button.setAttribute("aria-label", "Appel vocal");
        const icon = el("span", "voice-call-icon"); icon.setAttribute("aria-hidden", "true");
        icon.innerHTML = '<svg class="icon" viewBox="0 0 24 24"><path d="M22 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 2.1 4.2 2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7c.1 1 .4 1.9.7 2.8a2 2 0 0 1-.5 2.1L8.1 9.9a16 16 0 0 0 6 6l1.3-1.3a2 2 0 0 1 2.1-.4c.9.3 1.8.6 2.8.7a2 2 0 0 1 1.7 2z"/></svg>';
        button.append(icon, el("span", "voice-call-label", "Appel vocal"));
        // A conversation action in the header: the composer row has no room for a fourth voice control.
        const actions = document.querySelector(".topbar-actions");
        actions.insertBefore(button, document.getElementById("conversation-menu-button"));
        // Under 900 px the header has no width to spare for the title: the same action moves to the conversation menu.
        menuItem = el("button", "voice-call-menu-item"); menuItem.type = "button";
        menuItem.onclick = () => {
          document.getElementById("conversation-menu").hidden = true;
          document.getElementById("conversation-menu-button").setAttribute("aria-expanded", "false");
          start().catch(error => panels.notify(error.message));
        };
        const menuIcon = el("span", "voice-call-icon"); menuIcon.setAttribute("aria-hidden", "true"); menuIcon.innerHTML = icon.innerHTML;
        menuItem.append(menuIcon, el("span", "", "Appel vocal"));
        document.getElementById("rename-button").before(menuItem);
        bar = el("section", "voice-call-bar"); bar.setAttribute("aria-label", "Appel vocal");
        const row = el("div", "voice-call-row");
        label = el("strong", "voice-call-state"); label.setAttribute("role", "status");
        transcriptToggle = el("button", "attachment-retry voice-call-transcript-toggle", "Afficher le transcript"); transcriptToggle.type = "button";
        transcriptToggle.onclick = () => { shown = !shown; render(); };
        mute = el("button", "icon-button voice-mute-toggle"); mute.type = "button"; mute.setAttribute("aria-keyshortcuts", "Meta+Shift+M");
        mute.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="2" width="6" height="13" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/><path class="voice-mute-slash" d="M3 3l18 18"/></svg>';
        mute.onclick = muteToggled;
        interrupt = el("button", "attachment-retry", "Couper la voix"); interrupt.type = "button"; interrupt.onclick = cutVoice;
        leave = el("button", "attachment-retry", "Quitter"); leave.type = "button";
        leave.onclick = () => active() ? quit().catch(error => panels.notify(error.message)) : (phase = "idle", detail = "", render());
        row.append(label, transcriptToggle, mute, interrupt, leave);
        notice = el("p", "voice-call-notice", disclosure);
        transcript = el("div", "voice-call-transcript"); transcript.setAttribute("role", "log"); transcript.setAttribute("aria-label", "Transcript de l’appel");
        bar.append(row, notice, transcript); container.append(bar);
        document.addEventListener("keydown", event => {
          if (event.key === "Escape" && active()) { event.preventDefault(); event.stopImmediatePropagation(); cutVoice(); }
          if (event.metaKey && event.shiftKey && !event.ctrlKey && !event.altKey && !event.repeat && event.key?.toLowerCase() === "m" && active()) {
            event.preventDefault(); event.stopImmediatePropagation(); muteToggled();
          }
        }, true);
        // Closing the window ends the call: the session stops, and the lines already written stay.
        const stopSession = () => { if (callId && callId !== "pending") bridge("cancel").catch(() => {}); };
        window.addEventListener("beforeunload", stopSession);
        window.addEventListener("pagehide", stopSession);
        for (const name of ["grokbot:voice-mode-change", "grokbot:voice-change", "grokbot:meeting-change"]) window.addEventListener(name, render);
        render();
      },
      discard() { if (active()) quit(); },
    },
    onState(state, context) { observe(context); },
    onConnection(connected) { if (!connected && active()) fail("failed", "La connexion au moteur local est interrompue. L’appel vocal est terminé."); },
    wantsPolling: active,
  });
})();
