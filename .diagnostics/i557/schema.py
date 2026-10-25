import json
import math
import re

SUBJECT = '6aca5529b049d04b3534c97414b329143fbf7a1e'
PATCH_SHA = 'a02eb73154db721a4a862412bd2294f0bd29cac7e9e019834bfe67582f30beb8'
TEST = 'a_long_history_still_shows_the_reply_growing_without_long_jumps'
LABELS = ['baseline-arm-6aca-exact', 'trace-arm-6aca-exact', 'control-arm-6aca-exact']
SOURCES = ['crates/domain/src/lib.rs', 'crates/domain/src/i557.rs', 'crates/engine/src/run_loop.rs', 'crates/engine/src/store.rs', 'crates/wrapper/src/features/engine.rs', 'crates/wrapper/src/gateway.rs', 'crates/wrapper/src/lib.rs', 'crates/wrapper/src/projection_hints.rs', 'crates/wrapper/src/provider/reliable.rs', 'crates/wrapper/tests/chat_reliability.rs']
PHASES = set('tx_enter begin_enter begin_return operation_return commit_enter tx_committed tx_error engine_input inserted reliable_begin request_link wire_send_enter wire_send_return wire_receive emit_enter emit_return gateway_received filter_input filter_return filter_refresh_enter filter_refresh_return filter_ready filtered_emit filtered_emit_return events_used projected hints_begin hints_loaded api_begin api_dispatch_return log_enter log_return api_return write_enter write_return flush_return fake_request words_begin word_enter word_return word_wake reply_return watch_begin poll_wake visible visible_stamp watch_end sleep_enter'.split())
ANALYSIS_KEYS = set('fixture run request words source_bytes commits polls visible gaps gap_ms first_break_demonstrated words_interval_ms emission_to_gateway_ms begin_ms commit_ms filter_refresh_ms wire_send_ms emit_ms api_dispatch_ms log_ms extraction_ms wake_lateness_ms committed_before_api_but_missing_polls worst_gap seq bytes inserted_ns commit_enter_ns committed_ns cumulative_bytes poll wake_ns api_begin_ns api_return_ns dispatcher_ms seq_used text_seq_used committed_seq_before_api durable_bytes_before_api commit_interval_overlaps_api_begin ms from_poll to_poll from_words to_words from_ns to_ns n min median max ns phase a b ok thread index'.split())
ERROR_TYPES = ['AssertionError', 'FileNotFoundError', 'CalledProcessError', 'JSONDecodeError', 'KeyError', 'OSError', 'ValueError', 'BlockingIOError', 'PermissionError', 'Other']
REASONS = set('pending completed duplicate_label load_gate_closed precondition_failed provenance_failed environment_failed spawn_failed collection_failed unexpected_preparation lock_failed entry_invalid gate_invalid entry_missing entry_incomplete gate_missing gate_incomplete metadata_missing pieces_missing child_nonzero wrapper_spawn_failed metadata_invalid measurement_receipt_invalid apply_check_failed provenance_mismatch machine_failed'.split())
REASONS.add('host_context_invalid')
REASONS.add('local_runtime_forbidden')
CONTEXTS = ['mac-local', 'github-actions', 'unverified-ci']

def fields(value, names):
    assert type(value) is dict and set(value) == set(names.split()), 'Object fields differ'

def integer(value):
    assert type(value) is int and 0 <= value < 2**128, 'Invalid unsigned integer'

def number(value):
    assert type(value) in (int, float) and math.isfinite(value) and 0 <= value < 2**128, 'Invalid number'

def boolean(value):
    assert type(value) is bool

def digest(value, length=64):
    assert type(value) is str and re.fullmatch('[0-9a-f]{' + str(length) + '}', value)

def return_code(value):
    assert type(value) is int and -128 <= value <= 255

def error_fields(value):
    assert value['error_category'] in [None, *ERROR_TYPES]
    if value['errno'] is not None:
        integer(value['errno'])
        assert value['errno'] <= 4095

def run_entry(value):
    names = 'schema_version label stage reason load_before wrapper_return_code child_return_code initial_return_code error_category errno head patch_sha256 started'
    assert value['schema_version'] in [1, 2] and value['label'] in LABELS
    if value['schema_version'] == 2:
        names += ' execution_context'
        assert value['execution_context'] in CONTEXTS
    fields(value, names)
    assert value['stage'] in ['entry', 'precondition', 'host', 'provenance', 'environment', 'spawn', 'running', 'collection', 'finished']
    assert value['reason'] in REASONS
    boolean(value['started'])
    error_fields(value)
    for key in ['wrapper_return_code', 'child_return_code', 'initial_return_code']:
        if value[key] is not None:
            return_code(value[key])
    if value['load_before'] is not None:
        assert type(value['load_before']) is list and len(value['load_before']) == 3
        for item in value['load_before']:
            number(item)
    if value['head'] is not None:
        digest(value['head'], 40)
    if value['patch_sha256'] is not None:
        digest(value['patch_sha256'])
    if value['reason'] == 'completed':
        assert value['stage'] == 'finished' and value['started']
        assert value['wrapper_return_code'] == value['child_return_code'] == value['initial_return_code']
        if value['schema_version'] == 2:
            assert value['execution_context'] != 'unverified-ci'
            if value['execution_context'] == 'mac-local':
                assert value['load_before'][1] < 20

