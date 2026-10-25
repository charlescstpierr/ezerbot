#!/usr/bin/env python3
"""Mesure `GET /api/state` avec des résultats de sous-tâche qui joignent des captures (ticket #328).

Chaque passe lance `grokbot-wrapper --port 0 --db <base temporaire>` en mode simulé (aucun
compte réel, aucun LLM). Elle crée la conversation parente, un run parent (un envoi simulé),
un coffre avec trois secrets et un fil par sous-tâche, par l'API du binaire mesuré. Le serveur
arrêté, elle écrit en SQL R résultats terminés de C captures chacun : le lien du graphe, l'appel
`delegate`, la carte dans le fil parent et C images jointes au fil de la sous-tâche. Le serveur
relancé, elle chronomètre côté client N lectures `GET /api/state` après 3 lectures de chauffe.

Chaque lecture est vérifiée : statut 200, R cartes dans `delegation.delegations` et dans le fil
parent, R × C captures servies de chaque côté, et le secret de la légende jamais en clair.
Une passe qui échoue à une vérification fait échouer le script.

Avec `--strace`, une passe de plus par binaire, non chronométrée, compte par lecture d'état les
ouvertures du fichier du coffre (`*.secrets.json`), de son verrou et les listes de dossier
(`getdents64`). Ouvrir le coffre ouvre son verrou, liste son dossier et lit le fichier ; chaque
lecture de son contenu rouvre le verrou et le fichier. Le serveur travaille aussi en fond (`tick`) :
une fenêtre au repos de même durée est tracée juste après et retranchée.

Plusieurs binaires se comparent en alternance (A B A B …) pour que la dérive de la machine ne
favorise aucun côté :

  cargo build --release --locked -p grokbot-wrapper
  python3 tools/bench/state-captures.py --passes 3 --strace \\
      avant=/chemin/base/grokbot-wrapper apres=target/release/grokbot-wrapper

Sans binaire, le binaire mesuré est `target/release/grokbot-wrapper`. Par défaut R = 50, C = 4,
N = 30 (le cas du ticket).
"""
from pathlib import Path
import argparse
import http.client
import json
import os
import re
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
ACCOUNT = "local-workspace"
CAPTION = "Page remplie apres envoi"
CONTEXT = json.dumps({"texts": [], "entry_ids": [], "fact_ids": [], "skill_ids": []})


