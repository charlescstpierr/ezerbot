const { spawn, execFileSync } = require("node:child_process");
const { createHash, randomUUID } = require("node:crypto");
const { EventEmitter } = require("node:events");
const { accessSync, constants, lstatSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, unlinkSync } = require("node:fs");
const { tmpdir, constants: osConstants } = require("node:os");
const { basename, dirname, isAbsolute, join, resolve } = require("node:path");
const { performance } = require("node:perf_hooks");
const { PassThrough } = require("node:stream");

const START_MS = 5000;
const MAX_MESSAGE = 1024 * 1024;
const MAX_OUTPUT_QUEUE = 4 * 1024 * 1024;
const MAX_PROCESS_TABLE = 16 * 1024 * 1024;
const now = () => performance.now();

function deferred() {
  let resolvePromise;
  let rejectPromise;
  const promise = new Promise((resolveValue, rejectValue) => {
    resolvePromise = resolveValue;
    rejectPromise = rejectValue;
  });
  promise.catch(() => {});
  return { promise, resolve: resolvePromise, reject: rejectPromise };
}

function beforeDeadline(work, deadline, message) {
  let timer;
  const expired = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), Math.max(0, deadline - now()));
  });
  return Promise.race([work, expired]).finally(() => clearTimeout(timer));
}

function stopBudget(value = 10000) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > 60000) throw new Error("invalid fixture cleanup budget");
  return value;
}

function rootIdentity(root) {
  if (typeof root !== "string" || !isAbsolute(root) || resolve(root) !== root || dirname(root) !== resolve(tmpdir()) || !basename(root)) {
    throw new Error("refusing a fixture root outside the temporary directory");
  }
  const stat = lstatSync(root, { bigint: true });
  const canonical = realpathSync(root);
  if (!stat.isDirectory() || stat.isSymbolicLink() || dirname(canonical) !== realpathSync(tmpdir())) {
    throw new Error("refusing a fixture root that is not a direct temporary directory");
  }
  return { dev: String(stat.dev), ino: String(stat.ino), canonical };
}

function sameRoot(left, right) {
  return left && right && left.dev === right.dev && left.ino === right.ino && left.canonical === right.canonical;
}

function nativeBinary() {
  if (process.platform !== "darwin" && process.platform !== "linux") throw new Error("fixture process ownership requires macOS or Linux");
  const source = join(__dirname, "fixture-process-owner.c");
  let contents;
  try { contents = readFileSync(source); } catch { throw new Error("fixture process owner source is unavailable"); }
  const digest = createHash("sha256").update(contents).update(process.platform).update(process.arch).digest("hex");
  const cache = resolve(__dirname, "../../target/fixture-owner");
  const binary = join(cache, `${process.platform}-${process.arch}-${digest}`);
  try { accessSync(binary, constants.X_OK); return binary; } catch {}
  mkdirSync(cache, { recursive: true });
  const partial = `${binary}.${randomUUID()}`;
  try {
    execFileSync("cc", ["-std=c11", "-Wall", "-Wextra", "-O2", source, "-o", partial], {
      timeout: 30000, stdio: ["ignore", "pipe", "pipe"], maxBuffer: MAX_MESSAGE,
    });
    renameSync(partial, binary);
  } catch {
    try { unlinkSync(partial); } catch {}
    throw new Error("fixture process owner compilation failed");
  }
  return binary;
}

function launchSpec(command, args, options = {}) {
  if (typeof command !== "string" || !command || command.includes("\0") || command.length > 16384) throw new Error("invalid fixture command");
  if (!Array.isArray(args) || args.length > 1024 || args.some((arg) => typeof arg !== "string" || arg.includes("\0"))) throw new Error("invalid fixture arguments");
  if (!options || typeof options !== "object" || Array.isArray(options)) throw new Error("invalid fixture launch options");
  const authenticated = options.authenticated ?? false;
  if (typeof authenticated !== "boolean") throw new Error("invalid fixture authentication request");
  let env;
  if (options.env !== undefined) {
    if (!options.env || typeof options.env !== "object" || Array.isArray(options.env)) throw new Error("invalid fixture launch environment");
    env = {};
    for (const [key, value] of Object.entries(options.env)) {
      if (value === undefined) continue;
      if (!key || key.includes("=") || key.includes("\0") || typeof value !== "string" || value.includes("\0")) {
        throw new Error("invalid fixture launch environment");
      }
      Object.defineProperty(env, key, { value, enumerable: true, configurable: true, writable: true });
    }
  }
  const spec = { command, args: [...args], env, authenticated };
  if (Buffer.byteLength(JSON.stringify(spec)) > MAX_MESSAGE) throw new Error("fixture launch specification is too large");
  return spec;
}

