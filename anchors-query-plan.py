from pathlib import Path
import re
import sqlite3
import subprocess

ROOT = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i295')
EVIDENCE = Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295')
SOURCE = 'crates/wrapper/src/features/pages/anchors.rs'

before = subprocess.check_output(
    ['git', '-C', str(ROOT), 'show', f'HEAD:{SOURCE}'], text=True
)
after = (ROOT / SOURCE).read_text()
lines = [
    f'SQLite {sqlite3.sqlite_version}, Python EXPLAIN QUERY PLAN.',
    'The Rust pages integration test separately measures the real GET with StatementStatus::VmStep.',
    'The fixture has one requested Page anchor and 10,000 anchors for another Page.',
]

for label, source in [('before', before), ('after', after)]:
    migration = re.search(r'conn.execute_batch\(\s*"([^"]+)"', source).group(1)
    query = re.search(r'pub fn list\b.*?conn.prepare\(\s*"([^"]+)"', source, re.S).group(1)
    db = sqlite3.connect(':memory:')
    db.executescript(migration)
    db.executescript(migration)
    db.execute(
        "INSERT INTO wrapper_page_anchors VALUES ('current', 'ancre', 'requested', 'Confort', 'v', 'open', 20000)"
    )
    db.executemany(
        'INSERT INTO wrapper_page_anchors VALUES (?, ?, ?, ?, ?, ?, ?)',
        [('old', f'message-{n}', 'other-page', 'Old passage', 'v', 'open', n) for n in range(10000)],
    )
    plan = [row[3] for row in db.execute('EXPLAIN QUERY PLAN ' + query, ['requested'])]
    rows = db.execute(query, ['requested']).fetchall()
    assert len(rows) == 1
    if label == 'after':
        assert rows[0][:2] == ('current', 'ancre')
        assert all(not step.startswith('SCAN') and 'TEMP B-TREE' not in step for step in plan)
    lines.extend([f'{label.upper()} SQL ' + ' '.join(query.split()), *[f'{label.upper()} PLAN {step}' for step in plan]])
    db.close()

output = '\n'.join(lines) + '\n'
(EVIDENCE / 'anchors-query-plan.txt').write_text(output)
print(output, end='')
