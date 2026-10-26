#!/usr/bin/env python3
"""Replay CDP messages; a ready old document is not a navigation receipt."""
import json
import unittest
from unittest.mock import Mock

import browser

URL = 'http://127.0.0.1:43210/'


def commit(loader='new', frame='main'):
    return {'method': 'Page.frameNavigated', 'params': {'frame': {
        'id': frame, 'loaderId': loader, 'url': URL,
        **({'parentId': 'main'} if frame != 'main' else {})}}}


def loaded(loader='new', frame='main', session=None):
    return {'method': 'Page.lifecycleEvent', 'params': {
        'frameId': frame, 'loaderId': loader, 'name': 'load'},
        **({'sessionId': session} if session else {})}


class Replay(browser.Cdp):
    def __init__(self, *, before_reply=False, events=None, navigation=None):
        self.next_id = 1
        self._event_handler = None
        self.sock = Mock()
        self.queue = []
        self.delayed = list(events if events is not None else [commit(), loaded()])
        self.before_reply = before_reply
        self.navigation = navigation if navigation is not None else {'frameId': 'main', 'loaderId': 'new'}
        self.document = 'old'
        self.evaluated = []
        self.methods = []

    def _send(self, opcode, payload):
        request = json.loads(payload)
        method = request['method']
        self.methods.append(method)
        result = {}
        if method == 'Page.getFrameTree':
            result = {'frameTree': {'frame': {'id': 'main', 'loaderId': self.document, 'url': URL}}}
        elif method == 'Page.navigate':
            result = self.navigation
            if self.before_reply:
                self.queue.extend(self.delayed)
                self.delayed = []
        elif method == 'Runtime.evaluate':
            self.evaluated.append(self.document)
            result = {'result': {'value': {'href': URL, 'editor': True, 'model': True}}}
        self.queue.append({'id': request['id'], 'result': result})

    def _message(self):
        if not self.queue:
            self.queue, self.delayed = self.delayed, []
        if not self.queue:
            raise TimeoutError('deterministic navigation deadline')
        message = self.queue.pop(0)
        if isinstance(message, BaseException):
            raise message
        if message.get('method') == 'Page.frameNavigated' and not message.get('sessionId'):
            frame = message['params']['frame']
            if frame['id'] == 'main':
                self.document = frame['loaderId']
        return message


class NavigationTests(unittest.TestCase):
    def test_same_url_ready_old_document_waits_for_new_commit_and_load(self):
        cdp = Replay()
        browser.command_open(cdp, URL, URL)
        self.assertEqual(cdp.evaluated, ['new'], 'old ready document must never satisfy navigation')

    def test_commit_and_load_before_navigate_reply_are_not_lost(self):
        cdp = Replay(before_reply=True)
        receipt = browser.command_open(cdp, URL, URL)
        self.assertEqual(cdp.evaluated, ['new'])
        self.assertEqual(receipt, {'frame_id': 'main', 'loader_id': 'new'})

    def test_unrelated_events_cannot_complete_navigation(self):
        for events in ([loaded('old')], [commit(frame='iframe'), loaded(frame='iframe')],
                       [commit(), loaded(session='foreign-target')], [loaded()], [commit()]):
            with self.subTest(events=events):
                cdp = Replay(events=events)
                with self.assertRaises((TimeoutError, SystemExit)):
                    browser.command_open(cdp, URL, URL)
                self.assertEqual(cdp.evaluated, [])

    def test_unrelated_events_before_actual_load_are_ignored(self):
        cdp = Replay(events=[loaded('old'), commit(frame='iframe'), loaded(frame='iframe'),
                             loaded(session='foreign-target'), commit(), loaded()])
        browser.command_open(cdp, URL, URL)
        self.assertEqual(cdp.evaluated, ['new'])

    def test_no_fresh_loader_download_error_and_foreign_frame_refuse(self):
        for result in ({'frameId': 'main'}, {'frameId': 'main', 'loaderId': 'old'},
                       {'frameId': 'main', 'loaderId': 'new', 'isDownload': True},
                       {'errorText': 'refused'}, {'frameId': 'iframe', 'loaderId': 'new'}):
            with self.subTest(result=result):
                cdp = Replay(navigation=result)
                with self.assertRaises(SystemExit):
                    browser.command_open(cdp, URL, URL)
                self.assertEqual(cdp.evaluated, [])

    def test_close_and_timeout_never_evaluate_ready_old_page(self):
        for events in ([ConnectionError('closed')], [],
                       [{'method': 'Inspector.detached', 'params': {}}],
                       [{'method': 'Page.frameDetached', 'params': {'frameId': 'main'}}]):
            with self.subTest(events=events):
                cdp = Replay(events=events)
                with self.assertRaises((ConnectionError, TimeoutError, SystemExit)):
                    browser.command_open(cdp, URL, URL)
                self.assertEqual(cdp.evaluated, [])

    def test_replaced_main_document_cannot_use_previous_load(self):
        cdp = Replay(before_reply=True, events=[commit(), loaded(), commit('replacement')])
        with self.assertRaises(SystemExit):
            browser.command_open(cdp, URL, URL)
        self.assertEqual(cdp.evaluated, [])

    def test_wrong_origin_is_rejected_before_cdp_effect(self):
        cdp = Replay()
        with self.assertRaises(SystemExit):
            browser.command_open(cdp, 'http://127.0.0.1:43211/', URL)
        self.assertEqual(cdp.methods, [])


if __name__ == '__main__':
    unittest.main()