def gate_entry(value):
    fields(value, 'schema_version stage reason wrapper_return_code child_return_code error_category errno started')
    assert value['schema_version'] == 1 and value['stage'] in ['entry', 'lock', 'spawn', 'running', 'finished']
    assert value['reason'] in REASONS
    boolean(value['started'])
    error_fields(value)
    for key in ['wrapper_return_code', 'child_return_code']:
        if value[key] is not None:
            return_code(value[key])
    if value['reason'] == 'completed':
        assert value['stage'] == 'finished' and value['started'] and value['wrapper_return_code'] == value['child_return_code']

def entry_attempt(value):
    fields(value, 'label wrapper_return_code reason error_category errno entry_present gate_present metadata_present log_present done_present runner gate')
    assert value['label'] in LABELS and value['reason'] in REASONS
    error_fields(value)
    if value['wrapper_return_code'] is not None:
        return_code(value['wrapper_return_code'])
    for key in ['entry_present', 'gate_present', 'metadata_present', 'log_present', 'done_present']:
        boolean(value[key])
    if value['runner'] is not None:
        run_entry(value['runner'])
        assert value['entry_present'] and value['runner']['label'] == value['label']
    if value['gate'] is not None:
        gate_entry(value['gate'])
        assert value['gate_present']
    if value['reason'] == 'completed':
        assert all(value[k] for k in ['entry_present', 'gate_present', 'metadata_present', 'log_present', 'done_present'])
        assert value['runner']['reason'] == value['gate']['reason'] == 'completed'
        assert value['wrapper_return_code'] == value['runner']['wrapper_return_code'] == value['gate']['wrapper_return_code']

def counts(value):
    fields(value, 'passed failed ignored measured filtered')
    for item in value.values():
        integer(item)
    assert value['passed'] + value['failed'] == 1
    assert value['ignored'] == value['measured'] == 0 and value['filtered'] == 23

def trace_file(data, name):
    assert re.fullmatch(r'thread-[1-9][0-9]*\.tsv', name)
    lines = data.decode('ascii').splitlines()
    thread = int(name[7:-4])
    assert lines[0] == f'sensor\ti557-tls-v1\tthread\t{thread}'
    assert lines[1] == 'ns\tfixture\trun\trequest\tpoll\tphase\ta\tb\tok'
    assert re.fullmatch(r'END\t[0-9]+\t[0-9]+', lines[-1])
    end = lines[-1].split('\t')
    assert int(end[1]) == len(lines)-3
    integer(int(end[2]))
    for line in lines[2:-1]:
        row = line.split('\t')
        assert len(row) == 9 and row[5] in PHASES and row[8] in ['0', '1']
        for index in [0, 1, 2, 3, 4, 6, 7]:
            assert re.fullmatch(r'[0-9]+', row[index])
            integer(int(row[index]))
    return len(lines)-3

def numeric_tree(value, key=None):
    if type(value) is dict:
        assert set(value) <= ANALYSIS_KEYS
        for name, child in value.items():
            numeric_tree(child, name)
    elif type(value) is list:
        for child in value:
            numeric_tree(child, key)
    elif type(value) is str:
        assert key == 'phase' and value in PHASES
    elif value is not None and type(value) is not bool:
        number(value)

