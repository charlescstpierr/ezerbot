const { createHash } = require("node:crypto");
const { execFileSync } = require("node:child_process");
const { readFileSync, writeFileSync } = require("node:fs");
const { join, resolve } = require("node:path");
const { launchFixture } = require("../app/ui/wrapper-chrome-fixture.cjs");

const VIEWPORT = Object.freeze({ width: 1440, height: 900, device_scale_factor: 1 });
const VIEWS = Object.freeze([
  { id: "home-dark", label: "Accueil", action: "GrokbotPanels.leaveConversation()", ready: "document.getElementById('conversation-title')?.textContent === 'Nouvelle conversation' && !document.getElementById('welcome')?.hidden && document.querySelector('.gb-editor')" },
  { id: "conversation-dark", label: "Conversation factice", action: "GrokbotPanels.openConversation(window.captureConversationId)", ready: "document.getElementById('conversation-title')?.textContent === 'Préparer la semaine' && document.getElementById('chat-scroll')?.textContent.includes('Voici votre document.')" },
  { id: "settings-dark", label: "Réglages", action: "document.getElementById('settings-button').click()", ready: "document.getElementById('settings-dialog')?.open && document.querySelector('#settings-dialog .setting-section')" },
  { id: "routines-dark", label: "Routines", action: "document.querySelector('[data-sidebar-destination=routines]').click()", ready: "document.querySelector('.routines-dialog')?.open && document.querySelector('.routines-content')?.textContent.includes('Aucune routine pour ce bot.')" },
  { id: "notifications-dark", label: "Notifications", action: "document.querySelector('[data-sidebar-destination=notifications]').click()", ready: "document.querySelector('.notifications-dialog')?.open && document.querySelector('.notifications-dialog .notification-item')?.textContent.includes('Préparer la semaine')" },
]);

