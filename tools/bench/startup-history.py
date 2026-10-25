#!/usr/bin/env python3
"""Démarrage du moteur sur une base avec historique (#302).

Sème une base de N conversations × M envois `/api/send` avec le premier
binaire, puis chronomètre des redémarrages à froid
sur des copies identiques de cette base : du `spawn` jusqu'à l'annonce
`{"url": …}` et jusqu'au premier `GET /api/ping` = 200. Chaque redémarrage
vérifie ensuite qu'un envoi passe. Avec plusieurs `--bin`, les passes
alternent les binaires (A B A B …) pour que la dérive de la machine ne
favorise aucun côté. Une référence base vide est mesurée de la même façon.

Comme le protocole de l'issue, le binaire release tourne sans compte ni LLM :
chaque envoi écrit sa soumission, son message et sa run, et la run se termine
en `failed` (`signin_required`). Le script affiche les runs par statut. Les
runs étant terminées, la reprise de runs interrompues n'est pas chronométrée
ici ; les tests `crash_recovery` et `orchestrated_crash` la couvrent.

    cargo build --release --locked -p grokbot-wrapper
    python3 tools/bench/startup-history.py --bin target/release/grokbot-wrapper
    python3 tools/bench/startup-history.py --bin avant/grokbot-wrapper \\
        --bin target/release/grokbot-wrapper --passes 3

Les fichiers temporaires sont supprimés à la fin, sauf `--keep-seed DIR`, qui
conserve la base semée pour la réutiliser avec `--seed DIR/w.sqlite`.
"""

import argparse
import json
import os
import platform
import select
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from local_api import LaunchIdentity, LocalEndpoint

BODY = "Peux-tu m'aider à préparer la réunion de demain ? " * 4
START_LIMIT_S = 60


def request(endpoint, method, path, body=None, timeout=120):
    status, data, _ = endpoint.request(method, path,
        body=json.dumps(body) if body is not None else None,
        headers={"Content-Type": "application/json"}, timeout=timeout)
    return status, data


def start(binary, db):
    started = time.perf_counter()
    identity = LaunchIdentity()
    process = identity.spawn(
        [binary, "--port", "0", "--db", db],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    try:
        ready, _, _ = select.select([process.stdout], [], [], START_LIMIT_S)
        if not ready:
            raise RuntimeError(f"{binary} n'a pas annoncé d'URL en {START_LIMIT_S} s")
        line = process.stdout.readline()
        announced = time.perf_counter()
        if not line:
            raise RuntimeError(f"{binary} s'est arrêté sans annoncer d'URL")
        endpoint = LocalEndpoint(identity, json.loads(line)["url"])
        deadline = announced + START_LIMIT_S
        while True:
            remaining = deadline - time.perf_counter()
            if remaining <= 0:
                raise RuntimeError(f"/api/ping n'a pas répondu 200 en {START_LIMIT_S} s")
            try:
                status, _ = request(endpoint, "GET", "/api/ping", timeout=remaining)
                if status == 200:
                    break
            except OSError:
                pass
    except BaseException:
        stop(process)
        raise
    pinged = time.perf_counter()
    return process, endpoint, (announced - started) * 1000, (pinged - started) * 1000


def stop(process):
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def new_conversation(endpoint):
    status, data = request(endpoint, "POST", "/api/conversations", {})
    if status != 200:
        raise RuntimeError(f"POST /api/conversations a répondu {status}")
    return json.loads(data)["conversation"]["id"]


def send(endpoint, conversation, message):
    status, _ = request(
        endpoint,
        "POST",
        "/api/send",
        {"conversation_id": conversation, "message_id": message, "body": BODY},
    )
    return status


def seed(binary, db, conversations, messages):
    process, endpoint, _, _ = start(binary, db)
    try:
        for c in range(conversations):
            conversation = new_conversation(endpoint)
            for m in range(messages):
                status = send(endpoint, conversation, f"c{c}m{m}")
                if status != 200:
                    raise RuntimeError(f"envoi de semis c{c}m{m} : {status}")
    finally:
        stop(process)


def count(db, table):
    with sqlite3.connect(db) as connection:
        return connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]


def runs_by_status(db):
    with sqlite3.connect(db) as connection:
        rows = connection.execute(
            "SELECT status, count(*) FROM engine_runs GROUP BY status ORDER BY status"
        ).fetchall()
    return " ".join(f"{status}={n}" for status, n in rows) or "aucune"


