import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(os.environ['I557_ROOT'])
OUT = Path(os.environ['I557_EVIDENCE'])
OUT.mkdir(parents=True, exist_ok=True)
label = sys.argv[1]
command = sys.argv[2:]
if any((OUT / (label + suffix)).exists() for suffix in ['.log', '.json', '.done']):
    raise SystemExit('Existing measurement label; choose a new label to preserve evidence')
load = os.getloadavg()
if load[1] >= 20:
    raise SystemExit('Five-minute load gate closed')
head = subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip()
patch = subprocess.check_output(['git', '-C', str(ROOT), 'diff', '--binary'])
for name in subprocess.check_output(['git', '-C', str(ROOT), 'ls-files', '--others', '--exclude-standard'], text=True).splitlines():
    if name.startswith('crates/'):
        patch += subprocess.run(['git', '-C', str(ROOT), 'diff', '--no-index', '--binary', '--', '/dev/null', name], stdout=subprocess.PIPE).stdout
env = {key: os.environ[key] for key in ['PATH', 'HOME', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_BUILD_JOBS', 'CARGO_TERM_COLOR']}
env['CARGO_TARGET_DIR'] = str(ROOT / 'target')
env['EZERBOT_GIT_COMMIT'] = head
env['TMPDIR'] = str(OUT / 'isolated')
Path(env['TMPDIR']).mkdir(exist_ok=True)
if label.startswith('trace'):
    env['I557_TRACE_DIR'] = str(OUT / (label + '-trace'))
meta = dict(label=label, command=command, cwd=str(ROOT), head=head,
            patch_sha256=hashlib.sha256(patch).hexdigest(), load_before=load,
            profile='cargo test dev/debug', started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
start = time.monotonic()
samples = []
with (OUT / (label + '.log')).open('wb') as output:
    proc = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
    meta['pid'] = proc.pid
    (OUT / (label + '.json')).write_text(json.dumps(meta, indent=2))
    while proc.poll() is None:
        samples.append(dict(seconds=time.monotonic()-start, load=os.getloadavg()))
        time.sleep(1)
    code = proc.returncode
meta.update(exit_code=code, seconds=time.monotonic()-start, load_after=os.getloadavg())
meta['load_samples'] = samples
artifacts = []
for line in (OUT / (label + '.log')).read_text(errors='replace').splitlines():
    try:
        row = json.loads(line)
    except ValueError:
        continue
    if row.get('reason') == 'compiler-artifact' and row.get('executable') and row['target']['name'] == 'chat_reliability':
        executable = Path(row['executable'])
        artifacts.append(dict(path=str(executable), sha256=hashlib.sha256(executable.read_bytes()).hexdigest(), profile=row.get('profile')))
meta['executables'] = artifacts
for arg in command:
    if arg in [str(OUT / 'baseline-executable'), str(OUT / 'instrumented-executable')]:
        executable = Path(arg)
        meta['executables'].append(dict(path=arg, sha256=hashlib.sha256(executable.read_bytes()).hexdigest(), profile='archived cargo test dev/debug'))
(OUT / (label + '.json')).write_text(json.dumps(meta, indent=2))
(OUT / (label + '.done')).write_text(str(code) + '\n')
print(json.dumps({k: v for k, v in meta.items() if k != 'load_samples'}))
raise SystemExit(code)
