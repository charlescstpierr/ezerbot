"""Behavioral checks of the SQLite -> private JSON report contract."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from prompt_cache import build_report


class PromptCacheReportTests(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.addCleanup(self.conn.close)
        self.conn.executescript("""
            CREATE TABLE engine_runs (run_id TEXT PRIMARY KEY, conversation_id TEXT);
            CREATE TABLE engine_response_usage (
                run_id TEXT, response INTEGER, input_tokens INTEGER,
                cached_input_tokens INTEGER, PRIMARY KEY (run_id, response)
            );
            CREATE TABLE engine_summaries (conversation_id TEXT, record TEXT);
        """)
        self.next_run = 0

    def response(self, conversation, inputs, cached, *, run=None, response=1):
        if run is None:
            # Neither raw identifier may leak into the result, even if it contains text.
            run = f"message-secret-{self.next_run}"
            self.next_run += 1
            self.conn.execute("INSERT INTO engine_runs VALUES (?, ?)", (run, conversation))
        self.conn.execute(
            "INSERT INTO engine_response_usage VALUES (?, ?, ?, ?)",
            (run, response, inputs, cached),
        )
        return run

    def test_exact_acceptance_ratio_after_separate_excluded_warmup(self):
        self.response("conversation-secret", 9999, 0)  # excluded warmup
        for inputs, cached in [(100, 0), (200, 150), (300, 270)]:
            self.response("conversation-secret", inputs, cached)
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / "replay.sqlite"
            # Commit before backup; uncommitted source writes would wait for themselves.
            self.conn.commit()
            with sqlite3.connect(db) as destination:
                self.conn.backup(destination)
            output = Path(directory) / "results.json"
            command = subprocess.run([
                sys.executable, str(Path(__file__).with_name("prompt_cache.py")),
                "report", "--db", str(db), "--output", str(output), "--fake",
            ], check=True, capture_output=True, text=True)
            result = json.loads(output.read_text())
            self.assertIn("420 ÷ 600 = 70,0 %", command.stdout)
            self.assertEqual(result["global"]["measured"]["calculation"], "420 ÷ 600 = 70,0 %")
            self.assertEqual(result["global"]["measured"]["responses"], 3)
            self.assertNotIn("secret", output.read_text())
            self.assertTrue(result["synthetic"])

    def test_first_response_is_excluded_per_conversation_not_per_run(self):
        run = self.response("one", 9000, 9000)
        self.response("one", 2000, 1000, run=run, response=2)
        self.response("two", 8000, 8000)
        self.response("one", 3000, 2000)
        self.response("two", 5000, 4000)
        report = build_report(self.conn)
        self.assertEqual(report["global"]["excluded_first_responses"], 2)
        self.assertEqual(report["global"]["measured"]["calculation"], "7000 ÷ 10000 = 70,0 %")
        self.assertEqual(report["conversations"][0]["turns"][0]["measured"]["input_tokens"], 2000)
        self.assertEqual(report["conversations"][1]["measured"]["input_tokens"], 5000)

    def test_short_responses_are_separate_and_1024_is_cacheable(self):
        self.response("one", 2000, 0)
        self.response("one", 1023, 0)
        self.response("one", 1024, 768)
        report = build_report(self.conn)
        self.assertEqual(report["global"]["cacheable"]["calculation"], "768 ÷ 1024 = 75,0 %")
        self.assertEqual(report["global"]["below_1024"]["input_tokens"], 1023)
        turns = report["conversations"][0]["turns"]
        self.assertTrue(turns[1]["responses"][0]["below_1024"])
        self.assertFalse(turns[2]["responses"][0]["below_1024"])
        self.assertEqual(report["global"]["measured"]["input_tokens"], 2047)

    def test_compaction_is_reported_without_reading_summary_text(self):
        self.response("secret", 2000, 0)
        self.conn.execute("INSERT INTO engine_summaries VALUES ('secret', 'private summary')")
        report = build_report(self.conn)
        self.assertEqual(report["compaction_count"], 1)
        self.assertEqual(report["conversations"][0]["compaction_count"], 1)
        self.assertNotIn("private summary", json.dumps(report))
        self.assertNotIn("secret", json.dumps(report))

    def test_zero_and_missing_telemetry_are_not_fabricated_ratios(self):
        self.conn.execute("INSERT INTO engine_runs VALUES ('missing', 'one')")
        self.response("one", 2000, 1000)
        self.response("two", 0, 0)
        report = build_report(self.conn)
        self.assertEqual(report["unmeasured_runs"], 1)
        self.assertEqual(report["global"]["measured"]["rate_percent"], 50.0)
        self.assertIsNone(report["conversations"][1]["measured"]["rate_percent"])

    def test_empty_replay_has_no_percentage(self):
        report = build_report(self.conn)
        self.assertEqual(report["conversations"], [])
        self.assertIsNone(report["global"]["measured"]["rate_percent"])


if __name__ == "__main__":
    unittest.main()