class OwnedProcess extends EventEmitter {
  #kill;
  #exited = false;
  #closed = false;
  #ended = new Set();
  #notifications = [];

  constructor(kill) {
    super();
    this.#kill = kill;
    this.pid = undefined;
    this.exitCode = null;
    this.signalCode = null;
    this.stdout = new PassThrough();
    this.stderr = new PassThrough();
    this.on("error", () => {});
  }

  kill(signal = "SIGKILL") {
    if (signal !== "SIGKILL") throw new Error("fixture owned processes support only SIGKILL");
    if (this.#exited || this.#closed) return false;
    return this.#kill();
  }

  _publish() {
    const pending = this.#notifications;
    this.#notifications = null;
    for (const [event, ...args] of pending) this.emit(event, ...args);
  }

  _notify(event, ...args) {
    if (this.#notifications) this.#notifications.push([event, ...args]);
    else this.emit(event, ...args);
  }

  _data(stream, chunk) {
    if (!this.#closed && !this.#ended.has(stream)) this[stream].write(Buffer.from(chunk, "base64"));
  }

  _end(stream) {
    if (this.#ended.has(stream)) return;
    this.#ended.add(stream);
    this[stream].end();
    this._maybeClose();
  }

  _exit(code, signal) {
    if (this.#exited) return;
    this.#exited = true;
    this.exitCode = code;
    this.signalCode = signal;
    this._notify("exit", code, signal);
    this._maybeClose();
  }

  _error(message) { this._notify("error", new Error(message)); }

  _lost(message) {
    if (this.#closed) return;
    this._error(message);
    this.#exited = true;
    this._end("stdout");
    this._end("stderr");
  }

  _maybeClose() {
    if (this.#closed || !this.#exited || this.#ended.size !== 2) return;
    this.#closed = true;
    this._notify("close", this.exitCode, this.signalCode);
  }
}

async function createFixtureOwner({ root, start = spawn }) {
  let identity;
  try { identity = rootIdentity(root); } catch { throw new Error("invalid fixture root"); }
  const binary = nativeBinary();
  let child;
  try {
    child = start(process.execPath, [__filename, "--owner", root, binary], {
      detached: true, stdio: ["ignore", "ignore", "ignore", "ipc"],
    });
  } catch { throw new Error("fixture owner could not start"); }
  if (!child || typeof child.send !== "function" || typeof child.on !== "function") throw new Error("fixture owner requires a private IPC channel");

  const ready = deferred();
  const pending = new Map();
  const launches = new Map();
  let phase = "starting";
  let nextRequest = 0;
  let closePromise;
  let failure;
  let lastWriters = [];

  function lost(message) {
    if (phase === "closed") return;
    phase = "closed";
    failure = new Error(message);
    ready.reject(failure);
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(failure); }
    pending.clear();
    for (const owned of launches.values()) owned._lost(message);
  }

  function disconnect() {
    if (child.connected) {
      try { child.disconnect(); } catch {}
    }
  }

  function request(type, values, ms = START_MS) {
    if (phase === "closed" || !child.connected) return Promise.reject(failure || new Error("fixture owner channel is closed"));
    const requestId = ++nextRequest;
    return new Promise((resolveRequest, rejectRequest) => {
      const timer = setTimeout(() => {
        pending.delete(requestId);
        const observed = lastWriters.length ? `; last observed ${lastWriters.join(", ")}` : "";
        rejectRequest(new Error(type === "close" ? `fixture cleanup response timed out${observed}; deletion is indeterminate` : "fixture owner request timed out"));
        disconnect();
      }, ms);
      pending.set(requestId, { resolve: resolveRequest, reject: rejectRequest, timer });
      try {
        child.send({ type, request: requestId, ...values }, (error) => {
          if (!error) return;
          lost("fixture owner channel failed");
          disconnect();
        });
      } catch {
        lost("fixture owner channel failed");
        disconnect();
      }
    });
  }

  child.on("message", (message) => {
    if (!message || typeof message !== "object") { lost("fixture owner protocol failed"); disconnect(); return; }
    if (message.type === "ready" && phase === "starting") { phase = "open"; ready.resolve(); return; }
    if (message.type === "inspection" && Array.isArray(message.writers) && message.writers.every((row) =>
      row && Number.isSafeInteger(row.pid) && row.pid > 1 && Number.isSafeInteger(row.pgid) && row.pgid >= 0)) {
      lastWriters = message.writers.map((row) => `process ${row.pid} (group ${row.pgid})`);
      return;
    }
    if (message.type === "response" || message.type === "spawned") {
      const entry = pending.get(message.request);
      if (!entry) return;
      pending.delete(message.request);
      clearTimeout(entry.timer);
      if (message.type === "spawned") {
        const owned = launches.get(message.launch);
        if (!owned || !Number.isSafeInteger(message.pid) || message.pid <= 1) {
          entry.reject(new Error("fixture owner launch acknowledgment failed"));
          lost("fixture owner protocol failed");
          disconnect();
          return;
        }
        owned.pid = message.pid;
        entry.resolve(owned);
        // IPC can batch terminal messages before the caller's await has received the handle.
        setImmediate(() => owned._publish());
      } else if (message.ok === true) entry.resolve(message.result);
      else entry.reject(new Error(typeof message.error === "string" ? message.error : "fixture owner request failed"));
      return;
    }
    const owned = launches.get(message.launch);
    if (!owned) { lost("fixture owner protocol failed"); disconnect(); return; }
    if (message.type === "data" && (message.stream === "stdout" || message.stream === "stderr") && typeof message.chunk === "string") {
      owned._data(message.stream, message.chunk);
    } else if (message.type === "stream-end" && (message.stream === "stdout" || message.stream === "stderr")) {
      owned._end(message.stream);
    } else if (message.type === "target-exit") {
      owned._exit(message.code, message.signal);
    } else if (message.type === "target-error") {
      owned._error("fixture payload launch or ownership failed");
    } else { lost("fixture owner protocol failed"); disconnect(); }
  });
  child.on("error", () => { lost("fixture owner could not start"); disconnect(); });
  child.on("disconnect", () => lost("fixture owner channel closed"));
  child.on("close", () => lost("fixture owner exited"));

  try {
    child.send({ type: "init", identity }, (error) => {
      if (error) { lost("fixture owner initialization failed"); disconnect(); }
    });
    await beforeDeadline(ready.promise, now() + START_MS, "fixture owner did not become ready");
  } catch (error) {
    disconnect();
    throw error;
  }

  return Object.freeze({
    root,
    async spawn(command, args, options = {}) {
      const spec = launchSpec(command, args, options);
      if (phase !== "open") throw new Error("fixture owner launch admission is closed");
      const launch = randomUUID();
      const owned = new OwnedProcess(() => {
        if (phase !== "open") return false;
        request("kill", { launch }).catch(() => owned._error("fixture owned target could not be stopped"));
        return true;
      });
      launches.set(launch, owned);
      return request("spawn", { launch, ...spec });
    },
    close(options = {}) {
      if (closePromise) return closePromise;
      const stopMs = stopBudget(options.stopMs);
      if (phase === "open") phase = "closing";
      closePromise = request("close", { stopMs }, stopMs).finally(() => {
        phase = "closed";
        for (const entry of pending.values()) {
          clearTimeout(entry.timer);
          entry.reject(new Error("fixture owner is closed"));
        }
        pending.clear();
        disconnect();
      });
      closePromise.catch(() => {});
      return closePromise;
    },
  });
}

function namesDirectory(command, directory) {
  const before = new Set([" ", "\t", "=", ":", ",", '"', "'"]);
  const after = new Set(["/", " ", "\t", ",", ":", '"', "'"]);
  for (let at = command.indexOf(directory); at >= 0; at = command.indexOf(directory, at + 1)) {
    const previous = at === 0 ? undefined : command[at - 1];
    const next = command[at + directory.length];
    if ((previous === undefined || before.has(previous)) && (next === undefined || after.has(next))) return true;
  }
  return false;
}

function processTable(deadline) {
  return new Promise((resolveTable, rejectTable) => {
    if (now() >= deadline) { rejectTable(new Error("no cleanup time remains for process inspection")); return; }
    const threaded = process.platform === "linux";
    const columns = threaded ? "pid=,pgid=,stat=,nlwp=,command=" : "pid=,pgid=,stat=,command=";
    let inspector;
    try { inspector = spawn("ps", ["-axww", "-o", columns], { stdio: ["ignore", "pipe", "pipe"] }); }
    catch { rejectTable(new Error("fixture process inspection could not start")); return; }
    let terminal = false;
    let settled = false;
    let listing = "";
    let bytes = 0;
    const fail = (message) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (!terminal) inspector.kill("SIGKILL");
      inspector.stdout.destroy();
      rejectTable(new Error(message));
    };
    const timer = setTimeout(() => fail("fixture process inspection timed out"), Math.max(0, deadline - now()));
    inspector.stdout.setEncoding("utf8");
    inspector.stdout.on("data", (chunk) => {
      bytes += Buffer.byteLength(chunk);
      if (bytes > MAX_PROCESS_TABLE) { fail("fixture process inspection was truncated"); return; }
      listing += chunk;
    });
    inspector.stdout.on("error", () => fail("fixture process inspection output failed"));
    inspector.stderr.on("data", () => fail("fixture process inspection reported an error"));
    inspector.stderr.on("error", () => fail("fixture process inspection error channel failed"));
    inspector.on("error", () => fail("fixture process inspection could not start"));
    inspector.on("exit", () => { terminal = true; });
    inspector.on("close", (code, signal) => {
      if (settled) return;
      clearTimeout(timer);
      settled = true;
      if (code !== 0 || signal || now() >= deadline || (listing && !listing.endsWith("\n"))) {
        rejectTable(new Error("fixture process inspection did not complete"));
        return;
      }
      const rows = [];
      for (const line of listing.split("\n")) {
        if (!line.trim()) continue;
        const match = line.match(threaded ? /^\s*(\d+)\s+(\d+)\s+(\S+)\s+(\d+)\s?(.*)$/ : /^\s*(\d+)\s+(\d+)\s+(\S+)\s?(.*)$/);
        if (!match) { rejectTable(new Error("fixture process inspection returned an invalid row")); return; }
        const pid = Number(match[1]);
        const pgid = Number(match[2]);
        const threads = threaded ? Number(match[4]) : 1;
        if (!Number.isSafeInteger(pid) || pid < 1 || !Number.isSafeInteger(pgid) || pgid < 0 || !Number.isSafeInteger(threads) || threads < 1) {
          rejectTable(new Error("fixture process inspection returned an invalid identity"));
          return;
        }
        rows.push({ pid, pgid, stat: match[3], threads, command: match[threaded ? 5 : 4] });
      }
      if (!rows.some((row) => row.pid === process.pid) || !rows.some((row) => row.pid === inspector.pid)) {
        rejectTable(new Error("fixture process inspection was incomplete"));
        return;
      }
      resolveTable({ rows, inspector: inspector.pid });
    });
  });
}

async function waitForWriters(root, identity, records, deadline, report) {
  const names = [...new Set([root, identity.canonical])];
  const groups = new Set([...records.values()].map((record) => record.anchor).filter(Boolean));
  let last = [];
  for (;;) {
    let table;
    try { table = await processTable(deadline); }
    catch (error) {
      if (last.length) throw new Error(`fixture writers remain: ${last.join(", ")}`);
      throw error;
    }
    const live = table.rows.filter((row) => row.pid !== process.pid && row.pid !== table.inspector &&
      (!row.stat.startsWith("Z") || row.threads > 1));
    const writers = live.filter((row) => groups.has(row.pgid) || names.some((name) => namesDirectory(row.command, name)));
    const left = writers.map((row) => `process ${row.pid} (group ${row.pgid})`);
    if (left.join(",") !== last.join(",")) report(writers.map(({ pid, pgid }) => ({ pid, pgid })));
    if (now() >= deadline) throw new Error(left.length ? `fixture writers remain: ${left.join(", ")}` : "fixture writer exit was observed after the cleanup deadline");
    if (!left.length) return;
    last = left;
    await new Promise((resolveTick) => setTimeout(resolveTick, Math.min(20, Math.max(0, deadline - now()))));
  }
}

function removeRoot(root, identity, deadline) {
  return new Promise((resolveRemoval, rejectRemoval) => {
    if (now() >= deadline) { rejectRemoval(new Error("fixture cleanup deadline expired before removal; directory retained")); return; }
    let worker;
    try {
      worker = spawn(process.execPath, [__filename, "--remove", root, JSON.stringify(identity)], {
        stdio: ["ignore", "ignore", "ignore", "ipc"],
      });
    } catch { rejectRemoval(new Error("fixture removal worker could not start; directory retained")); return; }
    let terminal = false;
    let settled = false;
    let receipt;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      if (!terminal) worker.kill("SIGKILL");
      rejectRemoval(new Error("fixture removal timed out; deletion is indeterminate"));
    }, Math.max(0, deadline - now()));
    worker.on("message", (message) => { receipt = message; });
    worker.on("error", () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      rejectRemoval(new Error("fixture removal worker failed; deletion is indeterminate"));
    });
    worker.on("exit", () => { terminal = true; });
    worker.on("close", (code, signal) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (code === 0 && !signal && receipt?.removed === true && now() < deadline) resolveRemoval({ removed: true });
      else if (receipt?.retained === true) rejectRemoval(new Error("fixture root identity changed; directory retained"));
      else rejectRemoval(new Error("fixture removal failed; deletion is indeterminate"));
    });
  });
}

function readSecret() {
  return new Promise((resolveSecret, rejectSecret) => {
    const chunks = [];
    let length = 0;
    process.stdin.on("data", (chunk) => {
      length += chunk.length;
      if (length > 32) {
        for (const held of chunks) held.fill(0);
        chunk.fill(0);
        chunks.length = 0;
        rejectSecret(new Error("invalid fixture authentication channel"));
      } else chunks.push(chunk);
    });
    process.stdin.on("error", () => rejectSecret(new Error("fixture authentication channel failed")));
    process.stdin.on("end", () => {
      if (length !== 0 && length !== 32) {
        for (const held of chunks) held.fill(0);
        rejectSecret(new Error("invalid fixture authentication channel"));
        return;
      }
      const secret = Buffer.concat(chunks);
      for (const held of chunks) held.fill(0);
      resolveSecret(secret);
    });
    process.stdin.resume();
  });
}

function runOwner(root, binary) {
  const records = new Map();
  let identity;
  let secret = Buffer.alloc(0);
  let initialized = false;
  let authenticatedLaunch;
  let phase = "starting";
  let failure;
  let lastRequest = 0;
  let outputQueue = 0;
  let closePromise;
  const initialization = deferred();

  function poison(message) {
    failure ||= message;
    if (phase !== "closing" && phase !== "closed") close().catch(() => {});
  }

  function send(message) {
    if (!process.connected) return false;
    let size;
    try { size = Buffer.byteLength(JSON.stringify(message)); } catch { poison("fixture owner output failed"); return false; }
    if (size > MAX_MESSAGE || outputQueue + size > MAX_OUTPUT_QUEUE) { poison("fixture owner output exceeded its bound"); return false; }
    outputQueue += size;
    try {
      process.send(message, (error) => {
        outputQueue -= size;
        if (error && phase !== "closed") poison("fixture owner output channel failed");
      });
      return true;
    } catch {
      outputQueue -= size;
      poison("fixture owner output channel failed");
      return false;
    }
  }

  function response(request, ok, result, error) { send({ type: "response", request, ok, result, error }); }

  function failRecord(record, message) {
    if (record.failure) return;
    record.failure = message;
    record.ready.reject(new Error(message));
    clearTimeout(record.timer);
    if (!record.replied) {
      record.replied = true;
      response(record.request, false, undefined, "fixture payload launch or ownership failed");
    } else send({ type: "target-error", launch: record.id });
    poison(message);
  }

  function command(record, text, deadline) {
    return beforeDeadline(new Promise((resolveCommand, rejectCommand) => {
      const control = record.helper?.stdio[3];
      if (!control || control.destroyed || !control.writable) { rejectCommand(new Error("fixture native control channel is closed")); return; }
      try { control.write(`${text}\n`, (error) => error ? rejectCommand(new Error("fixture native command failed")) : resolveCommand()); }
      catch { rejectCommand(new Error("fixture native command failed")); }
    }), deadline, "fixture native command timed out");
  }

  function nativeEvent(record, message) {
    if (!message || typeof message !== "object" || typeof message.event !== "string" ||
        !Number.isSafeInteger(message.pid) || message.pid < 0 || !Number.isSafeInteger(message.value)) {
      failRecord(record, "fixture native event is invalid");
      return;
    }
    if (message.event === "ready" && !record.anchor && record.finished === undefined && message.pid > 1 && message.value === 0) {
      record.anchor = message.pid;
      record.ready.resolve();
    } else if (message.event === "started" && record.anchor && !record.target && record.finished === undefined && message.pid > 1 && message.value === 0) {
      record.target = message.pid;
      clearTimeout(record.timer);
      if (!record.replied) {
        record.replied = true;
        send({ type: "spawned", request: record.request, launch: record.id, pid: record.target });
      }
    } else if (message.event === "target-signal" && !record.exited && record.finished === undefined && message.pid > 1 &&
        (record.target === message.pid || (!record.target && phase === "closing")) && message.value > 0) {
      record.target ||= message.pid;
      record.signal = Object.entries(osConstants.signals).find(([, value]) => value === message.value)?.[0];
      if (!record.signal) failRecord(record, "fixture native target signal is invalid");
    } else if (message.event === "exited" && !record.exited && record.finished === undefined && message.pid > 1 && message.value >= 0 && message.value <= 255) {
      record.exited = true;
      if (record.target && record.target !== message.pid) { failRecord(record, "fixture native target identity changed"); return; }
      record.target ||= message.pid;
      send({ type: "target-exit", launch: record.id, code: record.signal ? null : message.value, signal: record.signal || null });
    } else if (message.event === "stopped" && !record.stopped && record.finished === undefined && message.value === 0 &&
        ((message.pid > 1 && (message.pid === record.anchor || (!record.anchor && phase === "closing"))) ||
         (message.pid === 0 && !record.anchor && !record.target && phase === "closing"))) {
      record.anchor ||= message.pid;
      record.stopped = true;
    } else if (message.event === "finished" && record.finished === undefined && message.value === 0 &&
        (message.pid === record.anchor || (!record.anchor && message.pid === 0))) {
      record.finished = 0;
    } else if (message.event === "target-terminal" && record.exited && record.finished === undefined && message.pid === record.target && message.value === 0) {
      return;
    } else if (message.event === "exec-failed") {
      failRecord(record, "fixture payload exec failed");
    } else failRecord(record, "fixture native ownership protocol failed");
  }

  async function launch(request, id, spec) {
    const record = {
      id, request, helper: null, anchor: null, target: null, ready: deferred(), complete: deferred(),
      replied: false, exited: false, stopped: false, finished: undefined, failure: undefined, signal: null,
    };
    records.set(id, record);
    record.timer = setTimeout(() => failRecord(record, "fixture native launch timed out"), START_MS);
    try {
      const helper = spawn(binary, [spec.command, ...spec.args], {
        env: spec.env, detached: true, stdio: ["pipe", "pipe", "pipe", "pipe", "pipe"],
      });
      record.helper = helper;
      helper.stdin.on("error", () => {});
      helper.stdio[3].on("error", () => failRecord(record, "fixture native control channel failed"));
      for (const stream of ["stdout", "stderr"]) {
        helper[stream].on("data", (chunk) => send({ type: "data", launch: id, stream, chunk: chunk.toString("base64") }));
        helper[stream].on("end", () => send({ type: "stream-end", launch: id, stream }));
        helper[stream].on("error", () => failRecord(record, "fixture payload output channel failed"));
      }
      let buffer = "";
      helper.stdio[4].setEncoding("utf8");
      helper.stdio[4].on("data", (chunk) => {
        buffer += chunk;
        if (buffer.length > 8192) { failRecord(record, "fixture native event exceeded its bound"); helper.stdio[4].destroy(); return; }
        for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
          const line = buffer.slice(0, end);
          buffer = buffer.slice(end + 1);
          if (line.length > 512) { failRecord(record, "fixture native event exceeded its bound"); continue; }
          try { nativeEvent(record, JSON.parse(line)); } catch { failRecord(record, "fixture native event could not be decoded"); }
        }
      });
      helper.stdio[4].on("error", () => failRecord(record, "fixture native event channel failed"));
      helper.on("error", () => failRecord(record, "fixture native owner could not start"));
      helper.on("close", (code, signal) => {
        clearTimeout(record.timer);
        if (buffer || code !== 0 || signal || record.finished !== 0 || (record.target && !record.exited)) {
          failRecord(record, "fixture native owner did not finish successfully");
        }
        record.complete.resolve();
      });
      helper.stdin.end(spec.authenticated ? secret : undefined);
      await record.ready.promise;
      if (phase !== "open") return;
      await command(record, "go", now() + START_MS);
    } catch {
      failRecord(record, "fixture useful launch was not released");
      if (!record.helper) record.complete.resolve();
    }
  }

