"""Behavioral checks of the SQLite -> private JSON calibration report."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from token_estimate import build_report, request_tokens


class FakeEncoder:
    def encode(self, text, disallowed_special=()):
        return text.split()


class TokenEstimateReportTests(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.addCleanup(self.conn.close)
        self.conn.executescript("""
            CREATE TABLE engine_runs (run_id TEXT PRIMARY KEY, conversation_id TEXT);
            CREATE TABLE engine_response_usage (
                run_id TEXT, response INTEGER, input_tokens INTEGER,
                output_tokens INTEGER, cached_input_tokens INTEGER,
                context_characters INTEGER, characters_per_token INTEGER,
                tool_characters INTEGER, PRIMARY KEY (run_id, response)
            );
            CREATE TABLE engine_summaries (conversation_id TEXT, record TEXT);
        """)
        self.next_run = 0

    def response(self, conversation, inputs, characters, tools, *, per_token=4, run=None, response=1):
        if run is None:
            run = f"run-secret-{self.next_run}"
            self.next_run += 1
            self.conn.execute("INSERT INTO engine_runs VALUES (?, ?)", (run, conversation))
        self.conn.execute(
            "INSERT INTO engine_response_usage VALUES (?, ?, ?, 1, 0, ?, ?, ?)",
            (run, response, inputs, characters, per_token, tools),
        )
        return run

    def populate(self):
        for inputs, characters, tools in [(100, 400, 40), (100, 200, 40), (100, 500, 40)]:
            self.response("conversation-secret-a", inputs, characters, tools)
        self.response("conversation-secret-b", 200, 1000, 0)
        self.conn.execute(
            "INSERT INTO engine_response_usage VALUES ('run-secret-3', 2, 50, 1, 0, NULL, NULL, NULL)"
        )
        self.response("conversation-secret-b", 0, 80, 0)
        self.conn.execute("INSERT INTO engine_summaries VALUES ('conversation-secret-a', '{}')")
        self.conn.execute("INSERT INTO engine_summaries VALUES ('conversation-secret-a', NULL)")

    def test_signed_errors_are_exact_per_conversation_and_global(self):
        self.populate()
        report = build_report(self.conn)
        first, second = report["conversations"]
        self.assertEqual([first["conversation"], second["conversation"]], [1, 2])
        self.assertEqual(
            [r["signed_error"] for r in first["responses"]], [0.0, -0.5, 0.25]
        )
        self.assertEqual(
            [r["signed_error_with_tools"] for r in first["responses"]], [0.1, -0.4, 0.35]
        )
        self.assertEqual([r["tool_share"] for r in first["responses"]], [0.1, 0.1, 0.1])
        self.assertEqual([r["estimated_tokens"] for r in first["responses"]], [100.0, 50.0, 125.0])
        stats = first["signed_error"]
        self.assertEqual(stats["n"], 3)
        self.assertEqual(stats["mean"], -0.0833333)
        self.assertEqual(stats["median"], 0.0)
        self.assertAlmostEqual(stats["pstdev"], 0.311805, places=6)
        self.assertEqual((stats["min"], stats["max"], stats["worst"]), (-0.5, 0.25, -0.5))
        self.assertEqual(stats["underestimates"], 1)
        with_tools = first["signed_error_with_tools"]
        self.assertEqual(with_tools["mean"], 0.0166667)
        self.assertEqual(with_tools["median"], 0.1)
        self.assertEqual((with_tools["min"], with_tools["max"], with_tools["worst"]), (-0.4, 0.35, -0.4))
        self.assertEqual(second["signed_error"]["worst"], 0.25)
        overall = report["global"]["signed_error"]
        self.assertEqual(overall["n"], 4)
        self.assertEqual(overall["mean"], 0.0)
        self.assertEqual(overall["median"], 0.125)
        self.assertEqual(overall["worst"], -0.5)
        self.assertEqual(overall["underestimates"], 1)
        self.assertEqual(first["compaction_count"], 1)

    def test_null_estimates_and_zero_input_are_unmeasured_never_zero(self):
        self.populate()
        report = build_report(self.conn)
        self.assertEqual(report["unmeasured_responses"], 2)
        self.assertEqual(report["conversations"][1]["unmeasured_responses"], 2)
        self.assertEqual(report["conversations"][1]["signed_error"]["n"], 1)
        self.assertEqual(report["global"]["signed_error"]["n"], 4)

    def test_a_conversation_without_measured_response_has_no_fabricated_statistics(self):
        self.conn.execute("INSERT INTO engine_runs VALUES ('run-secret-x', 'conversation-secret-x')")
        self.conn.execute(
            "INSERT INTO engine_response_usage VALUES ('run-secret-x', 1, 10, 1, 0, NULL, NULL, NULL)"
        )
        report = build_report(self.conn)
        self.assertEqual(report["conversations"][0]["signed_error"]["n"], 0)
        self.assertIsNone(report["conversations"][0]["signed_error"]["mean"])
        self.assertEqual(report["global"]["signed_error"]["underestimates"], 0)

    def test_an_underestimate_smaller_than_the_display_precision_still_counts(self):
        self.response("conversation-secret-a", 600_000, 2_399_999, 0)
        signed = build_report(self.conn)["global"]["signed_error"]
        self.assertEqual(signed["underestimates"], 1)
        self.assertLess(signed["min"], 0)

    def test_report_states_that_tool_definitions_are_not_counted(self):
        report = build_report(self.conn)
        self.assertIs(report["tool_definitions_counted"], False)
        self.assertIn("n'inclut pas les définitions d'outils", report["notice"])
        self.assertIs(report["synthetic"], False)
        self.assertIs(build_report(self.conn, synthetic=True)["synthetic"], True)

    def test_cli_prints_and_writes_report_without_raw_ids(self):
        self.populate()
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "replay.sqlite"
            self.conn.commit()
            with sqlite3.connect(db) as destination:
                self.conn.backup(destination)
            output = Path(directory) / "results.json"
            command = subprocess.run([
                sys.executable, str(Path(__file__).with_name("token_estimate.py")),
                "report", "--db", str(db), "--output", str(output), "--synthetic",
            ], check=True, capture_output=True, text=True)
            text = output.read_text()
            self.assertEqual(command.stdout, text)
            self.assertNotIn("secret", text)
            result = json.loads(text)
            self.assertTrue(result["synthetic"])
            self.assertFalse(result["tool_definitions_counted"])
            self.assertEqual(result["global"]["signed_error"]["n"], 4)


class RequestTokenTests(unittest.TestCase):
    def test_counts_instructions_items_and_tools_with_fixed_overhead(self):
        body = {
            "instructions": "un deux",
            "input": [{"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "a b c"}]}],
            "tools": [{"type": "function", "name": "lire",
                       "description": "Lit un fait", "parameters": {}}],
        }
        # 2 + (4 + 3) + (4 + 1 + 3 + 1)
        self.assertEqual(request_tokens(body, FakeEncoder()), 18)

    def test_counts_function_call_arguments_and_outputs(self):
        body = {"input": [
            {"type": "function_call", "name": "lire", "arguments": "x y"},
            {"type": "function_call_output", "output": "z"},
        ]}
        # (4 + 1 + 2) + (4 + 1)
        self.assertEqual(request_tokens(body, FakeEncoder()), 12)


if __name__ == "__main__":
    unittest.main()
