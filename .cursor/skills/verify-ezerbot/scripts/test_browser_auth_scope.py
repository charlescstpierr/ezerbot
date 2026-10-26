#!/usr/bin/env python3
"""Drive the production auth reader with deterministic CDP messages."""
import copy
import json
from pathlib import Path
import queue
import sys
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / 'tools'))
import local_api

ORIGIN = 'http://127.0.0.1:43210'
WS = 'ws://127.0.0.1:43123/devtools/browser/current'


class Identity:
    def __init__(self):
        self.calls = []

    def headers(self, origin):
        self.calls.append(origin)
        return {'X-Ezerbot-Launch': 'dummy-identity', 'Origin': origin, 'X-Grokbot-Request': 'local'}


class Transport:
    def __init__(self, contexts):
        self.incoming = queue.Queue()
        self.commands = []
        self.readers = set()
        self.condition = threading.Condition()
        self.contexts = contexts
        self.attached = False
        self.closed = False
        self.sock = SimpleNamespace(settimeout=lambda value: None, shutdown=self.shutdown)

    def shutdown(self, how):
        self.incoming.put(ConnectionError('closed by owning handle'))

    def close(self):
        self.closed = True

    def _message(self):
        self.readers.add(threading.get_ident())
        message = self.incoming.get(timeout=3)
        if isinstance(message, BaseException):
            raise message
        return message

    def _send(self, opcode, payload):
        message = json.loads(payload)
        with self.condition:
            self.commands.append(copy.deepcopy(message))
            self.condition.notify_all()
        if message['method'] == 'Target.setAutoAttach' and not message.get('sessionId') and not self.attached:
            self.attached = True
            for session, context in self.contexts:
                info = {'targetId': 'target-' + session, 'type': 'page'}
                if context is not None:
                    info['browserContextId'] = context
                self.incoming.put({'method': 'Target.attachedToTarget', 'params': {'sessionId': session, 'targetInfo': info}})
        self.incoming.put({'id': message['id'], 'result': {}})

    def wait_command(self, predicate):
        deadline = time.monotonic() + 2
        with self.condition:
            while True:
                match = next((message for message in self.commands if predicate(message)), None)
                if match is not None:
                    return match
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AssertionError('expected CDP effect was not observed')
                self.condition.wait(remaining)

    def pause(self, session, identifier, url=ORIGIN + '/api/state', **extra):
        self.incoming.put({'method': 'Fetch.requestPaused', 'sessionId': session, 'params': {
            'requestId': identifier, 'request': {'url': url, 'headers': {'x-ezerbot-launch': 'untrusted'}}, **extra}})
        return self.wait_command(lambda message: message['method'].startswith('Fetch.')
                                 and message['params'].get('requestId') == identifier)


class AuthScopeTests(unittest.TestCase):
    def start(self, contexts, context='run-A'):
        self.transport = Transport(contexts)
        self.identity = Identity()
        with patch.object(local_api, '_browser_module', return_value=SimpleNamespace(Cdp=lambda endpoint: self.transport)):
            authentication = local_api.BrowserAuthentication(self.identity, ORIGIN, WS, browser_context=context)
        self.addCleanup(authentication.close)
        return authentication

    def test_only_attributed_context_is_prepared_and_resumed(self):
        authentication = self.start([('owned', 'run-A'), ('foreign', 'run-B'), ('default', None)])
        authentication.check()
        prepared = [message['sessionId'] for message in self.transport.commands if message['method'] == 'Fetch.enable']
        resumed = [message['sessionId'] for message in self.transport.commands if message['method'] == 'Runtime.runIfWaitingForDebugger']
        detached = [message['params']['sessionId'] for message in self.transport.commands if message['method'] == 'Target.detachFromTarget']
        self.assertEqual(prepared, ['owned'])
        self.assertEqual(resumed, ['owned'])
        self.assertCountEqual(detached, ['foreign', 'default'])
        self.assertEqual(self.identity.calls, [])
        self.assertEqual(len(self.transport.readers), 1)

    def test_reader_applies_exact_origin_policy_and_rejects_unbound_session(self):
        self.start([('owned', 'run-A')])
        local = self.transport.pause('owned', 'local')
        headers = local['params']['headers']
        self.assertTrue(any(header['name'] == 'X-Ezerbot-Launch' and header['value'] == 'dummy-identity' for header in headers))
        foreign_origin = self.transport.pause('owned', 'other-origin', 'http://127.0.0.1:43211/')
        self.assertFalse(any(header['name'].lower() == 'x-ezerbot-launch' for header in foreign_origin['params']['headers']))
        foreign_session = self.transport.pause('unbound', 'unbound')
        self.assertEqual(foreign_session['method'], 'Fetch.failRequest')
        self.assertEqual(self.identity.calls, [ORIGIN])

    def test_detach_revokes_session_and_late_request_has_no_identity(self):
        self.start([('owned', 'run-A')])
        self.transport.incoming.put({'method': 'Target.detachedFromTarget', 'params': {'sessionId': 'owned'}})
        late = self.transport.pause('owned', 'late')
        self.assertEqual(late['method'], 'Fetch.failRequest')
        self.assertEqual(self.identity.calls, [])

    def test_close_joins_the_only_reader_and_is_idempotent(self):
        authentication = self.start([('owned', 'run-A')])
        authentication.close()
        self.assertTrue(self.transport.closed)
        with self.assertRaises(local_api.LocalApiError):
            authentication.check()
        self.assertFalse(any(thread.ident in self.transport.readers and thread.is_alive() for thread in threading.enumerate()))
        authentication.close()

    def test_context_mode_refuses_page_connection_before_connect(self):
        with patch.object(local_api, '_browser_module') as connect:
            with self.assertRaises(local_api.LocalApiError):
                local_api.BrowserAuthentication(Identity(), ORIGIN, WS.replace('/browser/', '/page/'), browser_context='run-A')
        connect.assert_not_called()

    def test_empty_context_refuses_before_connect(self):
        with patch.object(local_api, '_browser_module') as connect:
            with self.assertRaises(local_api.ScopeError):
                local_api.BrowserAuthentication(Identity(), ORIGIN, WS, browser_context='')
        connect.assert_not_called()

    def test_legacy_per_run_caller_keeps_its_existing_interface(self):
        self.start([('owned', 'any-context')], context=None)
        local = self.transport.pause('owned', 'legacy')
        self.assertEqual(local['method'], 'Fetch.continueRequest')
        self.assertTrue(any(header['name'] == 'X-Ezerbot-Launch' for header in local['params']['headers']))


if __name__ == '__main__':
    unittest.main()