  function close(stopMs = 10000) {
    if (closePromise) return closePromise;
    phase = "closing";
    clearTimeout(startupTimer);
    const deadline = now() + stopMs;
    closePromise = (async () => {
      const finishes = [...records.values()].map(async (record) => {
        clearTimeout(record.timer);
        if (!record.replied) {
          record.replied = true;
          response(record.request, false, undefined, "fixture launch admission closed before release");
        }
        record.ready.resolve();
        if (record.finished === 0) return record.complete.promise;
        try { await command(record, "finish", deadline); }
        catch { failRecord(record, "fixture native shutdown command failed"); record.helper?.stdio[3]?.destroy(); }
        await record.complete.promise;
      });
      try { await beforeDeadline(Promise.all(finishes), deadline, "fixture native shutdown timed out"); }
      catch {
        failure ||= "fixture native shutdown timed out";
        for (const record of records.values()) record.helper?.stdio[3]?.destroy();
      }
      if (!initialized || !identity || failure) throw new Error(`fixture directory ${root} kept: ${failure || "fixture owner initialization was incomplete"}`);
      try { await waitForWriters(root, identity, records, deadline, (writers) => send({ type: "inspection", writers })); }
      catch (error) { throw new Error(`fixture directory ${root} kept: ${error.message}`); }
      return removeRoot(root, identity, deadline);
    })();
    closePromise.catch(() => {});
    return closePromise;
  }

