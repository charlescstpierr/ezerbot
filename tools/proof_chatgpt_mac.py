"""Operator-only ChatGPT proof through the real Cocoa host's private transport."""

import argparse
import json
import math
import os
from pathlib import Path
import re
import selectors
import shutil
import stat
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
TOKEN_KEYS = ("access_token", "refresh_token", "id_token")
SENSITIVE = re.compile(r"eyJ[A-Za-z0-9_-]{8,}|Bearer\s+\S+|refresh_token|access_token|id_token", re.I)
EVENTS = (r"status=200 code=completed\b", r"OpenAI request model=gpt-6\.1-sol effort=high\b",
          r"status=200 code=token_refresh_saved\b")


class ProofError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise ProofError(message)


def forbid_secrets(text, credentials=()):
    require(not SENSITIVE.search(text) and not any(value and value in text for value in credentials),
            "Contenu refusé. Un secret a été détecté, sa valeur reste masquée.")


def journal_ok(path, credentials=()):
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    forbid_secrets(text, credentials)
    require(all(re.search(pattern, text) for pattern in EVENTS),
            "Journal incomplet. Réponse, modèle/effort ou renouvellement non confirmé.")


def read_grant(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    require(isinstance(data, dict), "Session de test illisible.")
    return {key: data.get(key) for key in TOKEN_KEYS}


def tokens_absent(path):
    data = read_grant(path)
    require(all(data.get(key) in (None, "") for key in TOKEN_KEYS),
            "Des jetons restent dans la copie après déconnexion.")
    require(stat.S_IMODE(Path(path).stat().st_mode) == 0o600, "La session de test doit rester privée, mode 0600.")


def session_has_grant(path):
    try:
        data = read_grant(path)
        return any(isinstance(data.get(key), str) and data[key] for key in TOKEN_KEYS[:2])
    except (OSError, ValueError, ProofError):
        return False


def reply_complete(state):
    for conversation in state.get("conversations") or []:
        for message in conversation.get("messages") or []:
            if message.get("id") != "reply-preuve-1":
                continue
            if message.get("state") == "streaming":
                return False
            require(message.get("state") == "echoed" and not message.get("error") and
                    all(part in (message.get("body") or "") for part in ("1. Sept.", "2. Sept.", "3. Sept.")),
                    "Réponse incomplète ou refusée.")
            return True
    return False


def quota_ready(data):
    codex = data.get("codex") or {}
    if codex.get("status") != "ready":
        return False
    quota = codex.get("quota") or {}
    percentages = [window.get("used_percent") for window in (quota.get("primary"), quota.get("secondary"))
                   if isinstance(window, dict)]
    require(bool(percentages) and all(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 100
                                     for value in percentages), "Quota WHAM invalide.")
    return True


def refresh_saved(before, after, new_log):
    return (bool(re.search(EVENTS[2], new_log)) and
            any(isinstance(after.get(key), str) and after[key] and after[key] != before.get(key)
                for key in TOKEN_KEYS[:2]))


class Proof:
    def __init__(self, call, support, shell_log, check_journals, sleep=time.sleep, note=print):
        self.call = call
        self.support = Path(support)
        self.shell_log = Path(shell_log)
        self.check_journals = check_journals
        self.sleep = sleep
        self.note = note
        self.credentials = set()

    @property
    def auth_file(self):
        return self.support / "workspace.codex-auth.json"

    @property
    def journal(self):
        return self.support / "logs/engine.log"

    def request(self, method, path, body=None):
        status, data = self.call(method, path, body)
        forbid_secrets(json.dumps(data), self.credentials)
        require(status == 200, "Appel local refusé pendant la preuve.")
        require(isinstance(data, dict), "Réponse locale illisible.")
        return data

    def wait(self, attempts, interval, predicate, failure):
        for attempt in range(attempts):
            if predicate():
                return
            if attempt + 1 < attempts:
                self.sleep(interval)
        raise ProofError(failure)

    def remember_grant(self):
        data = read_grant(self.auth_file)
        self.credentials.update(value for value in data.values() if isinstance(value, str) and value)
        return data

    def run(self, reuse=False):
        self.note("Session de test réutilisée." if reuse else
                  "Cliquez une seule fois sur Se connecter dans la fenêtre de cette copie.")
        self.note("En attente de la session et d'une sonde réussie, dix minutes au maximum.")
        attempts = 0

        def connected():
            nonlocal attempts
            attempts += 1
            state = self.request("GET", "/api/state")
            provider = state.get("provider") or {}
            if reuse and attempts == 10 and provider.get("signed_in") is not True:
                self.note("Cette session ne répond plus. Cliquez une seule fois sur Se connecter.")
            ready = provider.get("signed_in") is True and provider.get("connected") is True
            if ready:
                require(state.get("mode") == "chatgpt", "Le parcours réel refuse une session simulée.")
            return ready

        self.wait(300, 2, connected, "Connexion non confirmée.")
        self.remember_grant()
        preferences = self.request("POST", "/api/preferences", {"model": "gpt-6.1-sol", "effort": "high"})
        require(preferences.get("preferences", {}).get("model") == "gpt-6.1-sol" and
                preferences.get("preferences", {}).get("effort") == "high",
                "GPT-6.1 Sol ou l'effort high n'est pas retenu par le compte.")
        conversation = self.request("POST", "/api/conversations", {}).get("conversation", {}).get("id")
        require(isinstance(conversation, str) and bool(conversation), "Conversation sans identifiant.")
        self.request("POST", "/api/send", {"conversation_id": conversation, "message_id": "preuve-1",
                     "body": "Réponds uniquement par trois lignes, exactement : 1. Sept. 2. Sept. 3. Sept."})
        self.wait(90, 2, lambda: reply_complete(self.request("GET", "/api/state")), "Réponse incomplète ou refusée.")
        before = self.remember_grant()
        log_offset = self.journal.stat().st_size
        self.request("POST", "/api/provider/test", {"refresh": True})

        def refreshed():
            provider = self.request("GET", "/api/state").get("provider") or {}
            after = self.remember_grant()
            with self.journal.open("rb") as stream:
                stream.seek(log_offset)
                new_log = stream.read().decode("utf-8", errors="replace")
            forbid_secrets(new_log, self.credentials)
            if provider.get("connected") is not True or not refresh_saved(before, after, new_log):
                return False
            journal_ok(self.journal, self.credentials)
            return True

        self.wait(30, 1, refreshed, "Le renouvellement effectif du jeton ou le journal n'est pas confirmé.")

        def quota():
            if quota_ready(self.request("GET", "/api/usage")):
                return True
            self.request("POST", "/api/usage/refresh", {})
            return False

        self.wait(20, 1, quota, "Quota WHAM indisponible.")
        self.request("POST", "/api/provider/signout", {})
        provider = self.request("GET", "/api/state").get("provider") or {}
        require(provider.get("signed_in") is False, "La session reste affichée connectée.")
        require(provider.get("connected") is False, "La sonde reste affichée connectée.")
        tokens_absent(self.auth_file)
        journal_ok(self.journal, self.credentials)
        forbid_secrets(self.shell_log.read_text(encoding="utf-8", errors="replace"), self.credentials)
        self.check_journals()


def compile_host(work, wrapper):
    host = work / "host/Ezerbot"
    host.parent.mkdir(parents=True, exist_ok=True)
    frameworks = ("Cocoa", "WebKit", "UniformTypeIdentifiers", "UserNotifications", "AVFoundation",
                  "CoreAudio", "AudioToolbox", "Speech", "ApplicationServices", "Security", "Carbon")
    command = ["/usr/bin/clang", "-fobjc-arc", "-Wall", "-Wextra", "-Werror", "-mmacosx-version-min=12.3"]
    for framework in frameworks:
        command.extend(["-framework", framework])
    command.extend([str(ROOT / "tools/proof-chatgpt-host.m"), "-o", str(host)])
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    shutil.copy2(wrapper, host.with_name("grokbot-wrapper"))
    return host


class NativeHost:
    def __init__(self, host, work):
        self.log = (work / "shell.err").open("w", encoding="utf-8")
        environment = {key: os.environ[key] for key in ("PATH", "TMPDIR", "LANG") if key in os.environ}
        environment.update(HOME=str(work), CFFIXED_USER_HOME=str(work), EZERBOT_TEST_HOME=str(work))
        self.process = subprocess.Popen([str(host)], env=environment, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.log, text=True, bufsize=1)
        try:
            require(self.receive(250).get("ready") is True, "La coque de preuve n'est pas prête.")
        except BaseException:
            self.close()
            raise

    def receive(self, seconds=35):
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            require(bool(selector.select(seconds)), "Le pilote natif ne répond pas.")
        line = self.process.stdout.readline(1024 * 1024 + 1)
        require(bool(line) and line.endswith("\n") and len(line) <= 1024 * 1024, "Réponse du pilote natif invalide.")
        forbid_secrets(line)
        result = json.loads(line)
        require(isinstance(result, dict) and "error" not in result, "Le pilote natif a refusé la commande.")
        return result

    def command(self, data):
        self.process.stdin.write(json.dumps(data, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        return self.receive()

    def call(self, method, path, body=None):
        result = self.command({"method": method, "path": path, "body": body})
        return result.get("status"), result.get("body")

    def check_journals(self):
        require(self.command({"command": "check-journals"}).get("ok") is True, "Le journal natif a échoué au contrôle de secrets.")

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=35)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                self.process.wait(timeout=35)
        self.process.stdout.close()
        self.log.close()
        require(self.process.returncode == 0, "Le pilote natif ne s'est pas arrêté proprement.")


def operator_proof(keep):
    require(sys.platform == "darwin", "La preuve réelle se lance sur le Mac du propriétaire.")
    require(not os.environ.get("GROKBOT_FAKE_OPENAI"), "Le mode réel refuse un faux fournisseur.")
    app = Path(os.environ.get("EZERBOT_APP", ROOT / "app/build/Ezerbot.app"))
    wrapper = Path(os.environ.get("EZERBOT_WRAPPER", app / "Contents/MacOS/grokbot-wrapper"))
    require(wrapper.is_file(), "Bundle absent. Construisez le wrapper via cargo-gate avant la preuve.")
    work = Path(tempfile.gettempdir()) / "ezerbot-preuve-session"
    require(not work.is_symlink(), "HOME de test refusé.")
    work.mkdir(mode=0o700, exist_ok=True)
    work = work.resolve()
    require(work != Path.home().resolve() and stat.S_IMODE(work.stat().st_mode) == 0o700 and
            work.stat().st_uid == os.getuid(), "HOME de test refusé.")
    support = work / "Library/Application Support/Grokbot"
    for path in (work / "host", support, work / "shell.err"):
        require(path.resolve().is_relative_to(work), "Chemin hors du HOME de test refusé.")
    host = None
    success = False
    try:
        binary = compile_host(work, wrapper)
        reuse = session_has_grant(support / "workspace.codex-auth.json")
        host = NativeHost(binary, work)
        Proof(host.call, support, work / "shell.err", host.check_journals).run(reuse)
        success = True
    finally:
        if host:
            host.close()
        if success and not keep:
            shutil.rmtree(work)
            print("copie jetable supprimée")
        else:
            print("Copie conservée pour la prochaine preuve. Elle peut contenir une session privée.")
            print(work)
    print("preuve ok : 200, réponse complète, gpt-6.1-sol, effort high, jeton rafraîchi, quota, déconnexion")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--self-check", action="store_true")
    modes.add_argument("--guard-calls", action="store_true")
    modes.add_argument("--keep", action="store_true")
    args = parser.parse_args(argv)
    if args.self_check:
        subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", str(ROOT / "tools"),
                        "-p", "test_proof_chatgpt_mac.py"], check=True)
    if args.self_check or args.guard_calls:
        subprocess.run([sys.executable, str(ROOT / "tools/verify-proof-guard.py"), "--guard-calls"], check=True)
        print("self-check ok, parcours réel non exécuté" if args.self_check else "guard-calls ok, parcours réel non exécuté")
    else:
        if sys.platform != "darwin":
            print("La preuve réelle se lance sur le Mac du propriétaire. Utilisez --self-check sans compte réel.", file=sys.stderr)
            raise SystemExit(2)
        operator_proof(args.keep)


if __name__ == "__main__":
    try:
        main()
    except ProofError as error:
        print(str(error), file=sys.stderr)
        print("Aucun succès du parcours réel n'est établi.", file=sys.stderr)
        raise SystemExit(1) from None
    except (Exception, KeyboardInterrupt):
        print("Preuve interrompue. Aucun succès du parcours réel n'est établi.", file=sys.stderr)
        raise SystemExit(1) from None