def analysis_fixture(value):
    fields(value, 'fixture run request words source_bytes commits polls visible gaps gap_ms first_break_demonstrated words_interval_ms emission_to_gateway_ms begin_ms commit_ms filter_refresh_ms wire_send_ms emit_ms api_dispatch_ms log_ms extraction_ms wake_lateness_ms committed_before_api_but_missing_polls worst_gap')
    numeric_tree(value)
    assert value['words'] == 40 and value['source_bytes'] == 230
    assert value['first_break_demonstrated'] is None
    assert len(value['commits']) > 0 and sum(c['bytes'] for c in value['commits']) == 230
    for item in value['commits']:
        fields(item, 'seq bytes inserted_ns commit_enter_ns committed_ns cumulative_bytes')
        for child in item.values():
            integer(child)
        assert item['inserted_ns'] <= item['commit_enter_ns'] <= item['committed_ns']
    assert type(value['polls']) is list and len(value['polls']) > 0
    for item in value['polls']:
        fields(item, 'poll wake_ns api_begin_ns api_return_ns dispatcher_ms log_ms extraction_ms bytes words seq_used text_seq_used committed_seq_before_api durable_bytes_before_api commit_interval_overlaps_api_begin wake_lateness_ms')
        assert item['wake_ns'] <= item['api_begin_ns'] <= item['api_return_ns']
        assert type(item['commit_interval_overlaps_api_begin']) is list
    assert [p['poll'] for p in value['polls']] == list(range(1, len(value['polls'])+1))
    assert value['polls'][-1]['words'] == 40 and value['polls'][-1]['bytes'] == 230
    assert type(value['gaps']) is list and len(value['gaps']) == value['visible']-1 and value['visible'] >= 1
    assert (value['worst_gap'] is None) == (value['visible'] == 1)
    for item in [*value['gaps'], *([value['worst_gap']] if value['worst_gap'] is not None else [])]:
        fields(item, 'ms from_poll to_poll from_words to_words from_ns to_ns')
        assert item['from_poll'] < item['to_poll'] and item['from_ns'] < item['to_ns']
    for key in ['gap_ms', 'words_interval_ms', 'emission_to_gateway_ms', 'begin_ms', 'commit_ms', 'filter_refresh_ms', 'wire_send_ms', 'emit_ms', 'api_dispatch_ms', 'log_ms', 'extraction_ms', 'wake_lateness_ms']:
        fields(value[key], 'n min median max')
        integer(value[key]['n'])
        assert value[key]['min'] <= value[key]['median'] <= value[key]['max']
    assert type(value['committed_before_api_but_missing_polls']) is list

def machine(value):
    fields(value, 'architecture os_version image_os image_version logical_cpus memory_bytes rustc_version cargo_version build_jobs test_thread_policy')
    assert value['architecture'] == 'arm64'
    assert re.fullmatch(r'[0-9]+(?:\.[0-9]+){1,3}', value['os_version'])
    assert re.fullmatch(r'macos[0-9]+', value['image_os'])
    assert re.fullmatch(r'[0-9]+(?:\.[0-9]+){1,4}', value['image_version'])
    assert value['rustc_version'] == '1.99.1'
    assert re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', value['cargo_version'])
    assert value['build_jobs'] == 3 and value['test_thread_policy'] == 'default'
    integer(value['logical_cpus'])
    integer(value['memory_bytes'])
    assert value['logical_cpus'] > 0 and value['memory_bytes'] > 0

def configuration(value):
    names = 'label return_code classification tests visible_count words gaps_ms test_seconds wall_seconds load_before load_after load_samples binary_sha256 source_diff_sha256 sources_sha256 binary_architecture binary_file cargo_profile'
    if 'execution_context' in value:
        names += ' execution_context'
        assert value['execution_context'] == 'github-actions'
    fields(value, names)
    assert value['label'] in LABELS
    assert type(value['return_code']) is int
    assert value['classification'] in ['passed', 'fluidity_failure']
    assert value['return_code'] in [0, 101]
    assert (value['return_code'] == 0) == (value['classification'] == 'passed')
    counts(value['tests'])
    assert value['tests']['failed'] == int(value['return_code'] != 0)
    integer(value['visible_count'])
    assert type(value['words']) is list and len(value['words']) == value['visible_count']
    assert type(value['gaps_ms']) is list and len(value['gaps_ms']) == value['visible_count']-1
    for item in value['words'] + value['gaps_ms']:
        integer(item)
    assert value['words'][-1] == 40 and all(a < b for a, b in zip(value['words'], value['words'][1:]))
    violates_fluidity = value['visible_count'] < 15 or max(value['gaps_ms'], default=0) > 300
    assert violates_fluidity == (value['return_code'] != 0)
    for key in ['test_seconds', 'wall_seconds']:
        number(value[key])
    for key in ['load_before', 'load_after']:
        assert type(value[key]) is list and len(value[key]) == 3
        for item in value[key]:
            number(item)
    if 'execution_context' not in value:
        assert value['load_before'][1] < 20
    for sample in value['load_samples']:
        fields(sample, 'seconds load')
        number(sample['seconds'])
        assert type(sample['load']) is list and len(sample['load']) == 3
        for item in sample['load']:
            number(item)
    digest(value['binary_sha256'])
    digest(value['source_diff_sha256'])
    expected = set(SOURCES) if value['label'] == LABELS[1] else set(SOURCES)-{'crates/domain/src/i557.rs'}
    assert type(value['sources_sha256']) is dict and set(value['sources_sha256']) == expected
    for item in value['sources_sha256'].values():
        digest(item)
    assert value['binary_architecture'] == 'arm64'
    assert re.fullmatch(r'chat_reliability-[0-9a-f]{16}|baseline-executable', value['binary_file'])
    fields(value['cargo_profile'], 'opt_level debug_info debug_assertions overflow_checks test')
    assert value['cargo_profile'] == dict(opt_level=0, debug_info='line-tables-only', debug_assertions=True, overflow_checks=True, test=True)

