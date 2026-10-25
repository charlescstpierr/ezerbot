import contextlib
import importlib.util
import io
import json
from pathlib import Path
import secrets
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch


SOURCE = Path(__file__).with_name("proof_chatgpt_mac.py")


class ProofChecks(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SOURCE.is_file(), "Real ChatGPT proof and its assertions must be restored")
        spec = importlib.util.spec_from_file_location("proof_chatgpt_mac", SOURCE)
        self.proof = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.proof)
        self.directory = tempfile.TemporaryDirectory(prefix="chatgpt-proof-checks-")
        self.addCleanup(self.directory.cleanup)
        self.home = Path(self.directory.name)

    def test_journal_requires_every_original_event(self):
        events = ["OpenAI request model=gpt-6.1-sol effort=high",
                  "status=200 code=completed", "status=200 code=token_refresh_saved"]
        journal = self.home / "engine.log"
        journal.write_text("\n".join(events))
        self.proof.journal_ok(journal)
        for missing in range(len(events)):
            journal.write_text("\n".join(event for index, event in enumerate(events) if index != missing))
            with self.assertRaises(self.proof.ProofError):
                self.proof.journal_ok(journal)

    def test_secret_rejection_never_prints_values(self):
        secret = "eyJ" + secrets.token_urlsafe(24)
        journal = self.home / "engine.log"
        journal.write_text(secret)
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            try:
                self.proof.journal_ok(journal)
            except self.proof.ProofError as error:
                output.write(str(error))
            else:
                self.fail("Credential-bearing journal accepted")
        self.assertTrue(secret not in output.getvalue(), "Rejection disclosed a credential")
        for text in ("Bearer " + secrets.token_urlsafe(24), "refresh_token", "access_token", "id_token"):
            with self.assertRaises(self.proof.ProofError):
                self.proof.forbid_secrets(text)

    def test_signout_requires_all_tokens_purged_and_private_file(self):
        auth = self.home / "auth.json"
        auth.write_text(json.dumps(dict.fromkeys(self.proof.TOKEN_KEYS)))
        auth.chmod(0o600)
        self.proof.tokens_absent(auth)
        for key in self.proof.TOKEN_KEYS:
            auth.write_text(json.dumps({key: secrets.token_urlsafe(24)}))
            with self.assertRaises(self.proof.ProofError):
                self.proof.tokens_absent(auth)
        auth.write_text("{}")
        auth.chmod(0o644)
        with self.assertRaises(self.proof.ProofError):
            self.proof.tokens_absent(auth)

    def test_reply_and_quota_keep_original_contract(self):
        reply = {"id": "reply-preuve-1", "state": "echoed", "body": "1. Sept.\n2. Sept.\n3. Sept."}
        state = {"conversations": [{"messages": [reply]}]}
        self.assertTrue(self.proof.reply_complete(state))
        reply["state"] = "streaming"
        self.assertFalse(self.proof.reply_complete(state))
        reply["state"] = "echoed"
        reply["body"] = "1. Sept."
        with self.assertRaises(self.proof.ProofError):
            self.proof.reply_complete(state)
        reply["body"] = "1. Sept. 2. Sept. 3. Sept."
        reply["error"] = "rejected"
        with self.assertRaises(self.proof.ProofError):
            self.proof.reply_complete(state)
        self.assertTrue(self.proof.quota_ready({"codex": {"status": "ready", "quota": {"primary": {"used_percent": 3}}}}))
        for percent in (-1, 101, "3", True):
            with self.assertRaises(self.proof.ProofError):
                self.proof.quota_ready({"codex": {"status": "ready", "quota": {"primary": {"used_percent": percent}}}})
        with self.assertRaises(self.proof.ProofError):
            self.proof.quota_ready({"codex": {"status": "ready", "quota": {}}})
        self.assertFalse(self.proof.quota_ready({"codex": {"status": "loading"}}))

    def test_refresh_needs_new_grant_and_new_journal_event(self):
        before = {"access_token": secrets.token_urlsafe(24), "refresh_token": secrets.token_urlsafe(24)}
        after = {"access_token": secrets.token_urlsafe(24), "refresh_token": secrets.token_urlsafe(24)}
        self.assertTrue(self.proof.refresh_saved(before, after, "status=200 code=token_refresh_saved"))
        self.assertFalse(self.proof.refresh_saved(before, before, "status=200 code=token_refresh_saved"))
        self.assertFalse(self.proof.refresh_saved(before, after, "status=200 code=completed"))
        self.assertFalse(self.proof.refresh_saved(before, {}, "status=200 code=token_refresh_saved"))

    def scenario(self, fault=None):
        home = Path(tempfile.mkdtemp(dir=self.home))
        support = home / "Library/Application Support/Grokbot"
        (support / "logs").mkdir(parents=True)
        auth = support / "workspace.codex-auth.json"
        grant = {key: secrets.token_urlsafe(24) for key in self.proof.TOKEN_KEYS}
        auth.write_text(json.dumps(grant))
        auth.chmod(0o600)
        journal = support / "logs/engine.log"
        journal.write_text("status=200 code=token_refresh_saved\n")
        shell_log = home / "shell.err"
        shell_log.write_text("")
        calls = []
        signed = True
        sent = False
        checked = []

        def call(method, path, body):
            nonlocal signed, sent
            calls.append((method, path))
            if path == "/api/state":
                messages = [{"id": "reply-preuve-1", "state": "echoed", "body": "1. Sept. 2. Sept. 3. Sept."}] if sent else []
                return 200, {"mode": "simulated" if fault == "simulated" else "chatgpt",
                             "provider": {"signed_in": signed, "connected": signed},
                             "conversations": [{"messages": messages}]}
            if path == "/api/preferences":
                return 200, {"preferences": body if fault != "model" else {"model": "other", "effort": "high"}}
            if path == "/api/conversations":
                return 200, {"conversation": {"id": "test-conversation"}}
            if path == "/api/send":
                sent = True
                with journal.open("a") as stream:
                    stream.write("OpenAI request model=gpt-6.1-sol effort=high\nstatus=200 code=completed\n")
                return (503 if fault == "http" else 200), {}
            if path == "/api/provider/test":
                if fault != "refresh":
                    auth.write_text(json.dumps({key: secrets.token_urlsafe(24) for key in self.proof.TOKEN_KEYS}))
                if fault != "stale-log":
                    with journal.open("a") as stream:
                        stream.write("status=200 code=token_refresh_saved\n")
                return 200, {}
            if path == "/api/usage":
                return 200, {"codex": {"status": "ready", "quota": {"primary": {"used_percent": 2}}}}
            if path == "/api/provider/signout":
                signed = fault == "signed"
                auth.write_text(json.dumps({} if fault != "purge" else grant))
                if fault == "journal":
                    with journal.open("a") as stream:
                        stream.write(grant["access_token"])
                if fault == "shell":
                    shell_log.write_text(grant["refresh_token"])
                return 200, {}
            self.fail("Unexpected proof endpoint")

        self.proof.Proof(call, support, shell_log, lambda: checked.append(True),
                         sleep=lambda _: None, note=lambda _: None).run(reuse=True)
        return calls, checked

    def test_full_scenario_checks_refresh_purge_journals_and_all_calls(self):
        calls, checked = self.scenario()
        self.assertTrue(checked == [True])
        self.assertTrue(set(calls) == {("GET", "/api/state"), ("GET", "/api/usage"),
                                      ("POST", "/api/preferences"), ("POST", "/api/conversations"),
                                      ("POST", "/api/send"), ("POST", "/api/provider/test"),
                                      ("POST", "/api/provider/signout")})

    def test_full_scenario_fails_if_refresh_does_not_happen(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("refresh")

    def test_old_refresh_log_cannot_prove_new_refresh(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("stale-log")

    def test_full_scenario_fails_if_grants_remain_after_signout(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("purge")

    def test_full_scenario_fails_if_engine_journal_discloses_a_grant(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("journal")

    def test_full_scenario_fails_if_shell_journal_discloses_a_grant(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("shell")

    def test_full_scenario_requires_signed_out_state(self):
        with self.assertRaises(self.proof.ProofError):
            self.scenario("signed")

    def test_full_scenario_requires_selected_model_and_successful_send(self):
        for fault in ("model", "http", "simulated"):
            with self.subTest(fault=fault):
                with self.assertRaises(self.proof.ProofError):
                    self.scenario(fault)

    def test_incompatible_modes_and_fake_real_mode_fail_before_launch(self):
        for arguments in (("--self-check", "--keep"), ("--guard-calls", "--keep"), ("--sphere", "accueil")):
            process = subprocess.run(["bash", str(SOURCE.with_name("preuve-chatgpt-mac.sh")), *arguments],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            self.assertTrue(process.returncode == 2)
            self.assertTrue("preuve ok" not in process.stdout)
        with patch.dict("os.environ", {"GROKBOT_FAKE_OPENAI": "http://127.0.0.1:1"}):
            with patch.object(self.proof.sys, "platform", "darwin"):
                with self.assertRaises(self.proof.ProofError):
                    self.proof.operator_proof(False)

    def test_operator_keep_failure_cleanup_and_session_reuse(self):
        wrapper = self.home / "wrapper"
        wrapper.write_bytes(b"unused")
        for keep, fail in ((False, False), (True, False), (False, True)):
            with self.subTest(keep=keep, fail=fail):
                scratch = Path(tempfile.mkdtemp(dir=self.home))
                work = scratch / "ezerbot-preuve-session"
                support = work / "Library/Application Support/Grokbot"
                support.mkdir(parents=True)
                work.chmod(0o700)
                auth = support / "workspace.codex-auth.json"
                auth.write_text(json.dumps({"access_token": secrets.token_urlsafe(24)}))
                native = Mock()
                scenario = Mock()
                if fail:
                    scenario.run.side_effect = self.proof.ProofError("Failed proof")
                with patch.dict("os.environ", {"EZERBOT_WRAPPER": str(wrapper), "GROKBOT_FAKE_OPENAI": ""}):
                    with patch.object(self.proof.sys, "platform", "darwin"), \
                         patch.object(self.proof.tempfile, "gettempdir", return_value=str(scratch)), \
                         patch.object(self.proof, "compile_host", return_value=wrapper), \
                         patch.object(self.proof, "NativeHost", return_value=native), \
                         patch.object(self.proof, "Proof", return_value=scenario), \
                         contextlib.redirect_stdout(io.StringIO()):
                        if fail:
                            with self.assertRaises(self.proof.ProofError):
                                self.proof.operator_proof(keep)
                        else:
                            self.proof.operator_proof(keep)
                self.assertTrue(scenario.run.call_args.args == (True,))
                self.assertTrue(native.close.call_count == 1)
                self.assertTrue(work.exists() == (keep or fail))


if __name__ == "__main__":
    unittest.main()
