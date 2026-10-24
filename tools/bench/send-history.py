#!/usr/bin/env python3
"""Mesure `POST /api/send` en fonction de l'historique déjà enregistré (ticket #301).

Chaque passe lance `grokbot-wrapper --port 0 --db <base temporaire>` en mode simulé
(aucun compte réel, aucun LLM), rejoue un scénario d'envois et chronomètre chaque
requête HTTP côté client. La base temporaire est supprimée à la fin de la passe.

Scénarios :
  history NC NM   NC conversations × NM envois (celui du ticket : 50 × 20), puis une
                  sonde : une conversation neuve reçoit 20 envois. La sonde ne dépend
                  que de l'historique des AUTRES conversations.
  curve           history pour NC = 10, 25, 50 (NM = 20) ; affiche la pente de la sonde
                  et des 50 derniers envois en ms par tranche de 1 000 soumissions.
  conversation N  une seule conversation de N envois ; compare les 20 premiers et les
                  20 derniers pour montrer le coût de la taille de la conversation courante.

Plusieurs binaires se comparent en alternance (A B A B …) pour que la dérive de la
machine ne favorise aucun côté :

  cargo build --release --locked -p grokbot-wrapper
  python3 tools/bench/send-history.py --passes 3 \\
      avant=/chemin/base/grokbot-wrapper apres=target/release/grokbot-wrapper

Sans argument, le binaire mesuré est `target/release/grokbot-wrapper` et le scénario
est `history 50 20`. Chaque passe affiche ses erreurs (statut ≠ 200) et le travail
réellement fait (soumissions en base) ; une passe avec erreur fait échouer le script.
"""
from pathlib import Path
import argparse
import hashlib
import http.client
import json
import os
import shutil
import signal
import sqlite3
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
BODY = "Peux-tu m'aider à préparer la réunion de demain ? " * 4
PROBE_SENDS = 20


