const { createHash } = require("node:crypto");
const { execFileSync, spawn, spawnSync } = require("node:child_process");
const { once } = require("node:events");
const { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { dirname, join, resolve } = require("node:path");
const { createInterface } = require("node:readline");
const { launchFixture } = require("../app/ui/wrapper-chrome-fixture.cjs");

const VIEWPORT = Object.freeze({ width: 1440, height: 900, device_scale_factor: 1 });
const VIEWS = Object.freeze([
  { id: "home-dark", label: "Accueil", action: "GrokbotPanels.leaveConversation()", ready: "document.getElementById('conversation-title')?.textContent === 'Nouvelle conversation' && !document.getElementById('welcome')?.hidden && document.querySelector('.gb-editor')" },
  { id: "conversation-dark", label: "Conversation factice", action: "GrokbotPanels.openConversation(window.captureConversationId)", ready: "document.getElementById('conversation-title')?.textContent === 'Préparer la semaine' && document.getElementById('chat-scroll')?.textContent.includes('Voici votre document.')" },
  { id: "settings-dark", label: "Réglages", action: "document.getElementById('settings-button').click()", ready: "document.getElementById('settings-dialog')?.open && document.querySelector('#settings-dialog .setting-section')" },
  { id: "routines-dark", label: "Routines", action: "document.querySelector('[data-sidebar-destination=routines]').click()", ready: "document.querySelector('.routines-dialog')?.open && document.querySelector('.routines-content')?.textContent.includes('Aucune routine pour ce bot.')" },
  { id: "notifications-dark", label: "Notifications", action: "document.querySelector('[data-sidebar-destination=notifications]').click()", ready: "document.querySelector('.notifications-dialog')?.open && document.querySelector('.notifications-dialog .notification-item')?.textContent.includes('Préparer la semaine')" },
  { id: "conversation-tools-dark", label: "Conversation avec outils", action: "window.captureToolConversation()", ready: "window.captureToolRan?.() && document.getElementById('conversation-title')?.textContent === 'Ranger le dossier' && document.querySelector('.gb-thinking')?.textContent.includes('Réfléchit') && document.querySelector('.gb-conversation')?.textContent.includes('Je regarde ce qu’il y a dans le dossier.') && !document.querySelector('.gb-conversation [data-card-kind], .gb-conversation details, .gb-conversation-problem') && ![...document.querySelectorAll('.gb-conversation .gb-assistant')].some(element => /list_directory|\\/tmp/.test(element.textContent))" },
  { id: "screen-consent-dark", label: "Accord de partage d’écran", action: "window.captureScreenConsent()", ready: "document.querySelector('.screen-dialog')?.open && document.querySelector('.screen-consent')?.textContent.includes('n’effectue aucun geste')" },
  { id: "screen-overlay", label: "Superposition du partage d’écran", native: true },
  { id: "screen-overlay-unproven", label: "Superposition en pause, bandeau non prouvé", native: true, compositor: true },
  { id: "settings-microphone-dark", label: "Paramètres › Micro", before: "Object.defineProperty(window, 'EzerbotShell', { value: { platform: 'linux', unavailablePanels: [] }, configurable: true })", action: "document.getElementById('settings-button').click()", step: "document.querySelector('.voice-microphone-settings')?.scrollIntoView({ block: 'center' })",
    ready: "document.getElementById('settings-dialog')?.open && ['Microphone par défaut du système', 'Microphone USB (test)'].every(text => [...document.querySelectorAll('.voice-microphone-settings .voice-microphone-name')].some(name => name.textContent === text && name.getClientRects().length))",
    settled: "['Microphone par défaut du système', 'Microphone USB (test)'].every(text => { const box = [...document.querySelectorAll('.voice-microphone-settings .voice-microphone-name')].find(name => name.textContent === text)?.getBoundingClientRect(); return box && box.top >= 0 && box.bottom <= innerHeight; })" },
]);

// Views drawn by the real GTK shell under openbox, not by Chromium.
const NATIVE_VIEWS = Object.freeze([
  { id: "floating-panel-native", label: "Panneau flottant au-dessus d’une autre fenêtre", renderer: "webkitgtk-native-shell" },
]);
const PANEL_TITLE = "Panneau · Ezerbot";
const OTHER_TITLE = "Autre application";

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

function linuxShell(source) {
  const script = readFileSync(join(source, "app/linux/src/local-api.js"), "utf8");
  const panels = script.match(/unavailablePanels: Object\.freeze\(\[([^\]]*)\]/)[1];
  return `window.EzerbotShell = Object.freeze({ platform: "linux", unavailablePanels: Object.freeze([${panels}]) });`;
}

async function overlayCapture(binary, file, signal, cleanup, compositor) {
  if (compositor) {
    const manager = spawn("picom", ["--backend", "xrender"], { stdio: "ignore" });
    const stopped = once(manager, "exit").catch(() => {});
    cleanup.push(async () => { manager.kill("SIGKILL"); await stopped; });
    await new Promise(done => setTimeout(done, 1000));
  }
  const helper = spawn(join(dirname(binary), "ezerbot-linux"), ["--screen-helper", "60000"], {
    stdio: ["pipe", "pipe", "ignore"],
    env: { ...process.env, GDK_BACKEND: "x11", NO_AT_BRIDGE: "1", DBUS_SESSION_BUS_ADDRESS: "disabled:" },
  });
  const exited = once(helper, "exit").catch(() => {});
  cleanup.push(async () => { helper.kill("SIGKILL"); await exited; });
  const lines = createInterface({ input: helper.stdout })[Symbol.asyncIterator]();
  const { value: ready } = await bounded(lines.next(), "Screen helper", signal);
  if (ready !== '{"ready":true}') throw new Error("Screen helper refused the lease.");
  if (compositor) {
    const { value: paused } = await bounded(lines.next(), "Screen helper pause", signal);
    if (paused !== '{"event":"paused","reason":"banner"}') throw new Error("The compositor did not pause the lease.");
    await new Promise(done => setTimeout(done, 200));
  }
  execFileSync("import", ["-window", "root", file]);
  helper.stdin.end();
  await bounded(exited, "Screen helper exit", signal);
}

const stackingIds = text => [...text.matchAll(/0x[0-9a-f]+/gi)].map(match => match[0].toLowerCase());
const hasAbove = text => text.includes("_NET_WM_STATE_ABOVE");
const nativeEntry = (view, bytes) => ({ id: view.id, label: view.label, file: `${view.id}.png`, renderer: view.renderer, ...pngInfo(bytes) });

function tool(command, args, options = {}) {
  const result = spawnSync(command, args, { maxBuffer: 64 * 1024 * 1024, ...options });
  if (result.error) throw new Error(`${command} is required and failed to start.`, { cause: result.error });
  return result;
}

async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolveExit => child.once("exit", resolveExit));
  child.kill("SIGTERM");
  const timer = setTimeout(() => child.kill("SIGKILL"), 5000);
  await exited;
  clearTimeout(timer);
}

