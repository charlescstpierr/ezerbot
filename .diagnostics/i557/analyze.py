import collections
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import sys
from schema import trace_file

OUT = Path(os.environ['I557_EVIDENCE'])
label = sys.argv[1]
log = (OUT / (label + '.log')).read_text(errors='replace')
receipt_log = re.sub(r'test [a-zA-Z0-9_]+ \.\.\. ok\n', '', log)
receipt_log = receipt_log.replace('sensor=i557-tls-v1ok thread=', 'sensor=i557-tls-v1 thread=')
receipt_rows = re.findall(r'I557_FLUSH sensor=i557-tls-v1 thread=(\d+) rows=(\d+) ok=(true|false)', receipt_log)
receipts = {int(t): (int(n), ok == 'true') for t, n, ok in receipt_rows}
assert len(receipts) == len(receipt_rows), 'Duplicate flush receipt'
rows = []
files = []
overhead = []
for path in sorted((OUT / (label + '-trace')).glob('thread-*.tsv')):
    lines = path.read_text().splitlines()
    trace_file(path.read_bytes(), path.name)
    assert lines[0].startswith('sensor\ti557-tls-v1\tthread\t'), path
    thread = int(lines[0].split('\t')[-1])
    assert path.name == f'thread-{thread}.tsv'
    assert lines[-1].startswith('END\t'), path
    end = lines[-1].split('\t')
    assert int(end[1]) == len(lines)-3, path
    assert receipts.get(thread) == (len(lines)-3, True), path
    overhead.append(int(end[2]))
    for index, line in enumerate(lines[2:-1]):
        fields = line.split('\t')
        assert len(fields) == 9, (path, index)
        row = {key: fields[i] if key == 'phase' else int(fields[i]) for i, key in enumerate(['ns', 'fixture', 'run', 'request', 'poll', 'phase', 'a', 'b', 'ok'])}
        row.update(thread=thread, index=index)
        rows.append(row)
    files.append(dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest(), rows=len(lines)-3))
assert rows, 'No trace rows, no causal conclusion allowed'
assert len(receipts) == len(files), 'Flush receipt without trace file'
rows.sort(key=lambda r: (r['ns'], r['thread'], r['index']))

def spread(values):
    return dict(n=len(values), min=min(values, default=0), median=statistics.median(values) if values else 0, max=max(values, default=0))

def durations(selected, begin, end):
    starts = {}
    values = []
    for row in selected:
        key = (row['thread'], row['run'])
        if row['phase'] == begin:
            starts[key] = row['ns']
        if row['phase'] == end and key in starts:
            values.append((row['ns']-starts.pop(key))/1e6)
    return spread(values)

