"use strict";
// Loaded before the shell: only the private listener is an iPhone client.
(() => {
  const remote = location.hostname.endsWith(".ts.net") || (location.hostname === "127.0.0.1" && location.port === "47863");
  const node = (tag, text, className) => { const e = document.createElement(tag); if (text) e.textContent = text; if (className) e.className = className; return e; };
  let overlay, error, receipt, timer, working = false;
  const rawFetch = window.fetch.bind(window);
  function pairing(message) {
    if (!remote) return;
    if (overlay) { if (message) error.textContent = message; return; }
    document.querySelectorAll("dialog[open]").forEach(dialog => dialog.close());
    for (const child of document.body.children) child.inert = true;
    overlay = node("section", null, "iphone-pairing"); overlay.setAttribute("aria-labelledby", "pairing-title");
    overlay.setAttribute("role", "dialog"); overlay.setAttribute("aria-modal", "true");
    const box = node("form", null, "iphone-pairing-box");
    const title = node("h1", "Associer cet iPhone"); title.id = "pairing-title";
    box.append(title, node("p", "Sur le Mac : Paramètres › iPhone › Créer un code. Saisissez-le ici, puis confirmez le nom de cet appareil sur le Mac."));
    const nameLabel = node("label", "Nom de l’appareil"); const name = node("input"); name.className = "text-input"; name.value = "Mon iPhone"; name.maxLength = 60; name.required = true; name.autocomplete = "off"; nameLabel.append(name);
    const codeLabel = node("label", "Code affiché sur le Mac"); const code = node("input"); code.className = "text-input"; code.required = true; code.maxLength = 14; code.autocapitalize = "characters"; code.autocomplete = "one-time-code"; code.spellcheck = false; codeLabel.append(code);
    const send = node("button", "Demander l’association", "primary-button"); send.type = "submit";
    error = node("p"); error.role = "status";
    const retry = node("button", "Recommencer", "secondary-button"); retry.type = "button"; retry.hidden = true;
    retry.onclick = () => { clearTimeout(timer); receipt = null; name.disabled = code.disabled = send.disabled = false; retry.hidden = true; error.textContent = "Créez un nouveau code sur le Mac."; };
    box.append(nameLabel, codeLabel, send, retry, error, node("p", "Dans Safari : Partager › Sur l’écran d’accueil › Ajouter. Ouvrez ensuite l’app depuis son icône pour l’associer. Connexion privée par Tailscale ; le Mac doit rester allumé et Ezerbot ouvert."));
    box.onsubmit = async e => {
      e.preventDefault(); if (working) return; working = true; send.disabled = true;
      try { const response = await request("/api/iphone/claim", { name: name.value, code: code.value }); receipt = response.receipt; name.disabled = code.disabled = true; retry.hidden = false; error.textContent = `En attente de confirmation sur le Mac pour « ${name.value} ». La demande expire après deux minutes.`; poll(); }
      catch (e) { error.textContent = e.message; send.disabled = false; }
      finally { working = false; }
    };
    overlay.append(box); document.body.append(overlay); name.focus();
    if (message) error.textContent = message;
  }
  async function request(path, body) {
    const response = await rawFetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" }, body: JSON.stringify(body), signal: AbortSignal.timeout(10000) });
    const value = await response.json(); if (!response.ok) throw new Error(value.error || "Connexion interrompue."); return value;
  }
  async function poll() {
    if (!receipt) return;
    try { const value = await request("/api/iphone/exchange", { receipt }); if (value.paired) { receipt = null; location.reload(); return; } timer = setTimeout(poll, 1500); }
    catch (e) { error.textContent = e.message; receipt = null; }
  }
  if (remote) {
    document.documentElement.dataset.iphone = "true";
    // iOS does not consistently resize 100dvh when its keyboard/dictation opens.
    // Keep the composer inside the actual visual viewport, not under the keyboard.
    const viewport = window.visualViewport;
    const fitKeyboard = () => document.documentElement.style.setProperty("--phone-height", `${viewport?.height || innerHeight}px`);
    viewport?.addEventListener("resize", fitKeyboard); window.addEventListener("orientationchange", fitKeyboard); fitKeyboard();
    window.fetch = async (...args) => {
      const response = await rawFetch(...args);
      const path = typeof args[0] === "string" ? args[0] : args[0]?.url || "";
      if (response.status === 401 && path.startsWith("/api/")) pairing();
      return response;
    };
    if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
    window.addEventListener("online", () => window.GrokbotPanels?.refresh().catch(() => {}));
    navigator.serviceWorker?.addEventListener("message", e => { if (e.data?.notification) window.EzerbotNotifications?.activate(e.data.notification); });
  }
  window.EzerbotIPhone = { remote, pairing };
})();
