"""Inspect state through the private channel of the bundle being verified."""

from __future__ import annotations

import ctypes
from dataclasses import dataclass, field
import errno
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time


MAX_MESSAGE_BYTES = 4096
ERROR_CODES = frozenset({"inspection_failed", "invalid_command", "state_failed", "stale_runtime"})


class BundleInspectionError(RuntimeError):
    pass


class _DarwinBSDInfo(ctypes.Structure):
    _fields_ = [
        ("flags", ctypes.c_uint32), ("status", ctypes.c_uint32),
        ("exit_status", ctypes.c_uint32), ("pid", ctypes.c_uint32),
        ("parent_pid", ctypes.c_uint32), ("credentials", ctypes.c_uint32 * 7),
        ("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32),
        ("scheduling", ctypes.c_uint32 * 6),
        ("start_seconds", ctypes.c_uint64), ("start_microseconds", ctypes.c_uint64),
    ]


def _process_info(pid):
    try:
        if sys.platform == "darwin":
            library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
            read_info = library.proc_pidinfo
            read_info.argtypes = (ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                  ctypes.c_void_p, ctypes.c_int)
            read_info.restype = ctypes.c_int
            info = _DarwinBSDInfo()
            ctypes.set_errno(0)
            size = read_info(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info))
            if size == 0 and ctypes.get_errno() == errno.ESRCH:
                return None
            if size != ctypes.sizeof(info) or info.pid != pid:
                raise ValueError()
            if info.status == 5:
                return None
            return info.parent_pid, (info.start_seconds, info.start_microseconds)
        if sys.platform.startswith("linux"):
            fields = (Path("/proc") / str(pid) / "stat").read_bytes().rsplit(b")", 1)[1].split()
            if fields[0] == b"Z":
                return None
            return int(fields[1]), (int(fields[19]), 0)
        raise ValueError()
    except FileNotFoundError:
        return None
    except (OSError, ValueError, IndexError):
        raise BundleInspectionError("Bundle inspection process identity unavailable") from None


def _process_executable(pid):
    if sys.platform == "darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        read_path = library.proc_pidpath
        read_path.argtypes = (ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32)
        read_path.restype = ctypes.c_int
        path = ctypes.create_string_buffer(4096)
        return path.value if read_path(pid, path, ctypes.sizeof(path)) > 0 else b""
    return os.fsencode(os.readlink(Path("/proc") / str(pid) / "exe"))


def _process_session(pid):
    try:
        return os.getsid(pid)
    except ProcessLookupError:
        return None
    except OSError:
        raise BundleInspectionError("Bundle inspection process identity unavailable") from None


def _process_pids():
    try:
        result = subprocess.run(
            ["/bin/ps", "-A", "-o", "pid="],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}, timeout=1, check=False,
        )
        pids = result.stdout.split()
        if result.returncode != 0 or any(not pid.isdigit() for pid in pids):
            raise ValueError()
        return tuple(int(pid) for pid in pids if int(pid) > 0)
    except (OSError, ValueError, subprocess.SubprocessError):
        raise BundleInspectionError("Bundle inspection process identity unavailable") from None


@dataclass(frozen=True)
class _ProcessIdentity:
    pid: int
    started: tuple[int, int]
    command: bytes = field(repr=False)
    executable: bytes = field(repr=False)
    parent_pid: int = field(compare=False)
    session_id: int

    @classmethod
    def read(cls, pid, *, session_id):
        session = _process_session(pid)
        if session is None or session != session_id:
            return None
        before = _process_info(pid)
        if before is None or _process_session(pid) != session:
            return None
        try:
            if sys.platform == "darwin":
                result = subprocess.run(
                    ["/bin/ps", "-ww", "-p", str(pid), "-o", "command="],
                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                    env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}, timeout=1, check=False,
                )
                command = result.stdout.rstrip(b"\n") if result.returncode == 0 else b""
            else:
                command = (Path("/proc") / str(pid) / "cmdline").read_bytes()
            if _process_session(pid) != session:
                return None
            executable = _process_executable(pid)
        except FileNotFoundError:
            return None
        except (OSError, subprocess.SubprocessError):
            raise BundleInspectionError("Bundle inspection process identity unavailable") from None
        after = _process_info(pid)
        if after is None or before[1] != after[1] or _process_session(pid) != session:
            return None
        if not command or not executable:
            raise BundleInspectionError("Bundle inspection process identity unavailable")
        return cls(pid, after[1], command, executable, after[0], session)

    def signal(self, number):
        if self.read(self.pid, session_id=self.session_id) != self:
            return
        try:
            os.kill(self.pid, number)
        except ProcessLookupError:
            pass
        except OSError:
            raise BundleInspectionError("Bundle inspection process cleanup failed") from None

    def wait_until_absent(self, timeout):
        deadline = time.monotonic() + timeout
        while self.read(self.pid, session_id=self.session_id) == self:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return False
            time.sleep(min(0.01, remaining))
        return True


