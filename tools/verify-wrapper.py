#!/usr/bin/env python3
"""Exercise the actual local HTTP wrapper and its SQLite restart behavior."""
from pathlib import Path
from datetime import datetime, timezone
import json
import selectors
import subprocess
import tempfile
import urllib.request
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/grokbot-wrapper'
checks = []


def require(name, condition):
    checks.append({'name': name, 'passed': bool(condition)})
    if not condition:
        raise AssertionError(name)


class Server:
    def __init__(self, database):
        self.process = subprocess.Popen([str(BINARY), '--port', '0', '--db', str(database)],
                                        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        selector = selectors.DefaultSelector()
        selector.register(self.process.stdout, selectors.EVENT_READ)
        if not selector.select(15):
            self.close()
            raise RuntimeError('Wrapper did not announce its URL')
        line = self.process.stdout.readline()
        selector.close()
        self.url = json.loads(line)['url']
        require('loopback binding', self.url.startswith('http://127.0.0.1:'))

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.process.stdout.close()
        self.process.stderr.close()

    def request(self, path, payload=None, headers=None, raw=None):
        values = {'X-Grokbot-Request': 'local', 'Origin': self.url,
                  'Content-Type': 'application/json'}
        if headers:
            values.update(headers)
        data = raw if raw is not None else (json.dumps(payload).encode() if payload is not None else None)
        request = urllib.request.Request(self.url + path, data=data, headers=values)
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                content = response.read()
                return response.status, content, dict(response.headers)
        except urllib.error.HTTPError as error:
            return error.code, error.read(), dict(error.headers)

    def json(self, path, payload=None):
        status, content, _ = self.request(path, payload)
        require(f'{path} returned success', status == 200 or status == 201)
        return json.loads(content)


report = {'scope': 'Local wrapper HTTP/SQLite with simulated responses, no external model',
          'verified_at_utc': datetime.now(timezone.utc).isoformat()}
active = None
try:
    with tempfile.TemporaryDirectory(prefix='grokbot-wrapper-qa-') as directory:
        database = Path(directory) / 'workspace.sqlite'
        active = Server(database)
        for path in ['/', '/styles.css', '/app.js', '/mascot.css', '/mascot.js']:
            status, content, _ = active.request(path)
            require(f'offline asset {path}', status == 200 and len(content) > 100)
        initial = active.json('/api/state')
        require('mode is explicitly simulated', initial['mode'] == 'simulated')
        conversation = active.json('/api/conversations', {'title': 'Conversation de vérification'})['conversation']
        identifier = conversation['id']
        body = 'Bonjour depuis la vérification HTTP.'
        request = {'conversation_id': identifier, 'message_id': 'wrapper-smoke-message',
                   'body': body, 'simulate_loss': True}
        uncertain = active.json('/api/send', request)['conversation']
        users = [item for item in uncertain['messages'] if item['role'] == 'user']
        require('lost reply remains uncertain', len(users) == 1 and users[0]['state'] == 'unknown')
        require('no premature assistant reply', not any(item['role'] == 'assistant' for item in uncertain['messages']))
        reconciled = active.json('/api/reconcile', {'conversation_id': identifier,
                                'message_id': 'wrapper-smoke-message'})['conversation']
        require('reconciliation reaches echo', any(item['role'] == 'user' and item['state'] == 'echoed'
                                                 for item in reconciled['messages']))
        repeated = active.json('/api/send', request)['conversation']
        require('same nonce has one user message and one reply',
                sum(item['role'] == 'user' for item in repeated['messages']) == 1 and
                sum(item['role'] == 'assistant' for item in repeated['messages']) == 1)
        status, _, _ = active.request('/api/send', {**request, 'body': 'Different immutable body'})
        require('same nonce and different body rejected', status == 409)
        active.json('/api/conversations/update', {'id': identifier, 'title': 'Retrouvée après redémarrage',
                                                  'favorite': True})
        active.json('/api/preferences', {'theme': 'dark'})
        active.close()
        active = Server(database)
        state = active.json('/api/state')
        restored = next(item for item in state['conversations'] if item['id'] == identifier)
        require('appearance survives dynamic-port restart', state['preferences']['theme'] == 'dark')
        require('title and favorite survive restart', restored['title'] == 'Retrouvée après redémarrage'
                and restored['favorite'] is True)
        require('message and reply survive restart', len(restored['messages']) == 2 and
                next(item['body'] for item in restored['messages'] if item['role'] == 'user') == body)
        status, _, _ = active.request('/api/conversations', {'title': 'Denied'},
                                     {'Origin': 'https://foreign.example'})
        require('foreign origin rejected', status == 403)
        status, _, _ = active.request('/api/state', headers={'Host': 'foreign.example'})
        require('foreign host rejected', status in [400, 403])
        status, _, _ = active.request('/api/conversations', {'title': 'Denied'}, {'X-Grokbot-Request': ''})
        require('missing local request marker rejected', status == 403)
        status, _, _ = active.request('/api/conversations', raw=b'x' * 65537)
        require('oversized body rejected', status == 413)
        status, _, _ = active.request('/../SESSION_STATE.json')
        require('arbitrary filesystem paths are not served', status == 404)
        active.json('/api/conversations/delete', {'id': identifier})
        require('deleted conversation hidden', all(item['id'] != identifier
                for item in active.json('/api/state')['conversations']))
        status, _, _ = active.request('/api/send', request)
        require('late send cannot resurrect a deleted conversation', status in [404, 409])
        active.close()
        active = None
    report['status'] = 'passed'
except Exception as error:
    report['status'] = 'failed'
    report['error'] = str(error)
finally:
    if active:
        active.close()
    report['checks'] = checks
    (ROOT / 'evidence/wrapper-verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(report, ensure_ascii=False, indent=2))
raise SystemExit(0 if report['status'] == 'passed' else 1)
