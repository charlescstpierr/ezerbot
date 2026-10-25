#!/usr/bin/env python3
"""Calibration of the context-size estimate against the provider's input tokens.

`report` reads engine_response_usage: the characters the compaction trigger
counted for each send, against the input tokens the provider billed for it.
`replay` produces such a database on the test-only FakeOpenAi. `count` is the
optional token counter the replay can use in place of a fixed formula.

The signed error is (estimated - actual) / actual. Negative means the
estimate was too small, which is the dangerous side: the window can overflow
before a compaction fires.
"""

import argparse
import json
import os
from pathlib import Path
import sqlite3
import statistics
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
REPLAY_TEST = "varied_conversations_report_the_trigger_estimate_against_counted_tokens"
PER_ITEM_OVERHEAD_TOKENS = 4
DIGITS = 6
NOTICE = (
    "context_characters est le nombre exact que le déclencheur de compaction "
    "compare au seuil : historique éligible, instructions et texte du résumé. "
    "Il n'inclut pas les définitions d'outils, pourtant envoyées avec la requête ; "
    "tool_characters les mesure à part. Les tokens de l'appel de résumé ne sont "
    "pas dans engine_response_usage."
)
SYNTHETIC_NOTICE = (
    "FakeOpenAi : input_tokens vient d'un compteur de test, pas du fournisseur ; "
    "aucune erreur réelle ni objectif de score validé."
)


def rounded(value):
    return round(value, DIGITS)


def summarize(errors):
    if not errors:
        return {"n": 0, "mean": None, "median": None, "pstdev": None,
                "min": None, "max": None, "worst": None, "underestimates": 0}
    return {
        "n": len(errors),
        "mean": rounded(statistics.fmean(errors)),
        "median": rounded(statistics.median(errors)),
        "pstdev": rounded(statistics.pstdev(errors)),
        "min": rounded(min(errors)),
        "max": rounded(max(errors)),
        "worst": rounded(max(errors, key=abs)),
        "underestimates": sum(error < 0 for error in errors),
    }


def statistics_of(responses):
    return {
        "signed_error": summarize([r["signed_error"] for r in responses]),
        "signed_error_with_tools": summarize(
            [r["signed_error_with_tools"] for r in responses]
        ),
    }


def build_report(conn, synthetic=False):
    # rowid is the durable run insertion order. Only counts are read: never
    # message bodies, run records, summary text or the raw identifiers.
    conversations = {}
    unmeasured = 0
    for (run, conversation, response, inputs, characters, per_token, tools) in conn.execute("""
        SELECT r.rowid, r.conversation_id, u.response, u.input_tokens,
               u.context_characters, u.characters_per_token, u.tool_characters
        FROM engine_runs r JOIN engine_response_usage u ON u.run_id = r.run_id
        ORDER BY r.rowid, u.response
    """):
        item = conversations.setdefault(conversation, {
            "conversation": len(conversations) + 1,
            "compaction_count": 0,
            "unmeasured_responses": 0,
            "runs": {},
            "responses": [],
        })
        if characters is None or per_token is None or tools is None or not inputs:
            item["unmeasured_responses"] += 1
            unmeasured += 1
            continue
        turn = item["runs"].setdefault(run, len(item["runs"]) + 1)
        estimated = characters / per_token
        with_tools = (characters + tools) / per_token
        item["responses"].append({
            "turn": turn,
            "response": response,
            "input_tokens": inputs,
            "context_characters": characters,
            "tool_characters": tools,
            "estimated_tokens": rounded(estimated),
            "signed_error": rounded((estimated - inputs) / inputs),
            "signed_error_with_tools": rounded((with_tools - inputs) / inputs),
            "tool_share": rounded(tools / per_token / inputs),
        })
    for conversation, count in conn.execute(
        "SELECT conversation_id, COUNT(*) FROM engine_summaries "
        "WHERE record IS NOT NULL GROUP BY conversation_id"
    ):
        if conversation in conversations:
            conversations[conversation]["compaction_count"] = count

    everything = []
    items = []
    for item in conversations.values():
        del item["runs"]
        item.update(statistics_of(item["responses"]))
        everything.extend(item["responses"])
        items.append(item)
    report = {
        "schema_version": 1,
        "source": "engine_response_usage (réponses principales seulement)",
        "synthetic": synthetic,
        "tool_definitions_counted": False,
        "notice": NOTICE,
        "signed_error_definition": "(tokens estimés - tokens réels) / tokens réels ; négatif = sous-estimation",
        "unmeasured_responses": unmeasured,
        "global": statistics_of(everything),
        "conversations": items,
    }
    if synthetic:
        report["synthetic_notice"] = SYNTHETIC_NOTICE
    return report