class Server:
    def __init__(self, binary, database):
        self.process = subprocess.Popen(
            [binary, "--port", "0", "--db", str(database)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.port = int(json.loads(self.process.stdout.readline())["url"].rsplit(":", 1)[1])
        deadline = time.monotonic() + 30
        while True:
            try:
                if self.request("GET", "/api/ping")[0] == 200:
                    return
            except OSError:
                pass
            if time.monotonic() > deadline:
                raise RuntimeError("le serveur ne répond pas à /api/ping")
            time.sleep(0.05)

    def request(self, method, path, body=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=60)
        headers = {"Host": f"127.0.0.1:{self.port}"}
        if method != "GET":
            headers.update({"X-Grokbot-Request": "local", "Content-Type": "application/json",
                            "Origin": f"http://127.0.0.1:{self.port}"})
        connection.request(method, path, body=None if body is None else json.dumps(body),
                           headers=headers)
        response = connection.getresponse()
        data = response.read()
        connection.close()
        return response.status, data

    def post(self, path, body):
        status, data = self.request("POST", path, body)
        if status != 200:
            raise RuntimeError(f"{path} : statut {status} {data[:200]!r}")
        return json.loads(data)

    def stop(self):
        self.process.send_signal(signal.SIGTERM)
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def seed(binary, database, results, captures):
    server = Server(binary, database)
    try:
        parent = server.post("/api/conversations", {})["conversation"]["id"]
        server.post("/api/send", {"conversation_id": parent, "message_id": "m0", "body": "Lance les sous-tâches."})
        for name, value in [("BENCH_JETON", "jeton-bench-0123456789"), ("BENCH_LEGENDE", CAPTION),
                            ("BENCH_MOT_DE_PASSE", "mot-de-passe-bench-42")]:
            server.post("/api/secrets/put", {"bot_id": "grokbot", "name": name,
                                             "description": "bench #328", "value": value})
        threads = [server.post("/api/conversations", {})["conversation"]["id"] for _ in range(results)]
    finally:
        server.stop()
    db = sqlite3.connect(database)
    (run,) = db.execute("SELECT run_id FROM engine_runs WHERE json_extract(record,'$.conversation_id')=?",
                        [parent]).fetchone()
    with db:
        for r, thread in enumerate(threads):
            joined = []
            for n in range(captures):
                image = f"bench-{r}-{n}"
                db.execute("INSERT INTO attachments_staged VALUES (?,?,'png',68,0)", [image, image])
                db.execute("INSERT INTO attachments_message_links VALUES (?,'joined',?,?)", [thread, image, n])
                joined.append({"id": image, "label": ["avant", "après"][n % 2], "caption": CAPTION})
            edge = f"delegation_bench_{r}"
            status = {"status": "done", "result": f"Sous-tâche {r} terminée.", "captures": joined}
            db.execute("INSERT INTO delegation_graph VALUES (?,?,?,'grokbot','grokbot',1,NULL,?,?,?)",
                       [edge, run, run, thread, CONTEXT, json.dumps(status)])
            db.execute("INSERT INTO wrapper_delegation_calls VALUES (?,?,'bench',?,NULL,?)",
                       [edge, run, f"Vérifier la page {r}", r])
            db.execute("INSERT INTO wrapper_messages (account_id,conversation_id,message_id,created_at,kind)"
                       " VALUES (?,?,?,?,'delegation')", [ACCOUNT, parent, edge, r])
    db.close()
    return parent


def check(data, parent, results, captures):
    if CAPTION.encode() in data:
        raise RuntimeError("la légende secrète sort en clair")
    state = json.loads(data)
    cards = state["delegation"]["delegations"]
    messages = next(c for c in state["conversations"] if c["id"] == parent)["messages"]
    entries = [m for m in messages if m["kind"] == "delegation"]
    shown = sum(len(card["status"].get("captures", [])) for card in cards)
    shown_entries = sum(len(m["payload"]["status"].get("captures", [])) for m in entries)
    if (len(cards), len(entries), shown, shown_entries) != (results, results, results * captures,
                                                             results * captures):
        raise RuntimeError(f"travail incomplet : {len(cards)} cartes, {len(entries)} entrées, "
                           f"{shown} et {shown_entries} captures servies")


def run_pass(binary, workdir, results, captures, reads, trace):
    directory = Path(tempfile.mkdtemp(dir=workdir))
    try:
        database = directory / "w.sqlite"
        parent = seed(binary, database, results, captures)
        server = Server(binary, database)
        try:
            for _ in range(3):
                status, data = server.request("GET", "/api/state")
                if status != 200:
                    raise RuntimeError(f"/api/state : statut {status}")
                check(data, parent, results, captures)
            if trace:
                return strace_counts(server, reads, parent, results, captures)
            times = []
            for _ in range(reads):
                started = time.perf_counter()
                status, data = server.request("GET", "/api/state")
                times.append((time.perf_counter() - started) * 1000)
                if status != 200:
                    raise RuntimeError(f"/api/state : statut {status}")
                check(data, parent, results, captures)
            return times
        finally:
            server.stop()
    finally:
        shutil.rmtree(directory, ignore_errors=True)


def traced(pid, work, busy):
    """Syscall counts of the server while `work` runs, and how long it ran. A busy window that
    traced nothing means strace never attached: an error, never zero opens."""
    log = Path(tempfile.mkstemp(suffix=".strace")[1])
    tracer = subprocess.Popen(["strace", "-f", "-qq", "-e", "trace=openat,getdents64", "-o", str(log),
                               "-p", str(pid)], stderr=subprocess.PIPE, text=True)
    time.sleep(1.0)
    if tracer.poll() is not None:
        raise RuntimeError(f"strace s'est arrêté : {tracer.stderr.read().strip()}")
    started = time.monotonic()
    work()
    elapsed = time.monotonic() - started
    tracer.send_signal(signal.SIGINT)
    errors = tracer.communicate()[1].strip()
    text = log.read_text()
    log.unlink()
    if busy and "secrets.json" not in text:
        raise RuntimeError(f"strace n'a vu aucune ouverture du coffre pendant les lectures : {errors}")
    return elapsed, {
        "coffre_ouvert": len(re.findall(r'secrets\.json", ', text)),
        "verrou_ouvert": len(re.findall(r'secrets\.json\.lock", ', text)),
        "dossier_liste": len(re.findall(r'getdents64\(', text)),
    }


def strace_counts(server, reads, parent, results, captures):
    def states():
        for _ in range(reads):
            status, data = server.request("GET", "/api/state")
            if status != 200:
                raise RuntimeError(f"/api/state : statut {status}")
            check(data, parent, results, captures)
    elapsed, busy = traced(server.process.pid, states, True)
    _, idle = traced(server.process.pid, lambda: time.sleep(elapsed), False)
    return {key: (busy[key] - idle[key]) / reads for key in busy}


def stats(values):
    return (f"médiane {statistics.median(values):.2f} ms, min {min(values):.2f}, max {max(values):.2f}, "
            f"écart-type {statistics.pstdev(values):.2f}, n {len(values)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binaries", nargs="*", help="nom=chemin du binaire release")
    parser.add_argument("--passes", type=int, default=3)
    parser.add_argument("--results", type=int, default=50)
    parser.add_argument("--captures", type=int, default=4)
    parser.add_argument("--reads", type=int, default=30)
    parser.add_argument("--strace", action="store_true")
    parser.add_argument("--workdir", default=os.environ.get("TMPDIR", tempfile.gettempdir()))
    args = parser.parse_args()
    sides = [b.split("=", 1) if "=" in b else (b, b) for b in args.binaries] or [
        ("branche", str(ROOT / "target/release/grokbot-wrapper"))]
    print(f"{args.results} résultats × {args.captures} captures, {args.reads} lectures par passe, "
          f"{args.passes} passes en alternance : " + " ".join(name for name, _ in sides))
    print(f"nproc {os.cpu_count()}, {' '.join(os.uname())}")
    print(f"charge {os.getloadavg()}")
    times = {name: [] for name, _ in sides}
    medians = {name: [] for name, _ in sides}
    for number in range(args.passes):
        for name, binary in sides:
            values = run_pass(binary, args.workdir, args.results, args.captures, args.reads, False)
            times[name] += values
            medians[name].append(statistics.median(values))
            print(f"passe {number + 1} {name} : {stats(values)}", flush=True)
    for name, _ in sides:
        print(f"{name} : toutes lectures {stats(times[name])} ; médianes des passes "
              + ", ".join(f"{m:.2f}" for m in medians[name]))
    if args.strace:
        for name, binary in sides:
            counts = run_pass(binary, args.workdir, args.results, args.captures, 10, True)
            print(f"{name} (strace, par lecture d'état) : fichier du coffre ouvert {counts['coffre_ouvert']:.1f} fois, "
                  f"verrou {counts['verrou_ouvert']:.1f}, dossier listé {counts['dossier_liste']:.1f}")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        print(f"échec : {error}", file=sys.stderr)
        sys.exit(1)
