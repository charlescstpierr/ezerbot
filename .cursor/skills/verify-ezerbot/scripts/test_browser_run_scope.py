#!/usr/bin/env python3
"""Capability/auth contracts, without Chrome, process ownership or real secrets."""
from pathlib import Path
import sys
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / 'tools'))
from browser_run_scope import BrowserRunScope, BrowserEpoch, RunLease, ScopeError, Phase

ORIGIN = 'http://127.0.0.1:43210'


class FakeIdentity:
    def __init__(self, label):
        self.label = label
        self.calls = []

    def headers(self, origin):
        self.calls.append(origin)
        return {'X-Ezerbot-Launch': 'dummy-' + self.label, 'Origin': origin, 'X-Grokbot-Request': 'local'}


def request(identifier='request', url=ORIGIN + '/api/state', **extra):
    return {'requestId': identifier, 'request': {'url': url, 'headers': {}}, **extra}


class RunScopeTests(unittest.TestCase):
    def setUp(self):
        self.scope = BrowserRunScope()
        self.identity_a = FakeIdentity('A')
        self.lease_a = self.scope.begin_run(self.scope.epoch, 'context-A', ORIGIN, self.identity_a)
        self.scope.bind_session(self.lease_a, 'session-A', {'targetId': 'target-A', 'browserContextId': 'context-A'})
        self.effects = []

    def paused(self, session, params):
        self.scope.dispatch_paused(session, params, lambda *effect: self.effects.append(effect))
        return self.effects[-1]

    def drain_a(self):
        self.assertEqual(self.scope.begin_draining(self.lease_a), 'context-A')
        self.scope.context_disposed(self.lease_a)
        self.scope.session_detached('session-A')
        self.scope.finish_draining(self.lease_a)

    def begin_b(self):
        self.identity_b = FakeIdentity('B')
        self.lease_b = self.scope.begin_run(self.scope.epoch, 'context-B', ORIGIN, self.identity_b)
        self.scope.bind_session(self.lease_b, 'session-B', {'targetId': 'target-B', 'browserContextId': 'context-B'})

    def test_only_one_run_and_only_living_epoch_are_admitted(self):
        for epoch in (self.scope.epoch, BrowserEpoch()):
            with self.assertRaises(ScopeError):
                self.scope.begin_run(epoch, 'context-B', ORIGIN, FakeIdentity('B'))
        self.assertEqual(self.scope.phase, Phase.ACTIVE)

    def test_foreign_missing_context_and_reused_session_never_bind(self):
        for target in ({'targetId': 'foreign', 'browserContextId': 'context-B'},
                       {'targetId': 'default'}, {'browserContextId': 'context-A'}):
            with self.subTest(target=target), self.assertRaises(ScopeError):
                self.scope.bind_session(self.lease_a, 'foreign-session', target)
        with self.assertRaises(ScopeError):
            self.scope.bind_session(self.lease_a, 'session-A', {'targetId': 'new', 'browserContextId': 'context-A'})
        self.assertEqual(self.paused('foreign-session', request())[0], 'Fetch.failRequest')
        self.assertEqual(self.identity_a.calls, [])

    def test_exact_origin_auth_and_incoming_header_stripping(self):
        local = request()
        local['request']['headers'] = {'x-ezerbot-launch': 'untrusted', 'Origin': 'wrong', 'Accept': 'text/html'}
        method, params, session = self.paused('session-A', local)
        headers = {header['name'].lower(): header['value'] for header in params['headers']}
        self.assertEqual(method, 'Fetch.continueRequest')
        self.assertEqual(session, 'session-A')
        self.assertEqual(headers['x-ezerbot-launch'], 'dummy-A')
        self.assertEqual(headers['origin'], ORIGIN)
        self.assertEqual(headers['accept'], 'text/html')
        for url in ('http://127.0.0.1:43211/', 'http://localhost:43210/',
                    'https://127.0.0.1:43210/', ORIGIN.replace('127.0.0.1', 'user@127.0.0.1')):
            params = request(url=url)
            params['request']['headers'] = {'X-Ezerbot-Launch': 'untrusted'}
            _, decision, _ = self.paused('session-A', params)
            self.assertFalse(any(header['name'].lower() == 'x-ezerbot-launch' for header in decision['headers']))
        self.assertEqual(self.identity_a.calls, [ORIGIN])

    def test_authenticated_request_and_response_redirects_are_refused(self):
        self.paused('session-A', request(networkId='network'))
        for code in (301, 302, 303, 307, 308):
            self.paused('session-A', request(identifier=str(code), networkId=str(code)))
            self.assertEqual(self.paused('session-A', request(identifier=str(code), networkId=str(code), responseStatusCode=code))[0], 'Fetch.failRequest')
        self.assertEqual(self.paused('session-A', request(identifier='redirect', redirectedRequestId='request', networkId='network'))[0], 'Fetch.failRequest')
        self.assertEqual(self.paused('session-A', request(responseStatusCode=304))[0], 'Fetch.continueRequest')

    def test_drain_requires_disposal_sessions_and_command_replies(self):
        token = self.scope.command_started(self.lease_a)
        self.scope.begin_draining(self.lease_a)
        with self.assertRaises(ScopeError):
            self.scope.command_started(self.lease_a)
        with self.assertRaises(ScopeError):
            self.scope.finish_draining(self.lease_a)
        self.scope.context_disposed(self.lease_a)
        with self.assertRaises(ScopeError):
            self.scope.finish_draining(self.lease_a)
        self.scope.session_detached('session-A')
        with self.assertRaises(ScopeError):
            self.scope.finish_draining(self.lease_a)
        self.scope.command_finished(token)
        self.scope.finish_draining(self.lease_a)
        self.assertEqual(self.scope.phase, Phase.IDLE)

    def test_revocation_blocks_late_attach_and_request_before_identity_access(self):
        self.scope.begin_draining(self.lease_a)
        with self.assertRaises(ScopeError):
            self.scope.bind_session(self.lease_a, 'late', {'targetId': 'late', 'browserContextId': 'context-A'})
        self.assertEqual(self.paused('session-A', request())[0], 'Fetch.failRequest')
        self.assertEqual(self.identity_a.calls, [])

    def test_cleanup_a_during_b_and_forged_lease_refuse_before_effect(self):
        self.drain_a()
        self.begin_b()
        for stale in (self.lease_a, RunLease(self.scope.epoch, self.lease_b.generation), None):
            with self.subTest(stale=stale), self.assertRaises(ScopeError):
                self.scope.begin_draining(stale)
        self.assertEqual(self.scope.phase, Phase.ACTIVE)
        self.assertEqual(self.effects, [])
        self.assertEqual(self.paused('session-A', request())[0], 'Fetch.failRequest')
        self.assertEqual(self.paused('session-B', request())[1]['headers'][0]['value'], 'dummy-B')
        self.assertEqual(self.identity_a.calls, [])

    def test_late_response_network_id_reuse_does_not_authorize_b(self):
        self.paused('session-A', request(networkId='reused'))
        self.drain_a()
        self.begin_b()
        self.assertEqual(self.paused('session-A', request(responseStatusCode=302, networkId='reused'))[0], 'Fetch.failRequest')
        self.assertEqual(self.paused('session-B', request(responseStatusCode=302, networkId='reused'))[0], 'Fetch.continueRequest')
        self.assertEqual(self.identity_b.calls, [])
        with self.assertRaises(ScopeError):
            self.scope.bind_session(self.lease_b, 'session-A', {'targetId': 'new', 'browserContextId': 'context-B'})

    def test_poison_close_and_restart_never_adopt_old_capabilities(self):
        token = self.scope.command_started(self.lease_a)
        self.scope.command_finished(token, failed=True)
        self.assertEqual(self.scope.phase, Phase.POISONED)
        self.assertEqual(self.paused('session-A', request())[0], 'Fetch.failRequest')
        with self.assertRaises(ScopeError):
            self.scope.begin_run(self.scope.epoch, 'context-B', ORIGIN, FakeIdentity('B'))
        self.scope.close(self.scope.epoch)
        self.assertEqual(self.scope.phase, Phase.CLOSED)
        restarted = BrowserRunScope()
        with self.assertRaises(ScopeError):
            restarted.begin_run(self.scope.epoch, 'context-B', ORIGIN, FakeIdentity('B'))
        with self.assertRaises(ScopeError):
            restarted.begin_draining(self.lease_a)

    def test_context_ids_cannot_be_reused_after_disposal(self):
        self.drain_a()
        with self.assertRaises(ScopeError):
            self.scope.begin_run(self.scope.epoch, 'context-A', ORIGIN, FakeIdentity('B'))

    def test_auth_and_send_cannot_cross_revocation(self):
        entered, release, attempting, draining = (threading.Event() for _ in range(4))
        failures = []
        def send(*effect):
            entered.set()
            if not release.wait(2):
                failures.append('send was not released')
            self.effects.append(effect)
        def dispatch():
            try:
                self.scope.dispatch_paused('session-A', request(), send)
            except BaseException as error:
                failures.append(type(error).__name__)
        def drain():
            try:
                attempting.set()
                self.scope.begin_draining(self.lease_a)
                draining.set()
            except BaseException as error:
                failures.append(type(error).__name__)
        sender = threading.Thread(target=dispatch)
        sender.start()
        self.assertTrue(entered.wait(2))
        closer = threading.Thread(target=drain)
        closer.start()
        self.assertTrue(attempting.wait(2))
        self.assertFalse(draining.is_set())
        release.set()
        sender.join(2)
        closer.join(2)
        self.assertFalse(sender.is_alive())
        self.assertFalse(closer.is_alive())
        self.assertEqual(failures, [])
        self.assertTrue(draining.is_set())
        self.assertEqual(self.paused('session-A', request())[0], 'Fetch.failRequest')


if __name__ == '__main__':
    unittest.main()
