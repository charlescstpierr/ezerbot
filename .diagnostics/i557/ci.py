import ast
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

import schema
import host
from observation import PreparationFailure, category, empty_attempt, inspect_attempt

TOOLS = Path(__file__).resolve().parent
ROOT = Path(os.environ['I557_ROOT']).resolve()
OUT = Path(os.environ['I557_EVIDENCE']).resolve()
ENV = {}

def gate(seconds, label, command):
    return [sys.executable, str(TOOLS / 'gate.py'), str(OUT / 'cargo.lock'), seconds, str(OUT / (label + '-gate.json')), *command]

def checkpoint(state):
    state['product_failed'] = any(c['return_code'] != 0 for c in state['configurations'])
    schema.receipt(state)
    (OUT / 'state.json').write_bytes(schema.encoded(state))

def private(command, name, env=ENV, cwd=ROOT):
    with (OUT / (name + '.private')).open('wb') as log:
        result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT)
    return result.returncode

def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args])

def source_hashes():
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in schema.SOURCES if (ROOT / name).exists()}

def machine(state):
    def output(command, name):
        code = private(command, name)
        if name == 'cargo-version':
            path = OUT / 'cargo-version-gate.json'
            if not path.is_file() or path.is_symlink():
                state['preparation_return_code'] = code
                raise PreparationFailure('gate_missing')
            try:
                data = json.loads(path.read_text())
                schema.gate_entry(data)
            except (ValueError, OSError, AssertionError, TypeError):
                state['preparation_return_code'] = code
                raise PreparationFailure('gate_invalid') from None
            state['machine_gate'] = data
            checkpoint(state)
            if data['wrapper_return_code'] != code:
                state['preparation_return_code'] = code
                raise PreparationFailure('gate_incomplete')
        if code != 0:
            state['preparation_return_code'] = code
            if name == 'cargo-version' and data['reason'] != 'completed':
                state['preparation_error_type'] = data['error_category']
                state['preparation_errno'] = data['errno']
                raise PreparationFailure(data['reason'])
            raise PreparationFailure('machine_failed')
        return (OUT / (name + '.private')).read_text().strip()
    rustc_rows = re.findall(r'^rustc ([0-9]+\.[0-9]+\.[0-9]+) \([0-9a-f]+ [0-9-]+\)$', output(['rustc', '--version'], 'rustc-version'), re.MULTILINE)
    cargo = re.fullmatch(r'cargo ([0-9]+\.[0-9]+\.[0-9]+) \([0-9a-f]+ [0-9-]+\)', output(gate('120', 'cargo-version', ['cargo', '--version']), 'cargo-version'))
    assert len(rustc_rows) == 1 and cargo
    result = dict(architecture=platform.machine(), os_version=platform.mac_ver()[0],
                  image_os=os.environ['ImageOS'], image_version=os.environ['ImageVersion'],
                  logical_cpus=os.cpu_count(), memory_bytes=int(output(['sysctl', '-n', 'hw.memsize'], 'memory')),
                  rustc_version=rustc_rows[0], cargo_version=cargo[1], build_jobs=3, test_thread_policy='default')
    schema.machine(result)
    return result