class Server:
    def __init__(self, binary, database):
        self.process = subprocess.Popen(
            [binary, '--port', '0', '--db', str(database)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.port = int(json.loads(self.process.stdout.readline())['url'].rsplit(':', 1)[1])
        deadline = time.monotonic() + 30
        while True:
            try:
                if self.request('GET', '/api/ping')[0] == 200:
                    return
            except OSError:
                pass
            if time.monotonic() > deadline:
                raise RuntimeError('le serveur ne répond pas à /api/ping')
            time.sleep(0.01)

    def request(self, method, path, body=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=60)
        headers = {'Host': f'127.0.0.1:{self.port}'}
        if method != 'GET':
            headers.update({
                'X-Grokbot-Request': 'local',
                'Content-Type': 'application/json',
                'Origin': f'http://127.0.0.1:{self.port}',
            })
        connection.request(method, path, body=None if body is None else json.dumps(body), headers=headers)
        response = connection.getresponse()
        data = response.read()
        connection.close()
        return response.status, data

    def conversation(self):
        status, data = self.request('POST', '/api/conversations', {})
        if status != 200:
            raise RuntimeError(f'création de conversation : statut {status}')
        return json.loads(data)['conversation']['id']

    def send(self, conversation, message):
        started = time.perf_counter()
        status, _ = self.request('POST', '/api/send', {
            'conversation_id': conversation, 'message_id': message, 'body': BODY})
        return (time.perf_counter() - started) * 1000, status

    def stop(self):
        self.process.send_signal(signal.SIGTERM)
        try:
            self.process.wait(5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def summary(values):
    return {
        'median': statistics.median(values), 'min': min(values), 'max': max(values),
        'sd': statistics.pstdev(values), 'n': len(values),
    }


def show(values):
    s = summary(values)
    return f"médiane={s['median']:.2f} min={s['min']:.2f} max={s['max']:.2f} écart-type={s['sd']:.2f} n={s['n']}"


def run_pass(binary, conversations, sends, scratch, profile=None):
    """Rejoue `conversations × sends` envois puis la sonde ; renvoie les mesures de la passe.

    `profile` est une commande lancée pendant les 50 derniers envois, `{pid}` désignant
    le serveur ; elle reçoit SIGINT juste après le dernier envoi chronométré.
    """
    workdir = Path(tempfile.mkdtemp(prefix='send-history-', dir=scratch))
    database = workdir / 'workspace.sqlite'
    try:
        server = Server(binary, database)
        seeded, probe, errors = [], [], 0
        profiler = None
        profile_from = conversations * sends - 50
        try:
            for c in range(conversations):
                conversation = server.conversation()
                for m in range(sends):
                    if profile and profiler is None and len(seeded) >= profile_from:
                        profiler = subprocess.Popen('exec ' + profile.format(pid=server.process.pid), shell=True)
                        time.sleep(1)
                    elapsed, status = server.send(conversation, f'c{c}m{m}')
                    seeded.append(elapsed)
                    errors += status != 200
            if profiler:
                profiler.send_signal(signal.SIGINT)
                profiler.wait(60)
            conversation = server.conversation()
            for m in range(PROBE_SENDS):
                elapsed, status = server.send(conversation, f'probe-m{m}')
                probe.append(elapsed)
                errors += status != 200
        finally:
            server.stop()
        with sqlite3.connect(database) as db:
            stored = db.execute('SELECT COUNT(*) FROM submissions').fetchone()[0]
        expected = conversations * sends + PROBE_SENDS
        return {
            'errors': errors, 'stored': stored, 'expected': expected,
            'db_bytes': database.stat().st_size,
            'first50': seeded[:50], 'last50': seeded[-50:], 'probe': probe, 'all': seeded,
        }
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def scenarios(args):
    if args.scenario == 'history':
        return [(args.conversations, args.sends)]
    if args.scenario == 'curve':
        return [(10, 20), (25, 20), (50, 20)]
    return [(1, args.sends)]


def binary_label(spec):
    label, sep, path = spec.partition('=')
    return (label, path) if sep else (Path(spec).parent.name or spec, spec)


def fingerprint(path):
    digest = hashlib.sha256(Path(path).read_bytes()).hexdigest()
    return digest[:12]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('binaries', nargs='*', default=[str(ROOT / 'target/release/grokbot-wrapper')],
                        help='binaire, ou étiquette=binaire ; plusieurs binaires alternent à chaque passe')
    parser.add_argument('--scenario', choices=['history', 'curve', 'conversation'], default='history')
    parser.add_argument('--conversations', type=int, default=50)
    parser.add_argument('--sends', type=int, default=20,
                        help='envois par conversation (scénario conversation : 200 par défaut)')
    parser.add_argument('--passes', type=int, default=1)
    parser.add_argument('--tmp', default=None, help='dossier des bases temporaires')
    parser.add_argument('--profile', default=None,
                        help="commande lancée pendant les 50 derniers envois ({pid} = serveur), "
                             "par ex. 'strace -c -f -o strace.txt -p {pid}' ou 'perf record -g -o perf.data -p {pid}' ; "
                             "les chiffres de cette passe sont faussés par l'outil")
    args = parser.parse_args()
    if args.scenario == 'conversation' and args.sends == 20:
        args.sends = 200

    binaries = [binary_label(spec) for spec in args.binaries]
    commit = subprocess.run(['git', '-C', str(ROOT), 'rev-parse', '--short', 'HEAD'],
                            capture_output=True, text=True).stdout.strip()
    print(f"machine nproc={os.cpu_count()} {' '.join(os.uname())}")
    print(f"dépôt {commit} ; charge {os.getloadavg()}")
    for label, path in binaries:
        print(f"binaire {label} = {path} sha256:{fingerprint(path)}")

    results = {}
    failed = False
    for pass_number in range(1, args.passes + 1):
        for conversations, sends in scenarios(args):
            for label, path in binaries:
                run = run_pass(path, conversations, sends, args.tmp, args.profile)
                key = (label, conversations, sends)
                results.setdefault(key, []).append(run)
                ok = run['errors'] == 0 and run['stored'] == run['expected']
                failed |= not ok
                print(f"passe {pass_number} {label} {conversations}×{sends} "
                      f"erreurs={run['errors']} soumissions={run['stored']}/{run['expected']} "
                      f"base={run['db_bytes']} octets")
                if args.scenario == 'conversation':
                    print(f"  20 premiers {show(run['all'][:20])}")
                    print(f"  20 derniers {show(run['all'][-20:])}")
                else:
                    print(f"  50 premiers {show(run['first50'])}")
                    print(f"  50 derniers {show(run['last50'])}")
                    print(f"  sonde       {show(run['probe'])}")

    print('\nrésumé (médiane des médianes par passe ; min, max, écart-type entre passes)')
    for (label, conversations, sends), runs in results.items():
        if args.scenario == 'conversation':
            metrics = {'20 premiers': [statistics.median(r['all'][:20]) for r in runs],
                       '20 derniers': [statistics.median(r['all'][-20:]) for r in runs]}
        else:
            metrics = {'50 derniers': [statistics.median(r['last50']) for r in runs],
                       'sonde': [statistics.median(r['probe']) for r in runs]}
        for name, values in metrics.items():
            print(f"{label} {conversations}×{sends} {name}: {show(values)}")

    if args.scenario == 'curve':
        print('\npente entre 10×20 et 50×20 (ms par 1 000 soumissions des autres conversations)')
        for label, _ in binaries:
            low = [statistics.median(r['probe']) for r in results[(label, 10, 20)]]
            high = [statistics.median(r['probe']) for r in results[(label, 50, 20)]]
            last_low = [statistics.median(r['last50']) for r in results[(label, 10, 20)]]
            last_high = [statistics.median(r['last50']) for r in results[(label, 50, 20)]]
            scale = 1000 / (50 * 20 - 10 * 20)
            print(f"{label} sonde {(statistics.median(high) - statistics.median(low)) * scale:.2f} ms ; "
                  f"50 derniers {(statistics.median(last_high) - statistics.median(last_low)) * scale:.2f} ms")
    if failed:
        raise SystemExit('au moins une passe a échoué (erreur HTTP ou soumissions manquantes)')


if __name__ == '__main__':
    main()
