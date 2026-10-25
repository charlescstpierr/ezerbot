#!/usr/bin/env python3
"""Coût de POST /api/conversations/delete sur un fil qui a une image jointe (#330).

Depuis #330, supprimer un fil efface ses images : la suppression parcourt les
racines de stockage, donc toute la base. Ce script mesure ce coût sur une base
réaliste, avant/après, avec deux binaires release.

Usage :
    python3 tools/bench-suppression-fil.py AVANT APRES [passes] [fils] [messages]

AVANT et APRES sont des binaires grokbot-wrapper (cargo build --release).
Chaque passe crée une base neuve : `fils` conversations de `messages` messages,
chacune avec une image distincte, puis supprime la moitié des fils un par un.
Les passes alternent AVANT, APRES, AVANT... pour neutraliser la dérive de la
machine. Le script imprime, par côté : médiane, min, max, écart-type et n des
suppressions en ms, et combien d'URL d'images supprimées répondent encore 200.
"""
import json
import os
import shutil
import signal
import statistics as st
import subprocess
import sys
import tempfile
import time
from local_api import LaunchIdentity, LocalEndpoint

PNG = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01bench"
BODY = "Peux-tu m'aider à préparer la réunion de demain ? " * 4


def req(endpoint, method, path, body=None, raw=None):
    headers = {"Content-Type": "image/png" if raw is not None else "application/json"}
    data = raw if raw is not None else (json.dumps(body) if body is not None else None)
    status, content, _ = endpoint.request(method, path, body=data, headers=headers, timeout=300)
    return status, content


def start(binary, db):
    identity = LaunchIdentity()
    p = identity.spawn([binary, "--port", "0", "--db", db], stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True)
    endpoint = LocalEndpoint(identity, json.loads(p.stdout.readline())["url"])
    while True:
        try:
            if req(endpoint, "GET", "/api/ping")[0] == 200:
                return p, endpoint
        except OSError:
            time.sleep(0.02)


def stop(p):
    p.send_signal(signal.SIGTERM)
    try:
        p.wait(5)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()


def one_pass(binary, threads, messages):
    directory = tempfile.mkdtemp(prefix="bench-330-")
    p, endpoint = start(binary, os.path.join(directory, "w.sqlite"))
    try:
        seeded = []
        for c in range(threads):
            s, b = req(endpoint, "POST", "/api/conversations", {})
            assert s == 200, (s, b)
            cid = json.loads(b)["conversation"]["id"]
            s, b = req(endpoint, "POST", "/api/attachments", raw=PNG + c.to_bytes(4, "big"))
            assert s == 200, (s, b)
            image = json.loads(b)
            for m in range(messages):
                attachments = [image["id"]] if m == 0 else []
                s, b = req(endpoint, "POST", "/api/send", {"conversation_id": cid, "message_id": f"c{c}m{m}",
                                                       "body": BODY, "attachments": attachments})
                assert s == 200, (s, b)
            seeded.append((cid, image["url"]))
        times, still_served = [], 0
        for cid, url in seeded[::2]:
            t = time.perf_counter()
            s, b = req(endpoint, "POST", "/api/conversations/delete", {"id": cid})
            times.append((time.perf_counter() - t) * 1000)
            assert s == 200, (s, b)
            still_served += req(endpoint, "GET", url)[0] == 200
        size = os.path.getsize(os.path.join(directory, "w.sqlite"))
        return times, still_served, size
    finally:
        stop(p)
        shutil.rmtree(directory, ignore_errors=True)


def summary(xs):
    return (f"med={st.median(xs):.2f} min={min(xs):.2f} max={max(xs):.2f} "
            f"sd={st.pstdev(xs):.2f} n={len(xs)}")


def main():
    before, after = sys.argv[1], sys.argv[2]
    passes = int(sys.argv[3]) if len(sys.argv) > 3 else 3
    threads = int(sys.argv[4]) if len(sys.argv) > 4 else 40
    messages = int(sys.argv[5]) if len(sys.argv) > 5 else 25
    results = {"AVANT": ([], [], 0, 0), "APRES": ([], [], 0, 0)}
    for i in range(passes):
        for side, binary in (("AVANT", before), ("APRES", after)):
            times, served, size = one_pass(binary, threads, messages)
            all_times, medians, total_served, _ = results[side]
            results[side] = (all_times + times, medians + [st.median(times)], total_served + served, size)
            print(f"passe {i + 1} {side}: {summary(times)} images_encore_servies={served} base={size}o",
                  flush=True)
    for side, (all_times, medians, served, size) in results.items():
        print(f"{side} toutes suppressions ms: {summary(all_times)}")
        print(f"{side} médianes par passe ms: {summary(medians)}")
        print(f"{side} images supprimées encore servies: {served}/{len(all_times)}")


if __name__ == "__main__":
    main()