results = []
begins = [r for r in rows if r['phase'] == 'words_begin']
assert len(begins) == 1, 'Expected exactly one Words fixture'
for begin in begins:
    fixture, request = begin['fixture'], begin['request']
    links = [r for r in rows if r['fixture'] == fixture and r['phase'] == 'request_link' and r['a'] == request]
    assert len(links) == 1 and links[0]['ok'] and links[0]['run'], 'Ambiguous/missing request-to-run link'
    run = links[0]['run']
    fake = [r for r in rows if r['fixture'] == fixture and r['request'] == request and r['thread'] == begin['thread']]
    selected = [r for r in rows if r['fixture'] == fixture and r['run'] == run]
    watches = [r for r in rows if r['fixture'] == fixture and r['phase'] == 'watch_begin']
    assert len(watches) == 1
    ends = [r for r in rows if r['fixture'] == fixture and r['phase'] == 'watch_end' and r['ns'] >= watches[0]['ns']]
    assert len(ends) == 1
    watch = [r for r in rows if r['fixture'] == fixture and watches[0]['ns'] <= r['ns'] <= ends[0]['ns']]
    assert all(r['ok'] for r in fake + selected + watch), 'Failed phase in target fixture'
    words = [r for r in fake if r['phase'] == 'word_enter']
    assert [r['a'] for r in words] == list(range(1, 41))
    assert sum(r['phase'] == 'word_return' for r in fake) == 40
    assert sum(r['phase'] == 'word_wake' for r in fake) == 40
    assert sum(r['phase'] == 'flush_return' for r in fake) == 41
    assert sum(r['phase'] == 'write_return' for r in fake) == 41
    received = [r for r in selected if r['phase'] == 'gateway_received']
    assert [r['a'] for r in received] == list(range(1, 42))
    wire = [r for r in selected if r['phase'] == 'wire_receive']
    assert [r['a'] for r in wire] == list(range(1, 42))
    assert sum(r['phase'] == 'wire_send_return' for r in selected) == 41
    assert sum(r['phase'] == 'emit_return' for r in selected) == 41
    inputs = [r for r in selected if r['phase'] == 'filter_input']
    assert [r['a'] for r in inputs] == list(range(1, 41))
    assert sum(r['b'] for r in inputs) == 230
    committed = []
    pending = {}
    for row in selected:
        thread = row['thread']
        if row['phase'] == 'tx_enter':
            pending[thread] = []
        elif row['phase'] == 'inserted' and row['b']:
            assert thread in pending, 'Insertion outside traced transaction'
            pending[thread].append(dict(row))
        elif row['phase'] == 'commit_enter':
            for inserted in pending.get(thread, []):
                inserted['commit_enter_ns'] = row['ns']
        elif row['phase'] == 'tx_error':
            pending.pop(thread, None)
        elif row['phase'] == 'tx_committed':
            for inserted in pending.pop(thread, []):
                assert inserted['a'] <= row['a']
                committed.append(dict(seq=inserted['a'], bytes=inserted['b'], inserted_ns=inserted['ns'], commit_enter_ns=inserted['commit_enter_ns'], committed_ns=row['ns']))
    assert sum(c['bytes'] for c in committed) == 230
    cumulative = 0
    for c in committed:
        cumulative += c['bytes']
        c['cumulative_bytes'] = cumulative
    visible = [r for r in watch if r['phase'] == 'visible_stamp']
    assert visible[-1]['b'] == 40
    gaps = [dict(ms=(b['a']-a['a'])/1e6, from_poll=a['poll'], to_poll=b['poll'], from_words=a['b'], to_words=b['b'], from_ns=a['a'], to_ns=b['a']) for a, b in zip(visible, visible[1:])]
    polls = []
    for poll in sorted({r['poll'] for r in watch if r['phase'] == 'poll_wake'}):
        rr = [r for r in watch if r['poll'] == poll]
        byphase = collections.defaultdict(list)
        for row in rr:
            if row['run'] in (0, run):
                byphase[row['phase']].append(row)
        for phase in ['poll_wake', 'api_begin', 'api_dispatch_return', 'log_enter', 'log_return', 'api_return', 'reply_return', 'visible']:
            assert len(byphase[phase]) == 1, (poll, phase, len(byphase[phase]))
        event_rows = byphase['events_used']
        projected = byphase['projected']
        assert event_rows and projected, ('Missing used event vector', poll)
        wake = byphase['poll_wake'][0]['ns']
        api_begin = byphase['api_begin'][0]['ns']
        api_end = byphase['api_return'][0]['ns']
        last_visible = byphase['visible'][0]
        previous_sleep = [r for r in watch if r['phase'] == 'sleep_enter' and r['poll'] == poll-1]
        durable_before = [c for c in committed if c['committed_ns'] < api_begin]
        item = dict(poll=poll, wake_ns=wake, api_begin_ns=api_begin, api_return_ns=api_end,
                    dispatcher_ms=(byphase['api_dispatch_return'][0]['ns']-api_begin)/1e6,
                    log_ms=(byphase['log_return'][0]['ns']-byphase['log_enter'][0]['ns'])/1e6,
                    extraction_ms=(last_visible['ns']-api_end)/1e6,
                    bytes=last_visible['a'], words=last_visible['b'],
                    seq_used=event_rows[-1]['a'], text_seq_used=event_rows[-1]['b'],
                    committed_seq_before_api=max((c['seq'] for c in durable_before), default=0),
                    durable_bytes_before_api=durable_before[-1]['cumulative_bytes'] if durable_before else 0,
                    commit_interval_overlaps_api_begin=[c['seq'] for c in committed if c['commit_enter_ns'] <= api_begin <= c['committed_ns']],
                    wake_lateness_ms=max(0, wake-previous_sleep[-1]['a'])/1e6 if previous_sleep else None)
        polls.append(item)
    worst = max(gaps, key=lambda g: g['ms'], default=None)
    window_begin = worst['from_ns']-100_000_000 if worst else watches[0]['ns']
    window_end = worst['to_ns']+100_000_000 if worst else ends[0]['ns']
    reduced = [r for r in rows if r['fixture'] == fixture and window_begin <= r['ns'] <= window_end and (r['run'] in (0, run) or r['thread'] == begin['thread'])]
    target_file = OUT / f'{label}-fixture-{fixture}-worst.json'
    target_file.write_text(json.dumps(reduced, indent=2))
    commits_by_seq = {c['seq']: c for c in committed}
    complete_before_missing = []
    for p in polls:
        if p['committed_seq_before_api'] > p['text_seq_used']:
            complete_before_missing.append(p['poll'])
    result = dict(fixture=fixture, run=run, request=request, words=40, source_bytes=230,
                  commits=committed, polls=polls, visible=len(visible), gaps=gaps,
                  gap_ms=spread([g['ms'] for g in gaps]), first_break_demonstrated=None,
                  words_interval_ms=spread([(b['ns']-a['ns'])/1e6 for a, b in zip(words, words[1:])]),
                  emission_to_gateway_ms=spread([(r['ns']-w['ns'])/1e6 for w, r in zip(words, received[:40])]),
                  begin_ms=durations(selected, 'begin_enter', 'begin_return'),
                  commit_ms=durations(selected, 'commit_enter', 'tx_committed'),
                  filter_refresh_ms=durations(selected, 'filter_refresh_enter', 'filter_refresh_return'),
                  wire_send_ms=durations(selected, 'wire_send_enter', 'wire_send_return'),
                  emit_ms=durations(selected, 'emit_enter', 'emit_return'),
                  api_dispatch_ms=spread([p['dispatcher_ms'] for p in polls]),
                  log_ms=spread([p['log_ms'] for p in polls]),
                  extraction_ms=spread([p['extraction_ms'] for p in polls]),
                  wake_lateness_ms=spread([p['wake_lateness_ms'] for p in polls if p['wake_lateness_ms'] is not None]),
                  committed_before_api_but_missing_polls=complete_before_missing,
                  worst_gap=worst, worst_artifact=str(target_file))
    results.append(result)
summary = dict(label=label, complete=True, sensor='i557-tls-v1', files=files, rows=len(rows),
               measured_record_overhead_ns=sum(overhead), mean_record_overhead_ns=sum(overhead)/len(rows),
               fixtures=results,
               tests=re.findall(r'test result:.*', log))
(OUT / (label + '-analysis.json')).write_text(json.dumps(summary, indent=2))
print(json.dumps({k: v for k, v in summary.items() if k not in ['files', 'fixtures']}))
for result in results:
    print(json.dumps({k: v for k, v in result.items() if k not in ['commits', 'polls', 'gaps']}))
