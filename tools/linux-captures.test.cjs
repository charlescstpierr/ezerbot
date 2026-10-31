const { test } = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync, spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { createHash } = require("node:crypto");
const { VIEWS, VIEWPORT, waitFor, pngInfo } = require("./linux-captures.cjs");

function repository(t) {
  const root = fs.mkdtempSync(join(tmpdir(), "linux-captures-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const repo = join(root, "repo");
  const bin = join(root, "bin");
  fs.mkdirSync(repo);
  fs.mkdirSync(bin);
  const git = (...args) => execFileSync("git", ["-C", repo, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "-q");
  git("config", "user.name", "Capture Test");
  git("config", "user.email", "capture@example.invalid");
  for (const path of ["tools", "app/ui", "app/web"]) fs.mkdirSync(join(repo, path), { recursive: true });
  fs.copyFileSync(join(__dirname, "linux-captures.sh"), join(repo, "tools/linux-captures.sh"));
  for (const path of ["app/ui/wrapper-chrome-fixture.cjs", "app/ui/local-api-client.cjs", "app/web/package-lock.json"]) fs.writeFileSync(join(repo, path), "{}");
  fs.writeFileSync(join(repo, ".gitignore"), "target/\nartifacts/\n");
  const log = join(root, "commands.jsonl");
  const diskLog = join(root, "disk-checks.jsonl");
  const control = join(root, "control");
  fs.writeFileSync(control, "success");
  const executable = (name, code) => fs.writeFileSync(join(bin, name), `#!${process.execPath}\n${code}`, { mode: 0o700 });
  executable("df", `const fs=require('node:fs'); fs.appendFileSync(${JSON.stringify(diskLog)},JSON.stringify(process.argv.slice(2))+'\\n'); const mode=fs.readFileSync(${JSON.stringify(control)},'utf8'); const path=process.argv.at(-1); const low=mode==='disk'||(mode==='disk-tmp'&&path.startsWith('/tmp/ezcap.'))||(mode==='disk-output'&&path.endsWith('/artifacts/captures-linux')); console.log('Filesystem 1B-blocks Used Available Use% Mounted'); console.log('test 200000000000 0 '+(process.argv.includes('-BG')?'100G':low?'26843545599':'100000000000')+' 0% /workspace');`);
  executable("npm", `const fs=require('node:fs'),cp=require('node:child_process'); fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({tool:'npm',args:process.argv.slice(2),cwd:process.cwd(),sha:cp.execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim()})+'\\n'); const mode=fs.readFileSync(${JSON.stringify(control)},'utf8'); if(mode==='web') process.exit(6); if(mode==='hold'){fs.writeFileSync(${JSON.stringify(join(root, "held-pid"))},String(process.pid));setInterval(()=>{},1000);}`);
  executable("cargo", `const fs=require('node:fs'),p=require('node:path'); fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({tool:'cargo',cwd:process.cwd(),sha:process.env.EZERBOT_GIT_COMMIT,unix:process.env.EZERBOT_GIT_COMMIT_UNIX,jobs:process.env.CARGO_BUILD_JOBS})+'\\n'); if(fs.readFileSync(${JSON.stringify(control)},'utf8')==='cargo') process.exit(7); fs.mkdirSync(p.join(process.env.CARGO_TARGET_DIR,'debug'),{recursive:true}); fs.writeFileSync(p.join(process.env.CARGO_TARGET_DIR,'debug/grokbot-wrapper'),'built '+process.env.EZERBOT_GIT_COMMIT,{mode:0o700});`);
  executable("google-chrome", "process.exit(0);");
  fs.writeFileSync(join(bin, "xvfb-run"), '#!/bin/bash\nshift\nshift\nshift\nexec "$@"\n', { mode: 0o700 });
  fs.writeFileSync(join(repo, "tools/linux-captures.cjs"), `const fs=require('node:fs'),cp=require('node:child_process'),p=require('node:path');
    const [sha,binary,output]=process.argv.slice(2); const mode=fs.readFileSync(${JSON.stringify(control)},'utf8');
    if(mode==='readiness'){ fs.writeFileSync(p.join(output,'partial.png'),'partial'); throw Error('readiness timed out'); }
    const manifest={sha, marker:fs.readFileSync(p.join(__dirname,'../marker'),'utf8'), binary:fs.readFileSync(binary,'utf8'), head:cp.execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(), detached:cp.spawnSync('git',['symbolic-ref','-q','HEAD']).status!==0, home:process.env.HOME, secret:process.env.OPENAI_API_KEY, ambientDb:process.env.GROKBOT_DB};
    fs.writeFileSync(p.join(output,'manifest.json'),JSON.stringify(manifest));`);
  const commit = marker => {
    fs.writeFileSync(join(repo, "marker"), marker);
    git("add", ".");
    git("commit", "-qm", marker);
    return git("rev-parse", "HEAD");
  };
  const first = commit("first");
  const second = commit("second");
  fs.writeFileSync(join(repo, "marker"), "dirty caller");
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, OPENAI_API_KEY: "synthetic-secret-sentinel", GROKBOT_DB: "synthetic-personal-profile", EZERBOT_GIT_COMMIT: "incorrect inherited commit", EZERBOT_GIT_COMMIT_UNIX: "1", CARGO_TARGET_DIR: join(root, "foreign-target"), CARGO_BUILD_JOBS: "99" };
  const run = sha => spawnSync("bash", [join(repo, "tools/linux-captures.sh"), sha], { cwd: repo, env, encoding: "utf8", timeout: 20000 });
  const clean = () => {
    assert.equal(git("worktree", "list", "--porcelain").split("worktree ").length - 1, 1);
    assert.deepEqual(fs.readdirSync(join(repo, "target/linux-captures")).filter(name => name.startsWith("run.")), []);
    assert.deepEqual(fs.readdirSync(join(repo, "artifacts/captures-linux")).filter(name => name.startsWith(".staging-") || name.startsWith(".previous-")), []);
  };
  return { root, repo, git, first, second, control, log, diskLog, env, run, clean };
}

test("capture controller builds only the detached SHA and filters the runtime environment", t => {
  const fixture = repository(t);
  const before = fixture.git("status", "--porcelain");
  const branch = fixture.git("symbolic-ref", "HEAD");
  const result = fixture.run(fixture.first.slice(0, 12));
  assert.equal(result.status, 0, result.stderr);
  const manifest = JSON.parse(fs.readFileSync(join(fixture.repo, "artifacts/captures-linux", fixture.first, "manifest.json")));
  assert.equal(manifest.sha, fixture.first);
  assert.equal(manifest.head, fixture.first);
  assert.equal(manifest.marker, "first");
  assert.equal(manifest.detached, true);
  assert.equal(manifest.binary, `built ${fixture.first}`);
  assert.equal(manifest.secret, undefined);
  assert.equal(manifest.ambientDb, undefined);
  assert.match(manifest.home, /^\/tmp\/ezcap\.[^/]+\/home$/);
  assert.equal(fs.existsSync(manifest.home), false);
  const diskChecks = fs.readFileSync(fixture.diskLog, "utf8").trim().split("\n").map(JSON.parse).filter(args => args.includes("-B1"));
  assert.deepEqual([...new Set(diskChecks.map(args => args.at(-1)))].sort(), [
    join(fixture.repo, "target/linux-captures"),
    join(manifest.home, ".."),
    join(fixture.repo, "artifacts/captures-linux"),
  ].sort());
  const commands = fs.readFileSync(fixture.log, "utf8").trim().split("\n").map(JSON.parse);
  assert.equal(commands.length, 3);
  assert.ok(commands.every(command => command.sha === fixture.first && command.cwd.includes("/source")));
  assert.equal(commands[2].jobs, "3");
  assert.equal(commands[2].unix, fixture.git("show", "-s", "--format=%ct", fixture.first));
  assert.equal(fixture.git("status", "--porcelain"), before);
  assert.equal(fixture.git("symbolic-ref", "HEAD"), branch);
  fixture.clean();
});

test("failure preserves a complete capture and removes staged output and worktrees", t => {
  const fixture = repository(t);
  assert.equal(fixture.run(fixture.first).status, 0);
  const path = join(fixture.repo, "artifacts/captures-linux", fixture.first, "manifest.json");
  const previous = fs.readFileSync(path);
  for (const failure of ["web", "cargo", "readiness", "disk", "disk-tmp", "disk-output"]) {
    fs.writeFileSync(fixture.control, failure);
    const result = fixture.run(fixture.first);
    assert.notEqual(result.status, 0, failure);
    assert.deepEqual(fs.readFileSync(path), previous, failure);
    fixture.clean();
  }
});

test("invalid SHA and unsupported commits fail without using caller assets", t => {
  const fixture = repository(t);
  for (const input of ["HEAD", "--help", "123", "abc;touch sentinel"]) assert.notEqual(fixture.run(input).status, 0);
  fixture.git("rm", "tools/linux-captures.cjs");
  fixture.git("commit", "-qm", "unsupported");
  const result = fixture.run(fixture.git("rev-parse", "HEAD"));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Unsupported commit.*missing tools\/linux-captures.cjs/);
  assert.equal(fs.existsSync(fixture.log), false);
  fixture.clean();
});

test("SIGTERM stops only the owned build group and removes the worktree", { timeout: 20000 }, async t => {
  const fixture = repository(t);
  fs.writeFileSync(fixture.control, "hold");
  const child = spawn("bash", [join(fixture.repo, "tools/linux-captures.sh"), fixture.first], { cwd: fixture.repo, env: fixture.env, stdio: "ignore" });
  t.after(() => { if (child.exitCode === null) child.kill("SIGKILL"); });
  const exited = new Promise(resolveExit => child.once("exit", (code, signal) => resolveExit({ code, signal })));
  const pidPath = join(fixture.root, "held-pid");
  await waitFor(() => fs.existsSync(pidPath), "owned build startup", undefined, 10000);
  const pid = Number(fs.readFileSync(pidPath, "utf8"));
  child.kill("SIGTERM");
  assert.equal((await exited).code, 143);
  assert.throws(() => process.kill(pid, 0), { code: "ESRCH" });
  fixture.clean();
});

test("view registry matches the documented set and PNG metadata hashes actual bytes", () => {
  const docs = fs.readFileSync(join(__dirname, "../docs/captures-linux.md"), "utf8");
  const documented = [...docs.matchAll(/^\| `([^`]+)` \|/gm)].map(match => match[1]);
  assert.deepEqual(VIEWS.map(view => view.id), documented);
  const bytes = Buffer.alloc(33);
  Buffer.from("89504e470d0a1a0a", "hex").copy(bytes);
  bytes.write("IHDR", 12);
  bytes.writeUInt32BE(VIEWPORT.width, 16);
  bytes.writeUInt32BE(VIEWPORT.height, 20);
  assert.deepEqual(pngInfo(bytes), { width: 1440, height: 900, sha256: createHash("sha256").update(bytes).digest("hex") });
  assert.throws(() => pngInfo(Buffer.from("not a PNG")), /not a PNG/);
  bytes.writeUInt32BE(1, 16);
  assert.throws(() => pngInfo(bytes), /dimensions/);
});

test("readiness observes delayed success and bounds false or stalled predicates", async () => {
  let calls = 0;
  await waitFor(() => ++calls === 3, "delayed state", undefined, 1000);
  assert.equal(calls, 3);
  await assert.rejects(waitFor(() => false, "missing state", undefined, 10), /missing state timed out/);
  await assert.rejects(waitFor(() => new Promise(() => {}), "stalled state", undefined, 10), /stalled state timed out/);
  const controller = new AbortController();
  const waiting = waitFor(() => new Promise(() => {}), "aborted state", controller.signal);
  controller.abort();
  await assert.rejects(waiting, /interrupted/);
});
