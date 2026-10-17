#!/usr/bin/env python3
"""Read shipped bytes; index compaction candidates and native helper strings."""
from pathlib import Path
import hashlib
import json
import re

ROOT = Path(__file__).resolve().parents[1]
TOKENS = [
    'client_speculative_summarization_config', 'tokenUsageThresholdPercentage',
    'PreCompactRequestQuery', 'PreCompactRequestResponse', 'pre_compact',
    'summarizeAction', 'SummarizeAction', 'summarizedComposers',
    'contextWindowOverflow', 'context-window overflow', 'compaction',
    'messagesToCompact', 'isFirstCompaction',
]

def main():
    files = sorted(p for p in (ROOT / 'artifacts/client').rglob('*')
                   if p.is_file() and p.suffix in {'.js', '.cjs', '.mjs'})
    rows = []
    for path in files:
        raw = path.read_bytes()
        text = raw.decode('utf-8')
        hits = []
        for token in TOKENS:
            matches = list(re.finditer(re.escape(token), text, re.IGNORECASE))
            if not matches:
                continue
            samples = []
            for match in matches[:12]:
                start = match.start()
                samples.append({'original_utf16_offset': len(text[:start].encode('utf-16-le')) // 2,
                                'context': text[max(0,start-100):match.end()+140]})
            hits.append({'token': token, 'count': len(matches), 'samples': samples,
                         'samples_complete': len(matches) <= 12})
        if hits:
            rows.append({'path': str(path.relative_to(ROOT)),
                         'sha256': hashlib.sha256(raw).hexdigest(), 'hits': hits})
    helper = ROOT / 'inputs/Grok Bot.app/Contents/Helpers/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
    raw = helper.read_bytes()
    pattern = re.compile(r'computer_use_|control/|messages/|peer|permission|rpcSocket|snapshot|screen.?capture|accessibility|signing',re.I)
    strings = [{'file_offset': m.start(), 'value': m.group().decode('ascii')}
               for m in re.finditer(rb'[\x20-\x7e]{5,}', raw)
               if pattern.search(m.group().decode('ascii'))]
    result = {'scope': 'Literal searches over all shipped JS, plus raw ASCII byte strings in the saved native helper',
              'javascript_files_scanned': len(files), 'matching_files': rows,
              'helper': {'path': str(helper.relative_to(ROOT)), 'sha256': hashlib.sha256(raw).hexdigest(),
                         'strings': strings},
              'limitations': ['Literal matches do not prove execution or reachability.',
                              'No model context or authenticated session was observed.',
                              'ASCII strings are not decompilation; file offsets refer to the universal carrier.',
                              'Missing literal matches do not exclude dynamically constructed equivalents.']}
    dest = ROOT / 'evidence/uncovered-search.json'
    dest.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'output':str(dest.relative_to(ROOT)), 'js_files':len(files),
                      'matching_files':len(rows), 'helper_strings':len(strings)}))

if __name__ == '__main__':
    main()