async function bounded(work, description, signal, timeoutMs = 20000) {
  if (signal?.aborted) throw new Error("Capture interrupted.");
  let timer;
  let abort;
  try {
    return await Promise.race([
      work,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${description} timed out.`)), timeoutMs);
        abort = () => reject(new Error("Capture interrupted."));
        signal?.addEventListener("abort", abort, { once: true });
      }),
    ]);
  } finally {
    clearTimeout(timer);
    if (abort) signal?.removeEventListener("abort", abort);
  }
}

async function waitFor(read, description, signal, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await bounded(Promise.resolve().then(read), description, signal, Math.max(1, deadline - Date.now()))) return;
    await new Promise(resolveWait => setTimeout(resolveWait, Math.min(50, Math.max(1, deadline - Date.now()))));
  }
  throw new Error(`${description} timed out.`);
}

function pngInfo(bytes) {
  if (bytes.length < 33 || !bytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")) || bytes.toString("ascii", 12, 16) !== "IHDR") throw new Error("Screenshot is not a PNG.");
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width !== VIEWPORT.width || height !== VIEWPORT.height) throw new Error(`Screenshot dimensions ${width}x${height} differ from the viewport.`);
  return { width, height, sha256: createHash("sha256").update(bytes).digest("hex") };
}

async function capture(sha, binary, output, signal) {
  const source = resolve(__dirname, "..");
  if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error("Expected a full lowercase commit SHA.");
  const actual = execFileSync("git", ["-C", source, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (actual !== sha) throw new Error("Capture driver does not belong to the requested commit.");
  const cleanup = [];
  let closing;
  const close = () => closing ||= (async () => { for (const dispose of cleanup.reverse()) await dispose(); })();
  const cancelled = new AbortController();
  const relay = () => cancelled.abort();
  signal?.addEventListener("abort", relay, { once: true });
  let stage = "fixture launch";
  try {
    const fixture = await bounded(launchFixture({ after: callback => cleanup.push(callback) }, {
      documents: [], binary, headless: false, navigate: false, viewport: { width: VIEWPORT.width, height: VIEWPORT.height },
    }).catch(() => { throw new Error("Isolated wrapper or Chrome failed to start."); }), stage, cancelled.signal, 45000);
    const { cdp } = fixture;
    const evaluate = expression => bounded(fixture.evaluate(expression).catch(() => { throw new Error("Browser operation failed."); }), stage, cancelled.signal);
    const send = (method, params) => bounded(cdp.send(method, params), stage, cancelled.signal);
    let browserFailure;
    cdp.on("Runtime.exceptionThrown", () => { browserFailure ||= "Uncaught browser exception."; });
    const requests = new Map();
    cdp.on("Network.requestWillBeSent", event => requests.set(event.requestId, event));
    cdp.on("Network.loadingFailed", event => {
      const request = requests.get(event.requestId);
      if (["Document", "Script", "Stylesheet", "Font", "Image"].includes(request?.type) && request.request.url.startsWith(fixture.url)) browserFailure ||= "A required application asset failed to load.";
    });
    cdp.on("Network.responseReceived", event => {
      if (event.response.status >= 400 && ["Document", "Script", "Stylesheet", "Font", "Image"].includes(event.type) && event.response.url.startsWith(fixture.url)) browserFailure ||= "A required application asset returned an error.";
    });
    await send("Network.enable");
    await send("Emulation.setTimezoneOverride", { timezoneId: "UTC" });
    await send("Emulation.setLocaleOverride", { locale: "fr-CA" });
    await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }, { name: "prefers-reduced-motion", value: "reduce" }] });
    await send("Page.navigate", { url: fixture.url });
    const check = async expression => {
      if (browserFailure) throw new Error(browserFailure);
      return evaluate(`Boolean(${expression})`);
    };
    stage = "application readiness";
    await waitFor(() => check("window.GrokbotPanels?.connected && document.querySelector('.gb-editor') && document.querySelector('.memory-entry')"), stage, cancelled.signal);
    stage = "runtime provenance";
    const installed = await bounded(evaluate("fetch('/api/updates').then(r => { if (!r.ok) throw Error('updates request failed'); return r.json(); }).then(state => state.installed_commit)"), stage, cancelled.signal);
    if (installed !== sha) throw new Error("Running wrapper commit differs from the requested commit.");
    for (const [path, file] of [["/", "index.html"], ["/styles.css", "styles.css"]]) {
      const served = await bounded(evaluate(`fetch(${JSON.stringify(path)}).then(r => { if (!r.ok) throw Error('asset request failed'); return r.text(); })`), stage, cancelled.signal);
      if (served !== readFileSync(join(source, "app/ui", file), "utf8")) throw new Error("Running wrapper assets differ from the target worktree.");
    }
    stage = "synthetic conversation";
    await bounded(evaluate(`(async () => {
      const post = async (path, body) => { const r = await fetch(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)}); if (!r.ok) throw Error('fixture request failed'); return r.json(); };
      const {conversation} = await post('/api/conversations', {title:'Préparer la semaine'});
      window.captureConversationId = conversation.id;
      await post('/api/send', {conversation_id:conversation.id, message_id:'linux-capture-message', body:'Bonjour, prépare une courte liste pour organiser ma semaine.'});
    })()`), stage, cancelled.signal);
    await waitFor(() => evaluate(`(async () => {
      await GrokbotPanels.refresh();
      const conversation = GrokbotPanels.state.conversations.find(item => item.id === window.captureConversationId);
      return conversation?.messages?.at(-1)?.role === 'assistant'
        && conversation.messages.at(-1).body?.includes('Voici votre document.')
        && !conversation.messages.some(message => GrokbotChat.isPendingReply(message, false));
    })()`), stage, cancelled.signal);
    await evaluate(`document.documentElement.dataset.theme='dark';
      const style=document.createElement('style'); style.textContent='*,*::before,*::after { animation:none!important; transition:none!important; caret-color:transparent!important; }'; document.head.append(style);`);
    const views = [];
    for (const view of VIEWS) {
      stage = `${view.id} readiness`;
      await bounded(evaluate(`document.querySelectorAll('dialog[open]').forEach(dialog => dialog.close()); ${view.action}`), stage, cancelled.signal);
      await waitFor(() => check(view.ready), stage, cancelled.signal);
      stage = `${view.id} fonts, images and paint`;
      await bounded(evaluate(`(async () => {
        await document.fonts.ready;
        await Promise.all([...document.images].filter(image => image.getClientRects().length).map(async image => { await image.decode(); if (!image.naturalWidth) throw Error('Image did not decode'); }));
        document.activeElement?.blur();
        window.scrollTo(0,0); document.querySelectorAll('dialog[open],#chat-scroll,#conversation-list').forEach(element => element.scrollTop=0);
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      })()`), stage, cancelled.signal);
      await waitFor(() => check(view.ready), `${view.id} after paint`, cancelled.signal);
      if (browserFailure) throw new Error(browserFailure);
      stage = `${view.id} screenshot`;
      const { data } = await send("Page.captureScreenshot", { format: "png", fromSurface: true, captureBeyondViewport: false });
      const file = `${view.id}.png`;
      writeFileSync(join(output, file), Buffer.from(data, "base64"), { flag: "wx" });
      views.push({ id: view.id, label: view.label, file, ...pngInfo(readFileSync(join(output, file))) });
    }
    stage = "manifest";
    const version = await send("Browser.getVersion");
    const manifest = { schema_version: 1, sha, captured_at: new Date().toISOString(), platform: "linux", renderer: "chromium-web-fallback", chrome_version: version.product, viewport: VIEWPORT, views };
    writeFileSync(join(output, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, { flag: "wx" });
    return manifest;
  } catch (error) {
    throw new Error(`${stage} failed. ${error.message}`, { cause: error });
  } finally {
    cancelled.abort();
    signal?.removeEventListener("abort", relay);
    await close();
  }
}

if (require.main === module) {
  const controller = new AbortController();
  for (const name of ["SIGINT", "SIGTERM"]) process.once(name, () => controller.abort());
  if (process.platform !== "linux" || process.argv.length !== 5) {
    console.error("Usage on Linux: node --experimental-websocket tools/linux-captures.cjs <sha40> <binary> <staging-directory>");
    process.exitCode = 1;
  } else capture(...process.argv.slice(2), controller.signal).catch(error => {
    console.error(`linux-captures: ${error.message}`);
    process.exitCode = 1;
  });
}

module.exports = { VIEWS, VIEWPORT, bounded, waitFor, pngInfo, capture };
