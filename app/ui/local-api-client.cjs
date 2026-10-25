const { randomBytes } = require("node:crypto");
const { spawn } = require("node:child_process");

function localOrigin(url) {
  const parsed = new URL(url);
  if (parsed.protocol !== "http:" || parsed.hostname !== "127.0.0.1" || !parsed.port || parsed.username || parsed.password) {
    throw new Error("expected a desktop wrapper loopback URL");
  }
  return parsed.origin;
}

function connectCdp(url) {
  const ws = new WebSocket(url);
  const pending = new Map();
  const listeners = new Map();
  let next = 0;
  ws.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data));
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error("devtools command failed"));
      else resolve(message.result || {});
      return;
    }
    for (const callback of listeners.get(message.method) || []) callback(message.params || {});
  });
  ws.addEventListener("close", () => {
    for (const { reject } of pending.values()) reject(new Error("devtools websocket closed"));
    pending.clear();
  });
  return new Promise((resolve, reject) => {
    ws.addEventListener("open", () => resolve({
      send(method, params = {}) {
        return new Promise((resolveSend, rejectSend) => {
          const id = ++next;
          pending.set(id, { resolve: resolveSend, reject: rejectSend });
          ws.send(JSON.stringify({ id, method, params }));
        });
      },
      on(method, callback) {
        const callbacks = listeners.get(method) || new Set();
        callbacks.add(callback);
        listeners.set(method, callbacks);
        return () => callbacks.delete(callback);
      },
      close() { ws.close(); },
    }));
    ws.addEventListener("error", () => reject(new Error("devtools websocket failed")));
  });
}

class LaunchIdentity {
  #secret = randomBytes(32);
  #origin;

  spawn(binary, args, options = {}) {
    const stdio = options.stdio || ["pipe", "pipe", "pipe"];
    if (!Array.isArray(stdio)) throw new Error("launch authentication requires explicit stdio");
    const child = spawn(binary, args, { ...options, stdio: ["pipe", ...stdio.slice(1)] });
    child.stdin.on("error", () => {});
    child.stdin.end(this.#secret);
    return child;
  }

  headers(origin) {
    const parsed = localOrigin(origin);
    if (this.#origin && this.#origin !== parsed) throw new Error("desktop wrapper origin changed");
    this.#origin = parsed;
    return { "X-Ezerbot-Launch": this.#secret.toString("hex"), "X-Grokbot-Request": "local", Origin: parsed };
  }

  async fetch(url, options = {}) {
    const headers = new Headers(options.headers);
    for (const [name, value] of Object.entries(this.headers(url))) headers.set(name, value);
    try {
      return await fetch(url, { ...options, headers, redirect: "error" });
    } catch {
      throw new Error("desktop wrapper request failed");
    }
  }

  async authenticateBrowser(cdp, origin) {
    const values = this.headers(origin);
    const authenticated = new Set();
    let rejectFailure;
    const failed = new Promise((_, reject) => { rejectFailure = reject; });
    failed.catch(() => {});
    cdp.on("Fetch.requestPaused", (params) => {
      (async () => {
        const { requestId, redirectedRequestId, networkId, request } = params;
        if (Object.hasOwn(params, "responseStatusCode") || Object.hasOwn(params, "responseErrorReason")) {
          const wasAuthenticated = authenticated.has(requestId) || authenticated.has(networkId);
          authenticated.delete(requestId);
          if (networkId) authenticated.delete(networkId);
          if (wasAuthenticated && [301, 302, 303, 307, 308].includes(params.responseStatusCode)) {
            await cdp.send("Fetch.failRequest", { requestId, errorReason: "BlockedByClient" });
          } else {
            await cdp.send("Fetch.continueRequest", { requestId });
          }
          return;
        }
        if (authenticated.has(redirectedRequestId) || (redirectedRequestId && authenticated.has(networkId))) {
          await cdp.send("Fetch.failRequest", { requestId, errorReason: "BlockedByClient" });
          return;
        }
        const headers = Object.entries(request.headers || {})
          .filter(([name]) => name.toLowerCase() !== "x-ezerbot-launch")
          .map(([name, value]) => ({ name, value: String(value) }));
        let sameOrigin = false;
        try { sameOrigin = localOrigin(request.url) === this.#origin; } catch {}
        if (sameOrigin) {
          for (const [name, value] of Object.entries(values)) {
            if (!headers.some((header) => header.name.toLowerCase() === name.toLowerCase())) headers.push({ name, value });
          }
          authenticated.add(requestId);
          if (networkId) authenticated.add(networkId);
        }
        await cdp.send("Fetch.continueRequest", { requestId, headers });
      })().catch(() => rejectFailure(new Error("browser authentication failed")));
    });
    await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
    await cdp.send("Fetch.enable", { patterns: ["Request", "Response"].map((requestStage) => ({ urlPattern: "*", requestStage })) });
    return { failed, settle() {} };
  }
}

module.exports = { LaunchIdentity, connectCdp };
