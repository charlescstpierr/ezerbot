import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./floating.css";

import { floatingConversations, floatingReply, floatingPendingRequests, floatingHasMore, resizeFloatingInput } from "./floating-view.js";
import { floatingFailureState } from "./connection-view.js";

async function api(path, body) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 15000);
  try {
    const response = await fetch(path, { ...(body === undefined ? { cache: "no-store" } : {
      method: "POST", headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" }, body: JSON.stringify(body),
    }), signal: controller.signal });
    const value = await response.json();
    if (!response.ok) throw Object.assign(new Error(value.error || "Le moteur local ne répond pas."), { status: response.status });
    return value;
  } finally { clearTimeout(timeout); }
}
const native = (body) => window.webkit?.messageHandlers?.ezerbotFloating?.postMessage(body);

function Floating() {
  const [state, setState] = useState(null);
  const [target, setTarget] = useState("");
  const [bot, setBot] = useState("grokbot");
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const latest = useRef(null);
  const replyContent = useRef(null);
  const inNativePanel = Boolean(window.webkit?.messageHandlers?.ezerbotFloating);
  const pending = useRef(null);
  const draftValue = useRef("");
  draftValue.current = draft;
  const input = useRef(null);
  const sending = useRef(false);
  const mounted = useRef(true);
  const epoch = useRef(0);
  const conversations = floatingConversations(state);
  const conversation = conversations.find((c) => c.id === target);
  const currentBot = state?.bots.find((b) => b.id === (conversation?.bot_id || bot));
  const reply = floatingReply(conversation);
  const requests = floatingPendingRequests(state, conversation);
  const displayReply = requests.length ? floatingReply(conversation, true) : reply;
  const question = conversation?.messages.filter((m) => m.role === "user").at(-1)?.body;
  const failed = reply?.state === "failed";
  const failure = floatingFailureState(reply, { providerConnected: state?.provider?.connected === true });
  const live = conversation?.messages.some((m) => ["streaming", "generating", "queued", "dispatching", "unknown"].includes(m.state));
  const unavailable = Boolean(target && !conversation);

  async function refresh() {
    const value = await api("/api/state");
    if (mounted.current) {
      setState(value);
      setError((previous) => previous.startsWith("Connexion interrompue.") || previous.startsWith("Le moteur local est indisponible.") ? "" : previous);
    }
    return value;
  }
  async function open() {
    if (sending.current || pending.current || draftValue.current.trim()) { input.current?.focus(); return; }
    const generation = ++epoch.current;
    try {
      const [value, selection] = await Promise.all([refresh(), api("/api/floating")]);
      if (!mounted.current || generation !== epoch.current) return;
      setTarget(selection.conversation_id || ""); setBot(selection.bot_id); setError("");
      setState(value); input.current?.focus();
    } catch { setError("Le moteur local est indisponible. Votre brouillon reste ici."); }
  }
  useLayoutEffect(() => { resizeFloatingInput(input.current); }, [draft]);
  useLayoutEffect(() => { if (latest.current) latest.current.scrollTop = 0; }, [target, reply?.id]);
  useEffect(() => {
    const measure = () => setOverflowing(floatingHasMore(latest.current));
    const observer = new ResizeObserver(measure);
    observer.observe(latest.current); observer.observe(replyContent.current);
    const resize = () => resizeFloatingInput(input.current);
    window.addEventListener("resize", resize);
    measure();
    return () => { observer.disconnect(); window.removeEventListener("resize", resize); };
  }, []);
  useEffect(() => {
    mounted.current = true;
    window.EzerbotFloating = { open };
    open();
    let timer;
    const poll = async () => {
      try { await refresh(); } catch { if (mounted.current) setError("Connexion interrompue. Votre texte est conservé."); }
      if (mounted.current) timer = setTimeout(poll, 1000);
    };
    timer = setTimeout(poll, 1000);
    const close = (event) => {
      if (event.key === "Escape") { event.preventDefault(); native({ command: "close" }); }
    };
    document.addEventListener("keydown", close);
    return () => { mounted.current = false; clearTimeout(timer); delete window.EzerbotFloating; document.removeEventListener("keydown", close); };
  }, []);
  useEffect(() => {
    const choice = state?.preferences?.theme || "dark";
    const system = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const theme = choice === "system" ? system.matches ? "dark" : "light" : choice;
      document.documentElement.dataset.theme = theme;
      Promise.resolve(native({ command: "appearance", theme })).catch(() => {});
    };
    apply(); system.addEventListener("change", apply);
    document.documentElement.dataset.accent = state?.preferences?.accent || "citron";
    return () => system.removeEventListener("change", apply);
  }, [state?.preferences?.theme, state?.preferences?.accent]);
  useEffect(() => {
    const mount = document.getElementById("floating-mascot");
    if (!mount || !currentBot) return;
    mount.replaceChildren(window.GrokbotMascot?.create?.(false, currentBot.shape) || document.createTextNode(""));
    mount.dataset.botColor = currentBot.color;
    mount.style.setProperty("--mascot-eye", "var(--bot-color)");
  }, [currentBot?.shape, currentBot?.color]);

  async function full(id) {
    if (!id) return;
    try {
      const result = native({ command: "openConversation", conversation_id: id });
      if (result) await result;
      else { localStorage.setItem("grokbot.selected", id); location.href = "/"; }
    } catch (failure) {
      const reason = typeof failure?.message === "string" ? failure.message.trim() : "";
      setError(reason || "Impossible d’ouvrir ce fil. Réessayez depuis Ezerbot.");
    }
  }
  async function send(event) {
    event.preventDefault();
    if (sending.current || unavailable || (!pending.current && (!draft.trim() || live))) return;
    sending.current = true; setBusy(true); setError("");
    try {
      let id = target;
      if (!id) {
        const created = await api("/api/conversations", { bot_id: bot });
        id = created.conversation.id; setTarget(id);
      }
      // Keep the nonce and exact body after transport failure. A retry reconciles
      // this send; it never creates a duplicate or changes its chosen recipient.
      if (!pending.current) pending.current = { conversation_id: id, message_id: crypto.randomUUID(), body: draft.trim() };
      await api("/api/floating/send", pending.current);
      pending.current = null; setUncertain(false); setDraft("");
      try { await refresh(); }
      catch { setError("Message enregistré. La connexion est interrompue ; ouvrez le fil pour le retrouver."); }
    } catch (failure) {
      if ([400, 404, 413, 422].includes(failure.status)) pending.current = null;
      setUncertain(Boolean(pending.current));
      const reason = failure.status ? failure.message : "La connexion au moteur local est interrompue.";
      setError(`${reason} Votre texte est conservé.${pending.current ? " Vérifiez l’envoi avec le même identifiant, ou ouvrez le fil." : ""}`);
    } finally { sending.current = false; setBusy(false); input.current?.focus(); }
  }
  return <section className="floating-panel">
    <header className="floating-heading"><span id="floating-mascot" aria-hidden="true" /><div><small>VOTRE BOT, À PORTÉE DE MAIN</small><h1>{conversation?.kind === "group" ? "Groupe" : currentBot?.name || "Ezerbot"}</h1></div>{!inNativePanel && <button className="icon-button floating-close" title="Fermer · Échap" aria-label="Fermer le panneau" onClick={() => window.close()}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true"><path d="m6 6 12 12M6 18 18 6" /></svg></button>}</header>
    <label className="form-label" htmlFor="floating-target">Envoyer dans</label>
    <select className="text-input" id="floating-target" value={target} disabled={busy || uncertain} onChange={(e) => { setTarget(e.target.value); setError(""); }}>
      <option value="">Nouvelle conversation · {state?.bots.find((b) => b.id === bot)?.name || "Ezerbot"}</option>
      {unavailable && <option value={target}>Conversation indisponible</option>}
      {conversations.map((c) => <option key={c.id} value={c.id}>{c.title} · {c.kind === "group" ? "Groupe" : state.bots.find((b) => b.id === c.bot_id)?.name}</option>)}
    </select>
    <div className={`floating-reading${overflowing ? " is-overflowing" : ""}`}>
      <div ref={latest} className="floating-latest" role="region" aria-label="Dernière réponse" tabIndex={0} onScroll={() => setOverflowing(floatingHasMore(latest.current))}>
        <div ref={replyContent}>
          {question && <div className="floating-question-preview"><p className="floating-question" title={question}>{question}</p></div>}
          <p className="floating-answer">{displayReply?.body || (failed || requests.length ? "" : live ? "Le bot prépare sa réponse…" : unavailable ? "Ce fil a été supprimé ou n’est plus accessible. Choisissez une autre conversation." : "Une question, une idée ? Votre prochain échange apparaîtra ici.")}</p>
          {live && reply?.body && <small>Réponse en cours…</small>}
        </div>
      </div>
      {overflowing && <span className="floating-overflow-hint">↓ Faites défiler pour lire la suite</span>}
    </div>
    {requests.map((request) => <div className="floating-notice" key={request.id} role="status"><p>Votre décision est attendue dans Ezerbot.</p><button type="button" className="secondary-button" onClick={() => full(request.route.target_conversation_id)}>Voir la demande</button></div>)}
    {failure && <div className={`floating-notice${failure.alert ? " floating-failure" : ""}`} role={failure.alert ? "alert" : "status"}><div><strong>{failure.title}</strong><p className={failure.detailClass || undefined}>{failure.detail}</p></div><button type="button" className="secondary-button" onClick={() => full(target)}>Ouvrir le fil</button></div>}
    {error && <p className="floating-error" role="alert">{error}</p>}
    <form className="floating-composer" onSubmit={send}>
      <textarea ref={input} id="floating-message" name="message" aria-label="Message au bot" placeholder="Qu’avez-vous en tête ?" value={draft} maxLength={16384} readOnly={busy || uncertain} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); e.currentTarget.form.requestSubmit(); } }} />
      <button className="primary-button" type="submit" disabled={busy || !state || unavailable || (!uncertain && (!draft.trim() || live))}>{busy ? "Envoi…" : uncertain ? "Vérifier l’envoi" : "Envoyer"}</button>
    </form>
    <footer><span>Entrée pour envoyer · Échap pour fermer</span><button className="secondary-button" disabled={!target || busy} onClick={() => full(target)}>Ouvrir dans Ezerbot</button></footer>
  </section>;
}

export function mountFloating(element) { createRoot(element).render(<Floating />); }