def receipt(value):
    names = 'schema_version workflow_sha subject_sha patch_sha256 run_id attempt machine configurations source_restored measurement_valid product_failed preparation_failed stage preparation_return_code preparation_error_type'
    assert value['schema_version'] in [1, 2, 3]
    if value['schema_version'] >= 2:
        names += ' entry_load apply_check entry_receipts preparation_reason preparation_errno machine_gate'
    if value['schema_version'] == 3:
        names += ' execution_context'
        assert value['execution_context'] in ['unverified-ci', 'github-actions']
    fields(value, names)
    assert value['subject_sha'] == SUBJECT and value['patch_sha256'] == PATCH_SHA
    digest(value['workflow_sha'], 40)
    integer(value['run_id'])
    integer(value['attempt'])
    assert value['run_id'] > 0 and value['attempt'] == 1
    if value['machine'] is not None:
        machine(value['machine'])
    assert type(value['configurations']) is list and len(value['configurations']) <= 3
    for index, item in enumerate(value['configurations']):
        configuration(item)
        assert item['label'] == LABELS[index]
        if value['schema_version'] < 3:
            assert 'execution_context' not in item
    for key in ['source_restored', 'measurement_valid', 'product_failed', 'preparation_failed']:
        boolean(value[key])
    assert value['stage'] in ['prepare', 'host', 'provenance', 'apply_check', 'machine', 'baseline', 'apply', 'trace', 'analysis', 'restore', 'control', 'complete']
    if value['preparation_return_code'] is not None:
        return_code(value['preparation_return_code'])
    assert value['preparation_error_type'] in [None, *ERROR_TYPES]
    if value['schema_version'] >= 2:
        assert value['preparation_reason'] in REASONS
        if value['machine_gate'] is not None:
            gate_entry(value['machine_gate'])
        if value['preparation_errno'] is not None:
            integer(value['preparation_errno'])
            assert value['preparation_errno'] <= 4095
        if value['entry_load'] is not None:
            assert type(value['entry_load']) is list and len(value['entry_load']) == 3
            for item in value['entry_load']:
                number(item)
        check = value['apply_check']
        fields(check, 'status return_code subject_sha patch_sha256')
        assert check['status'] in ['not_run', 'passed', 'failed']
        if check['return_code'] is not None:
            return_code(check['return_code'])
        if check['subject_sha'] is not None:
            digest(check['subject_sha'], 40)
        if check['patch_sha256'] is not None:
            digest(check['patch_sha256'])
        if check['status'] == 'passed':
            assert check['return_code'] == 0 and check['subject_sha'] == SUBJECT and check['patch_sha256'] == PATCH_SHA
        assert type(value['entry_receipts']) is list and len(value['entry_receipts']) <= 3
        for index, entry in enumerate(value['entry_receipts']):
            entry_attempt(entry)
            assert entry['label'] == LABELS[index]
        assert len(value['entry_receipts']) >= len(value['configurations'])
        for configuration_value, entry in zip(value['configurations'], value['entry_receipts']):
            assert entry['reason'] == 'completed'
            assert entry['wrapper_return_code'] == configuration_value['return_code']
            assert entry['runner']['head'] == SUBJECT
            assert entry['runner']['patch_sha256'] == configuration_value['source_diff_sha256']
            assert entry['runner']['load_before'] == configuration_value['load_before']
            if value['schema_version'] == 3:
                assert value['execution_context'] == configuration_value['execution_context'] == entry['runner']['execution_context'] == 'github-actions'
    assert value['product_failed'] == any(c['return_code'] != 0 for c in value['configurations'])
    if value['measurement_valid']:
        assert not value['preparation_failed'] and value['source_restored'] and len(value['configurations']) == 3 and value['stage'] == 'complete'
        assert value['preparation_error_type'] is value['preparation_return_code'] is None
        if value['schema_version'] >= 2:
            assert value['preparation_reason'] == 'completed' and value['apply_check']['status'] == 'passed'
            assert value['machine_gate']['reason'] == 'completed' and value['machine_gate']['wrapper_return_code'] == 0
            assert len(value['entry_receipts']) == 3 and all(e['reason'] == 'completed' for e in value['entry_receipts'])
            if value['schema_version'] == 2:
                assert value['entry_load'][1] < 20

def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + '\n').encode('ascii')