  async function terminate(request, stopMs) {
    let result;
    let error;
    try { result = await close(stopMs); } catch (caught) { error = caught.message; }
    phase = "closed";
    secret.fill(0);
    const exit = () => {
      if (process.connected) {
        try { process.disconnect(); } catch {}
      }
      process.exit(error ? 1 : 0);
    };
    if (request && process.connected) {
      const timer = setTimeout(exit, 250);
      try {
        process.send({ type: "response", request, ok: !error, result, error }, () => {
          clearTimeout(timer);
          exit();
        });
      } catch { clearTimeout(timer); exit(); }
    } else {
      exit();
    }
  }

  const startupTimer = setTimeout(() => poison("fixture owner initialization timed out"), START_MS);
  process.on("disconnect", () => { terminate().catch(() => {}); });
  process.on("SIGTERM", () => { terminate().catch(() => {}); });
  process.on("SIGINT", () => { terminate().catch(() => {}); });
  process.on("SIGHUP", () => { terminate().catch(() => {}); });
  process.on("message", (message) => {
    try {
      if (!message || typeof message !== "object" || Buffer.byteLength(JSON.stringify(message)) > MAX_MESSAGE) throw new Error("invalid fixture owner request");
      if (message.type === "init" && phase === "starting" && !initialized) {
        identity = rootIdentity(root);
        if (!sameRoot(identity, message.identity)) throw new Error("fixture root identity changed");
        initialized = true;
        initialization.resolve();
        return;
      }
      if (!Number.isSafeInteger(message.request) || message.request <= lastRequest) throw new Error("invalid fixture owner request order");
      lastRequest = message.request;
      if (message.type === "close") {
        terminate(message.request, stopBudget(message.stopMs)).catch(() => {});
        return;
      }
      if (phase !== "open") { response(message.request, false, undefined, "fixture owner launch admission is closed"); return; }
      if (message.type === "spawn") {
        if (typeof message.launch !== "string" || !/^[0-9a-f-]{36}$/.test(message.launch) || records.has(message.launch)) throw new Error("invalid fixture launch identity");
        let spec;
        try { spec = launchSpec(message.command, message.args, message); }
        catch { response(message.request, false, undefined, "invalid fixture launch specification"); return; }
        if (spec.authenticated && (secret.length !== 32 || authenticatedLaunch)) {
          response(message.request, false, undefined, "fixture authentication channel is unavailable");
          return;
        }
        if (spec.authenticated) authenticatedLaunch = message.launch;
        launch(message.request, message.launch, spec).catch(() => poison("fixture launch failed"));
      } else if (message.type === "kill") {
        const record = records.get(message.launch);
        if (!record || !record.target) { response(message.request, false, undefined, "unknown fixture owned target"); return; }
        if (record.exited) { response(message.request, true, false); return; }
        command(record, "kill-target", now() + START_MS).then(
          () => response(message.request, true, true),
          () => { failRecord(record, "fixture owned target stop failed"); response(message.request, false, undefined, "fixture owned target stop failed"); },
        );
      } else throw new Error("invalid fixture owner command");
    } catch { poison("fixture owner control protocol failed"); }
  });