def copy_db(source, directory):
    target = os.path.join(directory, "w.sqlite")
    for suffix in ("", "-wal", "-shm"):
        if os.path.exists(source + suffix):
            shutil.copyfile(source + suffix, target + suffix)
    return target


def restart(binary, source, scratch, errors):
    directory = tempfile.mkdtemp(dir=scratch)
    db = copy_db(source, directory) if source else os.path.join(directory, "w.sqlite")
    process, endpoint, url_ms, ping_ms = start(binary, db)
    try:
        conversation = new_conversation(endpoint)
        status = send(endpoint, conversation, "apres-demarrage")
        if status != 200:
            errors.append(f"envoi après démarrage : {status}")
    finally:
        stop(process)
    shutil.rmtree(directory)
    return url_ms, ping_ms


def summary(values):
    return (
        f"med={statistics.median(values):.1f} min={min(values):.1f} "
        f"max={max(values):.1f} sd={statistics.pstdev(values):.1f} n={len(values)}"
    )


def machine():
    commit = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        capture_output=True,
        text=True,
        cwd=os.path.dirname(os.path.abspath(__file__)),
    ).stdout.strip()
    return (
        f"uname: {' '.join(platform.uname())}\n"
        f"nproc: {os.cpu_count()}  load: {' '.join(f'{x:.2f}' for x in os.getloadavg())}\n"
        f"commit du dépôt: {commit or '?'}"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", action="append", required=True, help="binaire release (répétable)")
    parser.add_argument("--conversations", type=int, default=50)
    parser.add_argument("--messages", type=int, default=20)
    parser.add_argument("--restarts", type=int, default=5, help="redémarrages par passe")
    parser.add_argument("--passes", type=int, default=1, help="passes par binaire, alternées")
    parser.add_argument("--empty", type=int, default=7, help="démarrages base vide par binaire")
    parser.add_argument("--seed", help="base déjà semée à réutiliser")
    parser.add_argument("--keep-seed", help="dossier où conserver la base semée")
    args = parser.parse_args()
    binaries = [os.path.abspath(path) for path in args.bin]

    scratch = tempfile.mkdtemp(prefix="startup-history-")
    errors = []
    try:
        if args.seed:
            source = os.path.abspath(args.seed)
        else:
            seed_dir = args.keep_seed or os.path.join(scratch, "seed")
            os.makedirs(seed_dir, exist_ok=True)
            source = os.path.join(seed_dir, "w.sqlite")
            began = time.perf_counter()
            seed(binaries[0], source, args.conversations, args.messages)
            print(f"semis {args.conversations}x{args.messages} en {time.perf_counter() - began:.0f} s")
        print(
            f"base: {os.path.getsize(source)} o, submissions={count(source, 'submissions')} "
            f"engine_runs={count(source, 'engine_runs')} "
            f"engine_run_events={count(source, 'engine_run_events')}"
        )
        print(f"runs par statut: {runs_by_status(source)}")
        print(machine())

        history = {binary: ([], []) for binary in binaries}
        empty = {binary: ([], []) for binary in binaries}
        for number in range(args.passes):
            for binary in binaries:
                urls, pings = [], []
                for _ in range(args.restarts):
                    url_ms, ping_ms = restart(binary, source, scratch, errors)
                    urls.append(url_ms)
                    pings.append(ping_ms)
                history[binary][0].extend(urls)
                history[binary][1].extend(pings)
                print(f"passe {number + 1} {binary}\n  historique ping {summary(pings)}")
            for binary in binaries:
                for _ in range(args.empty // args.passes + (number < args.empty % args.passes)):
                    url_ms, ping_ms = restart(binary, None, scratch, errors)
                    empty[binary][0].append(url_ms)
                    empty[binary][1].append(ping_ms)

        print("\nrésultat")
        for binary in binaries:
            print(binary)
            print(f"  historique url  {summary(history[binary][0])}")
            print(f"  historique ping {summary(history[binary][1])}")
            print(f"  base vide  url  {summary(empty[binary][0])}")
            print(f"  base vide  ping {summary(empty[binary][1])}")
        restarts = sum(len(pings) for _, pings in history.values())
        restarts += sum(len(pings) for _, pings in empty.values())
        print(f"redémarrages: {restarts}, envois après démarrage en échec: {len(errors)}")
        for error in errors:
            print(f"  {error}")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
