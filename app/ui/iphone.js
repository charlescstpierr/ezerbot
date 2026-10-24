"use strict";
// Loaded before the shell: only the private listener is an iPhone client.
(() => {
  const remote = location.hostname.endsWith(".ts.net") || (location.hostname === "127.0.0.1" && location.port === "47863");
  const node = (tag, text, className) => { const e = document.createElement(tag); if (text) e.textContent = text; if (className) e.className = className; return e; };
  let overlay, error, receipt, timer, working = false, authenticated = false, scanned = null;
  const rawFetch = window.fetch.bind(window);
  // The Mac's QR carries the code in the fragment, which no request sends.
  // Read it once and drop it from the address before anything else runs.
  if (remote) {
    const match = /^#jumelage=([0-9A-F]{10})$/i.exec(location.hash);
    if (location.hash.startsWith("#jumelage=")) history.replaceState(null, "", location.pathname + location.search);
    if (match) scanned = match[1].toUpperCase();
  }
  function status(text, failed = false) {
    error.textContent = text; error.className = failed ? "iphone-warning" : "";
    error.role = failed ? "alert" : "status";
  }
  function pairing(message) {
    if (!remote || overlay) return;
    document.querySelectorAll("dialog[open]").forEach(dialog => dialog.close());
    for (const child of document.body.children) child.inert = true;
    overlay = node("section", null, "iphone-pairing"); overlay.setAttribute("aria-labelledby", "pairing-title");
    overlay.setAttribute("role", "dialog"); overlay.setAttribute("aria-modal", "true"); overlay.tabIndex = -1;
    const box = node("form", null, "iphone-pairing-box");
    const brand = node("div", null, "brand iphone-pairing-brand");
    const mascot = node("span", null, "iphone-brand-mascot"); mascot.setAttribute("aria-hidden", "true");
    if (window.GrokbotMascot) mascot.append(window.GrokbotMascot.create(false));
    else { const icon = node("img"); icon.src = "/icon-192.png"; icon.alt = ""; mascot.append(icon); }
    const word = node("span", "ezerbot"); word.append(node("span", ".", "brand-period")); brand.append(mascot, word);
    const title = node("h1", "Associer cet iPhone"); title.id = "pairing-title";
    box.append(brand, title);
    if (!matchMedia("(display-mode: standalone)").matches && !navigator.standalone) {
      const install = node("div", null, "iphone-install-note");
      install.append(node("strong", "Installez d’abord Ezerbot"), node("p", scanned
        ? "Dans Safari : Partager › Sur l’écran d’accueil › Ajouter, puis ouvrez l’app depuis son icône. iOS ne lui transmet pas ce QR : tapez-y une seule fois le code affiché sur le Mac."
        : "Dans Safari : Partager › Sur l’écran d’accueil › Ajouter. Ouvrez ensuite l’app depuis son icône pour l’associer.")); box.append(install);
    }
    box.append(node("p", scanned ? "Adresse et code remplis depuis le QR du Mac. Vérifiez le nom, demandez l’association, puis confirmez-la sur le Mac."
      : "Sur le Mac : Paramètres › iPhone › Créer un QR de jumelage. Scannez-le avec l’appareil photo, ou saisissez son code ici, puis confirmez le nom de cet appareil sur le Mac."));
    const nameLabel = node("label", "Nom de l’appareil"); const name = node("input"); name.className = "text-input";
    let savedName; try { savedName = localStorage.getItem("ezerbot.iphone.name"); } catch { /* private browsing */ }
    name.value = savedName || "Mon iPhone"; name.maxLength = 60; name.required = true; name.autocomplete = "off"; nameLabel.append(name);
    const codeLabel = node("label", "Code affiché sur le Mac"); const code = node("input"); code.className = "text-input"; code.required = true; code.maxLength = 14; code.autocapitalize = "characters"; code.autocomplete = "one-time-code"; code.spellcheck = false; codeLabel.append(code);
    if (scanned) { code.value = scanned; scanned = null; }
    const send = node("button", "Demander l’association", "primary-button"); send.type = "submit";
    error = node("p"); error.role = "status";
    const retry = node("button", "Recommencer", "secondary-button"); retry.type = "button"; retry.hidden = true;
    retry.onclick = () => { clearTimeout(timer); receipt = null; name.disabled = code.disabled = send.disabled = false; retry.hidden = true; status("Créez un nouveau code sur le Mac."); };
    box.append(nameLabel, codeLabel, send, retry, error, node("p", "Connexion privée par Tailscale ; le Mac doit rester allumé et Ezerbot ouvert."));
    box.onsubmit = async e => {
      e.preventDefault(); if (working) return; working = true; send.disabled = true; status("");
      try { localStorage.setItem("ezerbot.iphone.name", name.value.trim()); } catch { /* private browsing */ }
      try {
        const response = await request("/api/iphone/claim", { name: name.value, code: code.value });
        receipt = response.receipt; name.disabled = code.disabled = true; retry.hidden = false;
        status(`En attente de confirmation sur le Mac pour « ${name.value} ». La demande expire après deux minutes.`); poll();
      } catch (e) { status(e.message, true); send.disabled = false; }
      finally { working = false; }
    };
    overlay.append(box); document.body.append(overlay); overlay.focus({ preventScroll: true });
    if (message) status(message, true);
  }
  async function request(path, body) {
    let response;
    try { response = await rawFetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" }, body: JSON.stringify(body), signal: AbortSignal.timeout(10000) }); }
    catch { throw new Error("Le Mac est injoignable. Vérifiez Tailscale, puis réessayez."); }
    let value; try { value = await response.json(); } catch { throw new Error("Le Mac a renvoyé une réponse illisible. Réessayez."); }
    if (!response.ok) throw new Error(value.error || "Connexion interrompue."); return value;
  }
  async function poll() {
    if (!receipt) return;
    try { const value = await request("/api/iphone/exchange", { receipt }); if (value.paired) { receipt = null; location.reload(); return; } timer = setTimeout(poll, 1500); }
    catch (e) { status(e.message, true); receipt = null; }
  }
  if (remote) {
    document.documentElement.dataset.iphone = "true";
    // iOS does not consistently resize 100dvh when its keyboard/dictation opens.
    const viewport = window.visualViewport;
    const fitKeyboard = () => document.documentElement.style.setProperty("--phone-height", `${viewport?.height || innerHeight}px`);
    viewport?.addEventListener("resize", fitKeyboard); window.addEventListener("orientationchange", fitKeyboard); fitKeyboard();
    window.fetch = async (...args) => {
      const response = await rawFetch(...args);
      const path = typeof args[0] === "string" ? args[0] : args[0]?.url || "";
      if (path.startsWith("/api/")) {
        if (response.ok) authenticated = true;
        if (response.status === 401) pairing(authenticated ? "Cet iPhone a été révoqué sur le Mac ou sa session a expiré. Créez un nouveau code pour l’associer de nouveau." : null);
      }
      return response;
    };
    if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
    window.addEventListener("online", () => window.GrokbotPanels?.refresh().catch(() => {}));
    navigator.serviceWorker?.addEventListener("message", e => { if (e.data?.notification) window.EzerbotNotifications?.activate(e.data.notification, { reply: e.data.reply === true, push: true }); });
  }
  window.EzerbotIPhone = { remote, pairing };
})();
