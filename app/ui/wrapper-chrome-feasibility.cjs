const assert = require('node:assert/strict');
const { spawn, fork, execFile } = require('node:child_process');
const { once } = require('node:events');
const { mkdirSync, mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } = require('node:fs');
const { join, resolve } = require('node:path');
const { createHash } = require('node:crypto');
const { promisify } = require('node:util');
const exec = promisify(execFile);
const pause = ms => new Promise(r => setTimeout(r, ms));
const invokingParent = process.ppid;
const base = process.env.PROBE_EVIDENCE;
if (!base) throw new Error('PROBE_EVIDENCE must name a dedicated proof directory');
const repo = resolve(__dirname, '../..');
const binary = require(join(repo, 'app/ui/chrome-binary.cjs')).pinnedFixtureShell();
const bounds = async (read, ms = 10000) => {
  const end = Date.now() + ms;
  do { const value = await read(end); if (value) return value; await pause(20); } while (Date.now() < end);
  throw new Error('probe deadline exceeded');
};
async function holders(root, end) {
  try {
    const { stdout } = await exec('lsof', ['-nP', '-Fpn', '+D', root], { timeout: Math.max(1, end - Date.now()), killSignal: 'SIGKILL' });
    return [...new Set(stdout.split('\n').filter(s => /^p\d+$/.test(s)).map(s => Number(s.slice(1))))];
  } catch (e) {
    if (e.code === 1 && !e.stderr) return [...new Set((e.stdout || '').split('\n').filter(s => /^p\d+$/.test(s)).map(s => Number(s.slice(1))))];
    throw new Error(`profile-holder inspection failed code=${e.code} signal=${e.signal} killed=${e.killed} stderrBytes=${(e.stderr || '').length}`);
  }
}
async function inventory(root, browser) {
  const { stdout } = await exec('ps', ['-axww', '-o', 'pid=,ppid=,stat=,command='], { timeout: 3000 });
  const rows = stdout.split('\n').map(s => s.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/)).filter(Boolean)
    .map(([, p, pp, state, command]) => ({ pid: +p, parent: +pp, state, command }));
  const found = new Set([browser]);
  for (let i = 0; i < rows.length; i++) for (const r of rows) if (r.pid !== process.pid && r.pid !== invokingParent && (found.has(r.parent) || r.command.includes(root))) found.add(r.pid);
  return rows.filter(r => found.has(r.pid)).map(r => ({ pid: r.pid, parent: r.parent, state: r.state, type: r.command.match(/--type=([^ ]+)/)?.[1] || 'browser', crashpad: /crashpad/i.test(r.command) }));
}
async function guard() {
  const root = process.argv[3], mode = process.argv[4];
  let child, closed, finished = false;
  process.on('disconnect', async () => {
    if (finished) return;
    finished = true;
    const result = { mode, platform: process.platform, arch: process.arch, start: Date.now(), root, browser: child?.pid };
    try {
      if (child) {
        if (child.exitCode === null && child.signalCode === null) child.kill(mode);
        await Promise.race([closed, pause(10000).then(() => { throw new Error('owned child did not close'); })]);
      }
      result.childClosed = true;
      await bounds(async end => (await holders(root, end)).length === 0);
      result.remaining = await holders(root, Date.now() + 10000);
      result.after = await inventory(root, child.pid);
      assert.equal(result.after.filter(r => !r.state.startsWith('Z')).length, 0, 'writers remain after shell closes');
      rmSync(root, { recursive: true });
      result.removals = 1; result.ok = true;
    } catch (e) { result.ok = false; result.error = e.message; }
    result.end = Date.now();
    writeFileSync(root + '.result.json', JSON.stringify(result, null, 2));
    process.exit(result.ok ? 0 : 1);
  });
  child = spawn(binary, ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--use-mock-keychain', '--password-store=basic', '--remote-debugging-port=0', `--user-data-dir=${root}`, 'about:blank'], { stdio: ['ignore','ignore','pipe'] });
  child.stderr.resume();
  closed = once(child, 'close');
  await bounds(() => existsSync(join(root, 'DevToolsActivePort')));
  const port = Number(readFileSync(join(root, 'DevToolsActivePort'), 'utf8').split('\n')[0]);
  const targets = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(3000) }).then(r=>r.json());
  assert.ok(targets.some(t => t.type === 'page'));
  process.send({ browser: child.pid, before: await inventory(root, child.pid), holders: await holders(root, Date.now() + 10000) });
}
async function runner() {
  const owner = fork(__filename, ['guard', process.argv[3], process.argv[4]], { detached: true, stdio: ['ignore','ignore','inherit','ipc'], env: process.env });
  owner.on('message', m => process.send({ ...m, owner: owner.pid }));
}
async function main() {
  mkdirSync(base, { recursive: true });
  const { stdout: version } = await exec(binary, ['--version'], { timeout: 3000 });
  const { stdout: head } = await exec('git', ['-C', repo, 'rev-parse', 'HEAD'], { timeout: 3000 });
  const facts = { head: head.trim(), binary: resolve(binary), version: version.trim(), sha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), platform: process.platform, arch: process.arch };
  console.log(JSON.stringify(facts));
  for (const mode of ['SIGTERM', 'SIGKILL']) {
    const root = mkdtempSync(join(base, 'probe-profile-'));
    const witness = spawn(process.execPath, ['-e', 'setInterval(()=>{}, 1000)'], { stdio: 'ignore' });
    const witnessClosed = once(witness, 'close');
    const expired = spawn(process.execPath, ['-e', ''], { stdio: 'ignore' });
    await once(expired, 'close');
    expired.pid = witness.pid;
    assert.equal(expired.kill('SIGKILL'), false, 'an exited Child must not signal a reused diagnostic PID');
    const runnerChild = fork(__filename, ['runner', root, mode], { stdio: ['ignore','ignore','inherit','ipc'] });
    const runnerClosed = once(runnerChild, 'close');
    try {
      const [before] = await Promise.race([once(runnerChild, 'message'), pause(20000).then(()=>{throw new Error('runner did not start shell')})]);
      assert.ok(!before.error, before.error);
      runnerChild.kill('SIGKILL'); await runnerClosed;
      await bounds(() => existsSync(root + '.result.json'), 25000);
      const result = JSON.parse(readFileSync(root + '.result.json'));
      assert.equal(witness.exitCode, null); assert.equal(witness.signalCode, null);
      Object.assign(result, { before, witnessSurvived: true, reusedPidMetadataDidNotSignal: true, ...facts });
      writeFileSync(root + '.result.json', JSON.stringify(result, null, 2));
      console.log(JSON.stringify(result));
      assert.equal(result.ok, true);
      assert.equal(existsSync(root), false);
    } finally {
      if (runnerChild.exitCode === null && runnerChild.signalCode === null) { runnerChild.kill('SIGKILL'); await runnerClosed; }
      witness.kill('SIGKILL'); await witnessClosed;
    }
  }
}
(process.argv[2] === 'guard' ? guard() : process.argv[2] === 'runner' ? runner() : main()).catch(e => { console.error(e.message); if (process.send) process.send({error: e.message}); process.exitCode = 1; });
