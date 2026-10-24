#!/usr/bin/env python3
"""Offline prompt-cache accounting and a replay against the test-only FakeOpenAi."""

import argparse
from decimal import Decimal, ROUND_HALF_UP
import json
from pathlib import Path
import os
import sqlite3
import subprocess
import sys
import tempfile


MIN_CACHEABLE_INPUT = 1024
ROOT = Path(__file__).resolve().parents[1]


def metric(responses):
    """Token-weighted ratio, never an average of per-response percentages."""
    responses = list(responses)
    inputs = sum(row["input_tokens"] for row in responses)
    cached = sum(row["cached_input_tokens"] for row in responses)
    percent = (
        (Decimal(cached) * 100 / Decimal(inputs)).quantize(
            Decimal("0.1"), rounding=ROUND_HALF_UP
        )
        if inputs else None
    )
    return {
        "responses": len(responses),
        "input_tokens": inputs,
        "cached_input_tokens": cached,
        "rate_percent": float(percent) if percent is not None else None,
        "calculation": (
            f"{cached} ÷ {inputs} = {str(percent).replace('.', ',')} %"
            if percent is not None else "n/a (aucun token d'entrée)"
        ),
    }


def totals(responses):
    measured = [row for row in responses if not row["excluded_first"]]
    return {
        "measured": metric(measured),
        "cacheable": metric(row for row in measured if not row["below_1024"]),
        "below_1024": metric(row for row in measured if row["below_1024"]),
        "excluded_first_responses": sum(row["excluded_first"] for row in responses),
    }


def build_report(conn):
    # rowid is the durable run insertion order; response is the model-call ordinal
    # within that run. Do not read message bodies, run records or summary text.
    conversations = {}
    for run, conversation, response, inputs, cached in conn.execute("""
        SELECT r.rowid, r.conversation_id, u.response, u.input_tokens, u.cached_input_tokens
        FROM engine_runs r LEFT JOIN engine_response_usage u ON u.run_id = r.run_id
        ORDER BY r.rowid, u.response
    """):
        item = conversations.setdefault(conversation, {
            "conversation": len(conversations) + 1,
            "compaction_count": 0,
            "unmeasured_runs": 0,
            "has_measured": False,
            "turns": [],
        })
        if not item["turns"] or item["turns"][-1]["_run"] != run:
            item["turns"].append({
                "_run": run, "turn": len(item["turns"]) + 1, "responses": [],
            })
        turn = item["turns"][-1]
        if response is None:
            item["unmeasured_runs"] += 1
            continue
        turn["responses"].append({
            "response": response,
            "input_tokens": inputs,
            "cached_input_tokens": cached,
            "excluded_first": not item["has_measured"],
            "below_1024": inputs < MIN_CACHEABLE_INPUT,
            "rate_percent": metric([{"input_tokens": inputs, "cached_input_tokens": cached}])["rate_percent"],
        })
        item["has_measured"] = True
    for conversation, count in conn.execute(
        "SELECT conversation_id, COUNT(*) FROM engine_summaries GROUP BY conversation_id"
    ):
        if conversation in conversations:
            conversations[conversation]["compaction_count"] = count

    all_responses = []
    for item in conversations.values():
        del item["has_measured"]
        responses = []
        for turn in item["turns"]:
            del turn["_run"]
            turn.update(totals(turn["responses"]))
            responses.extend(turn["responses"])
        item.update(totals(responses))
        all_responses.extend(responses)
    items = list(conversations.values())
    return {
        "schema_version": 1,
        "source": "engine_response_usage (réponses principales seulement)",
        "synthetic": True,
        "notice": "FakeOpenAi : télémétrie fixe simulée, aucun taux réel ni objectif de score validé.",
        "minimum_cacheable_input_tokens": MIN_CACHEABLE_INPUT,
        "compaction_count": sum(item["compaction_count"] for item in items),
        "unmeasured_runs": sum(item["unmeasured_runs"] for item in items),
        "global": totals(all_responses),
        "conversations": items,
    }


def report_database(db):
    # A read-only snapshot, including when the wrapper still has its database open.
    with sqlite3.connect(Path(db).resolve().as_uri() + "?mode=ro", uri=True) as conn:
        conn.execute("BEGIN")
        return build_report(conn)


def replay():
    # The integration fixture owns its disposable DB/session and uses only loopback
    # endpoints. No installed Ezerbot, HOME data or external credentials are read.
    with tempfile.TemporaryDirectory(prefix="ezerbot-cache-report-") as directory:
        output = Path(directory) / "results.json"
        env = dict(os.environ, EZERBOT_PROMPT_CACHE_REPLAY_OUTPUT=str(output))
        subprocess.run([
            "cargo", "test", "--locked", "-p", "grokbot-wrapper",
            "--test", "prompt_cache_replay", "five_turn_fake_replay_writes_private_report",
            "--", "--exact",
        ], cwd=ROOT, env=env, check=True, stdout=sys.stderr)
        return json.loads(output.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("replay", help="five turns per scenario, FakeOpenAi only").add_argument(
        "--output", type=Path, required=True
    )
    report = commands.add_parser("report", help="read a FakeOpenAi replay SQLite database, without network")
    report.add_argument("--db", type=Path, required=True)
    report.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = replay() if args.command == "replay" else report_database(args.db)
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