// The real debug shell under openbox: another window, then the panel opened by its shortcut.
async function captureNative(shellBinary, output, signal) {
  const [view] = NATIVE_VIEWS;
  const cleanup = [];
  const root = mkdtempSync(join(tmpdir(), "ezcapn."));
  cleanup.push(() => rmSync(root, { recursive: true, force: true }));
  try {
    for (const name of ["home", "config", "cache", "data"]) mkdirSync(join(root, name));
    const server = spawn("Xvfb", ["-displayfd", "1", "-screen", "0", `${VIEWPORT.width}x${VIEWPORT.height}x24`, "-nolisten", "tcp"], { stdio: ["ignore", "pipe", "ignore"] });
    cleanup.push(() => stop(server));
    const number = await bounded(new Promise((resolveNumber, reject) => {
      server.stdout.once("data", data => resolveNumber(String(data).trim()));
      server.once("error", () => reject(new Error("Xvfb failed to start.")));
    }), "native Xvfb", signal);
    const env = {
      PATH: process.env.PATH, DISPLAY: `:${number}`, HOME: join(root, "home"), XDG_CONFIG_HOME: join(root, "config"),
      XDG_CACHE_HOME: join(root, "cache"), XDG_DATA_HOME: join(root, "data"), LANG: "C.UTF-8", TZ: "UTC",
      GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", EZERBOT_FAKE_SIGNIN: "1",
    };
    const x = (command, args, options) => tool(command, args, { env, encoding: "utf8", ...options });
    const start = (command, args) => {
      const child = spawn(command, args, { env, stdio: "ignore" });
      cleanup.push(() => stop(child));
      return new Promise((resolveStart, reject) => {
        child.once("error", () => reject(new Error(`${command} is required and failed to start.`)));
        child.once("spawn", () => resolveStart(child));
      });
    };
    await start("openbox", []);
    await waitFor(() => x("xprop", ["-root", "_NET_SUPPORTING_WM_CHECK"]).stdout.includes("window id"), "native window manager", signal);
    const clients = () => stackingIds(x("xprop", ["-root", "_NET_CLIENT_LIST_STACKING"]).stdout).map(id => {
      const properties = x("xprop", ["-id", id, "_NET_WM_NAME", "WM_NAME", "_NET_WM_STATE"]).stdout;
      return { id, name: properties.match(/WM_NAME\(\w+\) = "(.*)"/)?.[1], above: hasAbove(properties) };
    });
    const named = title => clients().find(client => client.name === title);
    // Right after it appears, openbox drops the windows mapped: the second window, started
    // until it is managed, proves the manager ready before the shell maps its own.
    let other;
    for (let attempt = 0; !other; attempt++) {
      if (attempt === 10) throw new Error("openbox never managed a window.");
      if (signal?.aborted) throw new Error("Capture interrupted.");
      const candidate = await start("display", ["-title", OTHER_TITLE, "-resize", "560x350", "-geometry", "+300+120", join(output, "conversation-dark.png")]);
      try {
        await waitFor(() => named(OTHER_TITLE), "native second window", signal, 3000);
        other = candidate;
      } catch {
        await stop(candidate);
      }
    }
    await start(shellBinary, ["--data-dir", join(root, "state")]);
    await waitFor(() => named("Ezerbot"), "native shell window", signal, 60000);
    let panel;
    await waitFor(() => {
      if (!named(PANEL_TITLE)) x("xdotool", ["key", "super+alt+j"]);
      return panel = named(PANEL_TITLE);
    }, "native panel opened by its shortcut", signal, 60000);
    if (!panel.above) throw new Error("The panel is not above other windows.");
    await waitFor(() => {
      const picture = x("import", ["-window", panel.id, "png:-"], { encoding: "buffer" }).stdout;
      return picture.length > 0 && Number(x("identify", ["-format", "%k", "png:-"], { input: picture }).stdout) >= 16;
    }, "native panel content", signal, 60000);
    x("xdotool", ["windowactivate", named(OTHER_TITLE).id]);
    await waitFor(() => clients().at(-1)?.id === panel.id, "native panel above the raised window", signal);
    const bytes = x("import", ["-window", "root", "png:-"], { encoding: "buffer" }).stdout;
    const entry = nativeEntry(view, bytes);
    writeFileSync(join(output, entry.file), bytes, { flag: "wx" });
    return entry;
  } finally {
    for (const dispose of cleanup.reverse()) await dispose();
  }
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
    await send("Page.addScriptToEvaluateOnNewDocument", { source: linuxShell(source) });
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
    await evaluate("EzerbotNotifications.focus(false)");
    await bounded(evaluate(`(async () => {
      const post = async (path, body) => { const r = await fetch(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)}); if (!r.ok) throw Error('fixture request failed'); return r.json(); };
      const {conversation} = await post('/api/conversations', {title:'Préparer la semaine'});
      window.captureConversationId = conversation.id;
      await post('/api/send', {conversation_id:conversation.id, message_id:'linux-capture-message', body:'Bonjour, prépare une courte liste pour organiser ma semaine.'});
      window.captureToolConversation = async () => {
        const {conversation} = await post('/api/conversations', {title:'Ranger le dossier'});
        window.captureToolRan = () => GrokbotPanels.state.conversations.find(item => item.id === conversation.id)?.messages?.some(message => message.kind === 'tool' && message.payload?.call?.name === 'list_directory' && message.payload.outcome?.status === 'done');
        await post('/api/send', {conversation_id:conversation.id, message_id:'linux-capture-tools', body:'Regarde le dossier /tmp'});
        await GrokbotPanels.refresh();
        GrokbotPanels.openConversation(conversation.id);
      };
      window.captureScreenConsent = async () => {
        await post('/api/screen/bot', {bot_id:'grokbot', enabled:true});
        GrokbotPanels.openConversation(window.captureConversationId);
        await GrokbotPanels.refresh();
        document.querySelector('.screen-button').click();
      };
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
      const file = `${view.id}.png`;
      if (view.native) {
        stage = `${view.id} helper`;
        await overlayCapture(binary, join(output, file), cancelled.signal, cleanup, view.compositor);
        views.push({ id: view.id, label: view.label, file, renderer: "gtk-screen-helper-xvfb", ...pngInfo(readFileSync(join(output, file))) });
        continue;
      }
      stage = `${view.id} readiness`;
      await bounded(evaluate(`document.querySelectorAll('dialog[open]').forEach(dialog => dialog.close()); ${view.before ?? ''}; ${view.action}`), stage, cancelled.signal);
      await waitFor(() => check(view.ready), stage, cancelled.signal);
      stage = `${view.id} fonts, images and paint`;
      await bounded(evaluate(`(async () => {
        await document.fonts.ready;
        await Promise.all([...document.images].filter(image => image.getClientRects().length).map(async image => { await image.decode(); if (!image.naturalWidth) throw Error('Image did not decode'); }));
        document.activeElement?.blur();
        window.scrollTo(0,0); document.querySelectorAll('dialog[open],#chat-scroll,#conversation-list').forEach(element => element.scrollTop=0);
        ${view.step ?? ''};
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      })()`), stage, cancelled.signal);
      await waitFor(() => check(view.settled ?? view.ready), `${view.id} after paint`, cancelled.signal);
      if (browserFailure) throw new Error(browserFailure);
      stage = `${view.id} screenshot`;
      const { data } = await send("Page.captureScreenshot", { format: "png", fromSurface: true, captureBeyondViewport: false });
      writeFileSync(join(output, file), Buffer.from(data, "base64"), { flag: "wx" });
      views.push({ id: view.id, label: view.label, file, renderer: "chromium-web-fallback", ...pngInfo(readFileSync(join(output, file))) });
    }
    stage = "native panel";
    views.push(await captureNative(join(dirname(binary), "ezerbot-linux"), output, cancelled.signal));
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

module.exports = { VIEWS, NATIVE_VIEWS, VIEWPORT, bounded, waitFor, pngInfo, stackingIds, hasAbove, nativeEntry, captureNative, capture };
