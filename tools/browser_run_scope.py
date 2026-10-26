"""In-memory run capabilities and authentication policy for a supplied browser.

This module owns no process or socket. Its caller must keep the living browser
owner, serialize CDP effects, and attest disposal/detachment/command replies.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
import threading
from urllib.parse import urlsplit


class ScopeError(RuntimeError):
    pass


class Phase(Enum):
    IDLE = 'idle'
    ACTIVE = 'active'
    DRAINING = 'draining'
    POISONED = 'poisoned'
    CLOSED = 'closed'


class BrowserEpoch:
    """Object identity is the capability; endpoint and PID are not authority."""
    __slots__ = ()


@dataclass(frozen=True, eq=False, repr=False)
class RunLease:
    epoch: BrowserEpoch
    generation: int


@dataclass(frozen=True, eq=False, repr=False)
class SessionBinding:
    lease: RunLease
    session: str
    target: str
    context: str
    origin: str
    identity: object = field(repr=False)


@dataclass(frozen=True, repr=False)
class FetchDecision:
    method: str
    params: dict = field(repr=False)


def _origin(url):
    try:
        parsed = urlsplit(url)
        if (parsed.scheme != 'http' or parsed.hostname != '127.0.0.1'
                or parsed.username is not None or parsed.password is not None
                or parsed.port is None or not 0 < parsed.port < 65536
                or any(character.isspace() or ord(character) < 32 for character in url)):
            raise ValueError()
        return f'http://127.0.0.1:{parsed.port}'
    except (TypeError, AttributeError, ValueError):
        raise ScopeError('Expected an exact wrapper loopback origin') from None


class BrowserRunScope:
    def __init__(self):
        self.epoch = BrowserEpoch()
        self.phase = Phase.IDLE
        self.__lock = threading.RLock()
        self.__lease = None
        self.__generation = 0
        self.__context = None
        self.__origin = None
        self.__identity = None
        self.__contexts = set()
        self.__sessions_seen = set()
        self.__bindings = {}
        self.__commands = {}
        self.__authenticated = set()
        self.__disposed = False

    def __require(self, lease, phase):
        if (not isinstance(lease, RunLease) or lease is not self.__lease
                or lease.epoch is not self.epoch or self.phase != phase):
            raise ScopeError('Inactive or stale run capability')

    def begin_run(self, epoch, context, origin, identity):
        """Admit an explicitly created, fresh context before creating its pages."""
        with self.__lock:
            if epoch is not self.epoch or self.phase != Phase.IDLE:
                raise ScopeError('Browser owner cannot admit a run')
            if not isinstance(context, str) or not context or context in self.__contexts:
                raise ScopeError('A fresh explicit browser context is required')
            origin = _origin(origin)
            self.__generation += 1
            lease = RunLease(self.epoch, self.__generation)
            self.__lease, self.__context = lease, context
            self.__origin, self.__identity = origin, identity
            self.__contexts.add(context)
            self.__disposed = False
            self.phase = Phase.ACTIVE
            return lease

    def bind_session(self, lease, session, target_info):
        with self.__lock:
            self.__require(lease, Phase.ACTIVE)
            if (not isinstance(session, str) or not session or session in self.__sessions_seen
                    or target_info.get('browserContextId') != self.__context
                    or not target_info.get('targetId')):
                raise ScopeError('Target session is not attributable to this run')
            binding = SessionBinding(lease, session, target_info['targetId'], self.__context,
                                     self.__origin, self.__identity)
            self.__bindings[session] = binding
            self.__sessions_seen.add(session)
            return binding

    def command_started(self, lease, *, draining=False):
        with self.__lock:
            self.__require(lease, Phase.DRAINING if draining else Phase.ACTIVE)
            token = object()
            self.__commands[token] = lease
            return token

    def command_finished(self, token, *, failed=False):
        with self.__lock:
            if token not in self.__commands:
                raise ScopeError('Unknown command receipt')
            del self.__commands[token]
            if failed:
                self.phase = Phase.POISONED

    def begin_draining(self, lease):
        with self.__lock:
            self.__require(lease, Phase.ACTIVE)
            self.phase = Phase.DRAINING
            return self.__context

    def context_disposed(self, lease):
        with self.__lock:
            self.__require(lease, Phase.DRAINING)
            self.__disposed = True

    def session_detached(self, session):
        with self.__lock:
            binding = self.__bindings.pop(session, None)
            if binding is not None:
                self.__authenticated = {key for key in self.__authenticated if key[0] is not binding}

    def finish_draining(self, lease):
        with self.__lock:
            self.__require(lease, Phase.DRAINING)
            if not self.__disposed or self.__bindings or self.__commands:
                raise ScopeError('Run draining barrier is incomplete')
            self.__authenticated.clear()
            self.__lease = self.__context = self.__origin = self.__identity = None
            self.phase = Phase.IDLE

    def poison(self):
        with self.__lock:
            if self.phase != Phase.CLOSED:
                self.phase = Phase.POISONED

    def close(self, epoch):
        """Revoke policy after the caller has stopped/joined its event reader."""
        with self.__lock:
            if epoch is not self.epoch:
                raise ScopeError('Foreign browser epoch')
            self.phase = Phase.CLOSED
            self.__bindings.clear()
            self.__commands.clear()
            self.__authenticated.clear()
            self.__lease = self.__context = self.__origin = self.__identity = None

    def dispatch_paused(self, session, params, send):
        """Check and synchronously send under the same revocation lock.

        The supplied send must write to the sole CDP transport before returning;
        enqueueing an effect for a later callback would break this boundary.
        """
        with self.__lock:
            decision = self.__paused(session, params)
            send(decision.method, decision.params, session)

    def __paused(self, session, params):
        with self.__lock:
            request_id = params['requestId']
            deny = FetchDecision('Fetch.failRequest', {'requestId': request_id, 'errorReason': 'BlockedByClient'})
            binding = self.__bindings.get(session)
            if (self.phase != Phase.ACTIVE or binding is None
                    or binding.lease is not self.__lease):
                return deny
            key = (binding, 'request', request_id)
            network = (binding, 'network', params.get('networkId'))
            redirected = (binding, 'request', params.get('redirectedRequestId'))
            if 'responseStatusCode' in params or 'responseErrorReason' in params:
                authenticated = key in self.__authenticated or network in self.__authenticated
                self.__authenticated.discard(key)
                self.__authenticated.discard(network)
                if authenticated and params.get('responseStatusCode') in (301, 302, 303, 307, 308):
                    return deny
                return FetchDecision('Fetch.continueRequest', {'requestId': request_id})
            if redirected in self.__authenticated or (params.get('redirectedRequestId') and network in self.__authenticated):
                return deny
            request = params['request']
            headers = [{'name': name, 'value': str(value)} for name, value in request.get('headers', {}).items()
                       if name.lower() != 'x-ezerbot-launch']
            try:
                same_origin = _origin(request['url']) == binding.origin
            except ScopeError:
                same_origin = False
            if same_origin:
                values = binding.identity.headers(binding.origin)
                reserved = {name.lower() for name in values}
                headers = [header for header in headers if header['name'].lower() not in reserved]
                headers.extend({'name': name, 'value': value} for name, value in values.items())
                self.__authenticated.add(key)
                if params.get('networkId'):
                    self.__authenticated.add(network)
            return FetchDecision('Fetch.continueRequest', {'requestId': request_id, 'headers': headers})