@dataclass(frozen=True)
class _Runtime:
    host_pid: int
    engine_pid: int
    url: str

    @classmethod
    def parse(cls, message):
        host = message.get("host_pid")
        engine = message.get("engine_pid")
        url = message.get("url")
        if (type(host) is not int or host <= 0 or type(engine) is not int or engine <= 0
                or engine == host or type(url) is not str):
            raise BundleInspectionError("Invalid bundle inspection runtime")
        origin = re.fullmatch(r"http://127\.0\.0\.1:([1-9][0-9]{0,4})", url)
        if not origin or int(origin[1]) > 65535:
            raise BundleInspectionError("Invalid bundle inspection runtime")
        return cls(host, engine, url)

    def message(self):
        return {"host_pid": self.host_pid, "engine_pid": self.engine_pid, "url": self.url}


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError()
        result[key] = value
    return result


def _reject_constant(_value):
    raise ValueError()


class BundleInspection:
    """Own one bundle process, its private HOME and its fixed state command."""

    def __init__(self, app, evidence, *, timeout=15, startup_timeout=240, shutdown_timeout=10):
        self.app = Path(app).resolve()
        self.evidence = Path(evidence).resolve()
        self.timeout = timeout
        self.startup_timeout = startup_timeout
        self.shutdown_timeout = shutdown_timeout
        self.process = None
        self.home = None
        self._directory = None
        self._stderr = None
        self._runtime = None
        self._host = None
        self._engines = set()
        self._session_id = None
        self._channel_failed = False
        self._closed = False

    @property
    def pid(self):
        if self.process is None:
            raise BundleInspectionError("Bundle inspection is not running")
        return self.process.pid

    @property
    def ready(self):
        if self._runtime is None:
            raise BundleInspectionError("Bundle inspection is not ready")
        return {"ready": True, **self._runtime.message()}

    def __enter__(self):
        if self.process is not None or self._closed:
            raise BundleInspectionError("Bundle inspection cannot be restarted")
        try:
            self.evidence.mkdir(parents=True, exist_ok=True)
            self._directory = tempfile.TemporaryDirectory(prefix="bundle-inspection-", dir=self.evidence)
            self.home = Path(self._directory.name).resolve()
            self.home.chmod(0o700)
            descriptor = os.open(self.home / "shell.err", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
            self._stderr = os.fdopen(descriptor, "wb", buffering=0)
            env = {name: os.environ[name] for name in ("PATH", "LANG") if name in os.environ}
            env.update(HOME=str(self.home), CFFIXED_USER_HOME=str(self.home),
                       EZERBOT_TEST_HOME=str(self.home), TMPDIR=str(self.home))
            self.process = subprocess.Popen(
                [str(self.app / "Contents/MacOS/Ezerbot"), "--inspect-private"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self._stderr,
                env=env, close_fds=True, bufsize=0, start_new_session=True,
            )
            self._session_id = self.process.pid
            os.set_blocking(self.process.stdout.fileno(), False)
            message = self._read_message(self.startup_timeout)
            if set(message) != {"ready", "host_pid", "engine_pid", "url"} or message["ready"] is not True:
                raise BundleInspectionError("Invalid bundle inspection readiness")
            runtime = _Runtime.parse(message)
            if runtime.host_pid != self.process.pid:
                raise BundleInspectionError("Bundle inspection host does not match launched process")
            self._host = _ProcessIdentity.read(self.process.pid, session_id=self._session_id)
            if self._host is None or self._host.parent_pid != os.getpid():
                raise BundleInspectionError("Bundle inspection process stopped")
            engine = _ProcessIdentity.read(runtime.engine_pid, session_id=self._session_id)
            if (engine is None or engine.parent_pid != self.process.pid
                    or self.process.poll() is not None):
                raise BundleInspectionError("Bundle inspection engine is not owned by launched process")
            self._engines.add(engine)
            self._runtime = runtime
            self._ensure_quiet()
            return self
        except BaseException as error:
            self.close()
            if isinstance(error, (KeyboardInterrupt, SystemExit)):
                raise
            if isinstance(error, BundleInspectionError):
                raise error from None
            raise BundleInspectionError("Bundle inspection could not start") from None

    def _ensure_quiet(self):
        if self.process.poll() is not None:
            raise BundleInspectionError("Bundle inspection process stopped")
        try:
            extra = os.read(self.process.stdout.fileno(), MAX_MESSAGE_BYTES + 1)
        except BlockingIOError:
            return
        except OSError:
            raise BundleInspectionError("Bundle inspection channel failed") from None
        if extra:
            raise BundleInspectionError("Unexpected bundle inspection output")
        raise BundleInspectionError("Bundle inspection channel closed")

    def _read_message(self, timeout):
        data = bytearray()
        deadline = time.monotonic() + timeout
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(self.process.stdout, selectors.EVENT_READ)
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0 or not selector.select(remaining):
                        raise BundleInspectionError("Bundle inspection response timed out")
                    try:
                        chunk = os.read(self.process.stdout.fileno(), MAX_MESSAGE_BYTES + 1 - len(data))
                    except BlockingIOError:
                        continue
                    if not chunk:
                        raise BundleInspectionError("Bundle inspection channel closed")
                    data.extend(chunk)
                    if len(data) > MAX_MESSAGE_BYTES:
                        raise BundleInspectionError("Bundle inspection response is too large")
                    if b"\n" in data:
                        if data[-1:] != b"\n" or data.count(b"\n") != 1:
                            raise BundleInspectionError("Unexpected bundle inspection output")
                        break
            message = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                                 parse_constant=_reject_constant)
        except BundleInspectionError:
            raise
        except (OSError, UnicodeError, ValueError, RecursionError):
            raise BundleInspectionError("Invalid bundle inspection response") from None
        if type(message) is not dict:
            raise BundleInspectionError("Invalid bundle inspection response")
        if "error" in message:
            if set(message) != {"error"} or type(message["error"]) is not str or message["error"] not in ERROR_CODES:
                raise BundleInspectionError("Invalid bundle inspection response")
            raise BundleInspectionError("Bundle inspection state unavailable")
        return message

    def state(self, engine_pid, url):
        if self._closed or self._runtime is None or self._channel_failed:
            raise BundleInspectionError("Bundle inspection is not ready")
        if (type(engine_pid) is not int or engine_pid != self._runtime.engine_pid
                or type(url) is not str or url != self._runtime.url):
            raise BundleInspectionError("Bundle inspection target changed")
        self._channel_failed = True
        self._ensure_quiet()
        try:
            self.process.stdin.write(b"state\n")
            self.process.stdin.flush()
        except (OSError, ValueError):
            raise BundleInspectionError("Bundle inspection command failed") from None
        message = self._read_message(self.timeout)
        if set(message) != {"host_pid", "engine_pid", "url", "status", "mode", "journals_safe"}:
            raise BundleInspectionError("Invalid bundle inspection state")
        if _Runtime.parse(message) != self._runtime:
            raise BundleInspectionError("Bundle inspection runtime changed")
        if type(message["status"]) is not int or message["status"] != 200:
            raise BundleInspectionError("Bundle inspection state was not successful")
        if type(message["mode"]) is not str or message["mode"] not in ("simulated", "chatgpt"):
            raise BundleInspectionError("Bundle inspection state has an unknown mode")
        if message["journals_safe"] is not True:
            raise BundleInspectionError("Bundle inspection journals were not verified")
        self._ensure_quiet()
        self._channel_failed = False
        return {**self._runtime.message(), "status": 200, "mode": message["mode"], "journals_safe": True}

    def _close_host(self):
        failed = False
        if self._host is None and self.process.poll() is None:
            try:
                host = _ProcessIdentity.read(self.process.pid, session_id=self._session_id)
                if host is not None and host.parent_pid == os.getpid():
                    self._host = host
            except BundleInspectionError:
                failed = True
        if self.process.stdin is not None:
            try:
                self.process.stdin.close()
            except (OSError, ValueError):
                failed = True
        try:
            self.process.wait(timeout=self.shutdown_timeout)
        except subprocess.TimeoutExpired:
            failed = True
            for number in (signal.SIGTERM, signal.SIGKILL):
                try:
                    self._remember_session_engines()
                except BundleInspectionError:
                    failed = True
                try:
                    if self._host is not None:
                        self._host.signal(number)
                    self.process.wait(timeout=self.shutdown_timeout)
                    break
                except (BundleInspectionError, OSError, subprocess.TimeoutExpired):
                    continue
        except OSError:
            failed = True
        return failed or self.process.returncode != 0

    def _remember_session_engines(self):
        if self._session_id is None:
            return
        executable = os.fsencode((self.app / "Contents/MacOS/grokbot-wrapper").resolve())
        remembered = {engine.pid for engine in self._engines}
        for pid in _process_pids():
            if pid in remembered or _process_session(pid) != self._session_id:
                continue
            engine = _ProcessIdentity.read(pid, session_id=self._session_id)
            if engine is not None and engine.executable == executable:
                self._engines.add(engine)

    def _close_engine(self, engine):
        try:
            current = engine.read(engine.pid, session_id=self._session_id)
            if current is None:
                return False
            if current != engine:
                return True
        except BundleInspectionError:
            pass
        for number in (signal.SIGTERM, signal.SIGKILL):
            try:
                engine.signal(number)
                if engine.wait_until_absent(self.shutdown_timeout):
                    break
            except BundleInspectionError:
                continue
        return True

    def close(self):
        if self._closed:
            return
        self._closed = True
        failed = False
        try:
            self._remember_session_engines()
        except BundleInspectionError:
            failed = True
        if self.process is not None:
            failed = self._close_host() or failed
        try:
            self._remember_session_engines()
        except BundleInspectionError:
            failed = True
        for engine in self._engines:
            failed = self._close_engine(engine) or failed
        for stream in (self.process.stdout if self.process is not None else None, self._stderr):
            if stream is not None:
                try:
                    stream.close()
                except (OSError, ValueError):
                    failed = True
        if self._directory is not None:
            try:
                self._directory.cleanup()
            except OSError:
                failed = True
        if failed:
            raise BundleInspectionError("Bundle inspection did not close normally")

    def __exit__(self, *_exception):
        self.close()