def report_database(db, synthetic=False):
    # A read-only snapshot, including when the wrapper still has its database open.
    with sqlite3.connect(Path(db).resolve().as_uri() + "?mode=ro", uri=True) as conn:
        conn.execute("BEGIN")
        return build_report(conn, synthetic)


def replay(counter):
    # The integration fixture owns its disposable DB/session and uses only loopback
    # endpoints. No installed Ezerbot, HOME data or external credentials are read.
    with tempfile.TemporaryDirectory(prefix="ezerbot-token-estimate-") as directory:
        output = Path(directory) / "results.json"
        env = dict(os.environ, EZERBOT_TOKEN_ESTIMATE_OUTPUT=str(output))
        if counter == "o200k":
            env["EZERBOT_TOKEN_COUNTER"] = f"{sys.executable} {Path(__file__).resolve()} count"
        subprocess.run([
            "cargo", "test", "--locked", "-p", "grokbot-wrapper",
            "--test", "token_estimate_replay", REPLAY_TEST, "--", "--exact",
        ], cwd=ROOT, env=env, check=True, stdout=sys.stderr)
        return json.loads(output.read_text())


def text_of(content):
    if isinstance(content, str):
        return content
    return "".join(
        part.get("text", "") for part in content or []
        if isinstance(part, dict) and isinstance(part.get("text"), str)
    )


def request_tokens(body, encoder):
    """o200k_base tokens of what a Responses API request carries.

    A proxy for the provider tokenizer, not the provider itself: the real
    count also includes provider-side framing and reasoning state this does not
    see. Counted: instructions; per input item its text content, function-call
    name and arguments, and function-call output, plus PER_ITEM_OVERHEAD_TOKENS
    for the item framing; per tool its name, description and JSON parameters
    plus the same overhead. Encrypted reasoning and image bytes are not counted.
    """
    def count(text):
        return len(encoder.encode(text, disallowed_special=()))

    total = count(body.get("instructions") or "")
    for item in body.get("input") or []:
        parts = [text_of(item.get("content"))]
        for key in ("name", "arguments", "output"):
            value = item.get(key)
            if isinstance(value, str):
                parts.append(value)
            elif value is not None:
                parts.append(json.dumps(value, ensure_ascii=False, separators=(",", ":")))
        total += PER_ITEM_OVERHEAD_TOKENS + sum(count(part) for part in parts if part)
    for tool in body.get("tools") or []:
        parameters = json.dumps(
            tool.get("parameters", {}), ensure_ascii=False, separators=(",", ":")
        )
        total += PER_ITEM_OVERHEAD_TOKENS + sum(count(part) for part in (
            tool.get("name") or "", tool.get("description") or "", parameters,
        ) if part)
    return total


def count_stream(lines, out):
    try:
        import tiktoken
    except ImportError:
        sys.exit("tiktoken est requis pour --counter o200k (pip install tiktoken)")
    encoder = tiktoken.get_encoding("o200k_base")
    for line in lines:
        line = line.strip()
        if not line:
            continue
        print(request_tokens(json.loads(line), encoder), file=out, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    replay_parser = commands.add_parser("replay", help="conversations on FakeOpenAi only")
    replay_parser.add_argument("--output", type=Path, required=True)
    replay_parser.add_argument("--counter", choices=["o200k"], default=None)
    report = commands.add_parser("report", help="read an engine SQLite database, without network")
    report.add_argument("--db", type=Path, required=True)
    report.add_argument("--output", type=Path, required=True)
    report.add_argument("--synthetic", action="store_true",
                        help="the database comes from a FakeOpenAi replay")
    commands.add_parser("count", help="request bodies on stdin, o200k token counts on stdout")
    args = parser.parse_args()
    if args.command == "count":
        count_stream(sys.stdin, sys.stdout)
        return
    if args.command == "replay":
        result = replay(args.counter)
    else:
        result = report_database(args.db, args.synthetic)
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