  Promise.all([initialization.promise, readSecret()]).then(([, supplied]) => {
    secret = supplied;
    if (phase !== "starting") { secret.fill(0); return; }
    phase = "open";
    clearTimeout(startupTimer);
    send({ type: "ready" });
  }, () => poison("fixture owner authentication initialization failed"));
}

function runRemoval(root, encoded) {
  let expected;
  try {
    expected = JSON.parse(encoded);
    if (!sameRoot(rootIdentity(root), expected)) throw new Error("changed root");
  } catch {
    if (process.send) process.send({ retained: true }, () => { process.disconnect(); process.exitCode = 1; });
    else process.exitCode = 1;
    return;
  }
  try {
    rmSync(root, { recursive: true, force: false, maxRetries: 0 });
    if (process.send) process.send({ removed: true }, () => { process.disconnect(); });
  } catch {
    if (process.send) process.send({ indeterminate: true }, () => { process.disconnect(); process.exitCode = 1; });
    else process.exitCode = 1;
  }
}

if (require.main === module) {
  if (process.argv[2] === "--owner" && process.argv.length === 5 && process.send) runOwner(process.argv[3], process.argv[4]);
  else if (process.argv[2] === "--remove" && process.argv.length === 5) runRemoval(process.argv[3], process.argv[4]);
  else process.exitCode = 2;
}

module.exports = { createFixtureOwner, nativeBinary };