def measure(state, label, binary=None, cargo_profile=None):
    command = gate('1800', label, ['cargo', 'test', '-p', 'grokbot-wrapper', '--test', 'chat_reliability', '--message-format=json', schema.TEST, '--', '--exact', '--nocapture'])
    if binary is not None:
        command = gate('120', label, [str(binary), schema.TEST, '--exact', '--nocapture'])
    entry = empty_attempt(label)
    state['entry_receipts'].append(entry)
    checkpoint(state)
    try:
        status = private([sys.executable, str(TOOLS / 'run-ci.py'), label, *command], label + '-runner')
        entry['wrapper_return_code'] = status
    except OSError as error:
        entry['reason'] = 'wrapper_spawn_failed'
        entry['error_category'], entry['errno'] = category(error)
        checkpoint(state)
        raise PreparationFailure(entry['reason']) from None
    inspect_attempt(OUT, entry)
    checkpoint(state)
    if entry['reason'] != 'completed':
        raise PreparationFailure(entry['reason'])
    try:
        raw = json.loads((OUT / (label + '.json')).read_text())
    except (OSError, ValueError):
        entry['reason'] = 'metadata_invalid'
        raise PreparationFailure(entry['reason']) from None
    assert raw['head'] == schema.SUBJECT and status == raw['exit_code']
    assert entry['runner']['head'] == raw['head'] and entry['runner']['patch_sha256'] == raw['patch_sha256']
    assert entry['runner']['load_before'] == raw['load_before']
    assert entry['runner']['execution_context'] == raw['execution_context'] == 'github-actions'
    text = (OUT / (label + '.log')).read_text()
    summaries = re.findall(r'^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored; ([0-9]+) measured; ([0-9]+) filtered out; finished in ([0-9.]+)s$', text, re.MULTILINE)
    assert len(summaries) == 1
    summary = summaries[0]
    tests = dict(zip(['passed', 'failed', 'ignored', 'measured', 'filtered'], map(int, summary[1:6])))
    schema.counts(tests)
    assert re.search(r'^test ' + schema.TEST + r' \.\.\. (ok|FAILED)$', text, re.MULTILINE)
    assert status in [0, 101] and tests['failed'] == int(status != 0)
    fluid = re.findall(r'^fluidité : ([0-9]+) textes visibles, mots (\[[0-9, ]+\]), écarts ms (\[[0-9, ]*\])$', text, re.MULTILINE)
    assert len(fluid) == 1
    visible, words, gaps = int(fluid[0][0]), ast.literal_eval(fluid[0][1]), ast.literal_eval(fluid[0][2])
    if status:
        gap_fail = re.search(r'^saut de ([0-9]+) ms entre deux textes visibles$', text, re.MULTILINE)
        count_fail = re.search(r'^seulement ([0-9]+) textes visibles : \[[0-9, ]+\]$', text, re.MULTILINE)
        assert (gap_fail is not None and int(gap_fail[1]) > 300) or (count_fail is not None and int(count_fail[1]) < 15)
    assert len(raw['executables']) == 1
    executable = raw['executables'][0]
    path = Path(executable['path'])
    assert path.is_file() and not path.is_symlink()
    assert hashlib.sha256(path.read_bytes()).hexdigest() == executable['sha256']
    if binary is None:
        profile = executable['profile']
        assert profile['opt_level'] == '0' and profile['test'] and profile['debug_assertions'] and profile['overflow_checks']
        cargo_profile = dict(opt_level=int(profile['opt_level']), debug_info=profile['debuginfo'],
                             debug_assertions=profile['debug_assertions'], overflow_checks=profile['overflow_checks'], test=profile['test'])
    assert private(['file', str(path)], label + '-binary-arch') == 0
    assert re.search(r': Mach-O 64-bit executable arm64\s*$', (OUT / (label + '-binary-arch.private')).read_text())
    result = dict(label=label, execution_context='github-actions', return_code=status, classification='passed' if status == 0 else 'fluidity_failure',
                  tests=tests, visible_count=visible, words=words, gaps_ms=gaps, test_seconds=float(summary[6]),
                  wall_seconds=raw['seconds'], load_before=list(raw['load_before']), load_after=list(raw['load_after']),
                  load_samples=raw['load_samples'], binary_sha256=executable['sha256'],
                  source_diff_sha256=raw['patch_sha256'], sources_sha256=source_hashes(), binary_architecture='arm64',
                  binary_file=path.name, cargo_profile=cargo_profile)
    schema.configuration(result)
    return result, path

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    state = dict(schema_version=3, execution_context='unverified-ci', workflow_sha=os.environ['I557_WORKFLOW_SHA'], subject_sha=schema.SUBJECT,
                 patch_sha256=schema.PATCH_SHA, run_id=int(os.environ['I557_RUN_ID']), attempt=int(os.environ['I557_ATTEMPT']),
                 machine=None, configurations=[], source_restored=False, measurement_valid=False, product_failed=False,
                 preparation_failed=False, stage='prepare', preparation_return_code=None, preparation_error_type=None,
                 preparation_reason='pending', preparation_errno=None, machine_gate=None, entry_load=None, entry_receipts=[],
                 apply_check=dict(status='not_run', return_code=None, subject_sha=None, patch_sha256=None))
    checkpoint(state)
    originals = {}
    applied = False
    try:
        state['entry_load'] = list(os.getloadavg())
        checkpoint(state)
        fixture_home = OUT / 'fixture-home'
        fixture_home.mkdir(exist_ok=True)
        ENV.update(PATH=os.environ['PATH'], HOME=str(fixture_home),
                   CARGO_HOME=os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')),
                   RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
                   CARGO_BUILD_JOBS='3', CARGO_TERM_COLOR='never', I557_ROOT=str(ROOT), I557_EVIDENCE=str(OUT))
        ENV.update({key: os.environ[key] for key in host.CI_KEYS if key in os.environ})
        state['stage'] = 'host'
        checkpoint(state)
        state['execution_context'] = host.github_actions(ROOT, OUT, os.environ, TOOLS)
        state['stage'] = 'provenance'
        checkpoint(state)
        assert os.environ['I557_ATTEMPT'] == '1'
        assert git('rev-parse', 'HEAD').decode().strip() == schema.SUBJECT
        assert not git('status', '--porcelain')
        assert subprocess.check_output(['git', '-C', str(TOOLS.parent.parent), 'rev-parse', 'HEAD']).decode().strip() == state['workflow_sha']
        patch = TOOLS / 'instrumentation-v2.patch'
        assert hashlib.sha256(patch.read_bytes()).hexdigest() == schema.PATCH_SHA
        originals = {name: (ROOT / name).read_bytes() for name in schema.SOURCES if (ROOT / name).exists()}
        state['stage'] = 'apply_check'
        state['apply_check'].update(subject_sha=git('rev-parse', 'HEAD').decode().strip(), patch_sha256=hashlib.sha256(patch.read_bytes()).hexdigest())
        checkpoint(state)
        code = private(['git', 'apply', '--check', str(patch)], 'apply-check')
        state['apply_check'].update(status='passed' if code == 0 else 'failed', return_code=code)
        checkpoint(state)
        if code != 0:
            state['preparation_return_code'] = code
            raise PreparationFailure('apply_check_failed')
        state['stage'] = 'machine'
        checkpoint(state)
        state['machine'] = machine(state)
        state['stage'] = 'baseline'
        result, executable = measure(state, schema.LABELS[0])
        assert result['source_diff_sha256'] == hashlib.sha256(b'').hexdigest()
        state['configurations'].append(result)
        baseline = OUT / 'baseline-executable'
        shutil.copyfile(executable, baseline)
        baseline.chmod(0o700)
        assert hashlib.sha256(baseline.read_bytes()).hexdigest() == result['binary_sha256']
        state['stage'] = 'apply'
        assert private(['git', 'apply', str(patch)], 'apply') == 0
        applied = True
        assert set(git('diff', '--name-only').decode().splitlines()) == set(schema.SOURCES)-{'crates/domain/src/i557.rs'}
        assert git('ls-files', '--others', '--exclude-standard').decode().splitlines() == ['crates/domain/src/i557.rs']
        state['stage'] = 'trace'
        result, _ = measure(state, schema.LABELS[1])
        assert result['source_diff_sha256'] != hashlib.sha256(b'').hexdigest()
        state['configurations'].append(result)
        state['stage'] = 'analysis'
        assert private([sys.executable, str(TOOLS / 'analyze.py'), schema.LABELS[1]], 'analysis') == 0
        state['stage'] = 'restore'
        for name, data in originals.items():
            (ROOT / name).write_bytes(data)
        (ROOT / 'crates/domain/src/i557.rs').unlink()
        applied = False
        assert not git('status', '--porcelain')
        state['source_restored'] = True
        state['stage'] = 'control'
        result, _ = measure(state, schema.LABELS[2], baseline, state['configurations'][0]['cargo_profile'])
        state['configurations'].append(result)
        assert result['binary_sha256'] == state['configurations'][0]['binary_sha256']
        assert result['sources_sha256'] == state['configurations'][0]['sources_sha256']
        state.update(stage='complete', measurement_valid=True, preparation_reason='completed')
    except Exception as error:
        state['preparation_failed'] = True
        state['preparation_reason'] = error.reason if isinstance(error, PreparationFailure) else 'unexpected_preparation'
        if not isinstance(error, PreparationFailure) and state['stage'] in ['host', 'provenance', 'machine']:
            state['preparation_reason'] = {'host': 'host_context_invalid', 'provenance': 'provenance_mismatch', 'machine': 'machine_failed'}[state['stage']]
        if not isinstance(error, PreparationFailure):
            state['preparation_error_type'], state['preparation_errno'] = category(error)
        if state['entry_receipts']:
            state['preparation_return_code'] = state['entry_receipts'][-1]['wrapper_return_code']
    finally:
        if applied:
            for name, data in originals.items():
                (ROOT / name).write_bytes(data)
            sensor = ROOT / 'crates/domain/src/i557.rs'
            if sensor.exists():
                sensor.unlink()
        state['source_restored'] = not bool(git('status', '--porcelain'))
        checkpoint(state)
    if not state['source_restored']:
        raise SystemExit('I557 restoration_failed')
    assert private([sys.executable, str(TOOLS / 'export.py'), str(OUT), os.environ['I557_GITLEAKS']], 'export', env=ENV) == 0, 'I557 export_failed'
    print('I557 safe_export stage=' + state['stage'] + ' product_failed=' + str(int(state['product_failed'])) + ' preparation_failed=' + str(int(state['preparation_failed'])))

if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('I557 collection_or_export_failed')
        raise SystemExit(1)
