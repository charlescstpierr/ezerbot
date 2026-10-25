#!/usr/bin/env python3
"""Octets que l'interface reçoit pour s'ouvrir, tels que le serveur les envoie (#303).

Lance le binaire sur une base temporaire, lit la liste ASSETS de /sw.js servi par ce
même binaire, puis demande chaque fichier comme WKWebView/Safari
(`Accept-Encoding: gzip, deflate, br`). Chaque réponse est décodée et comparée octet
pour octet à la même ressource demandée sans compression : un écart compte comme une
erreur (le Brotli est décodé par le zlib de Node, que le build de app/web demande
déjà). Les fichiers chargés seulement quand un message en a besoin (liste ON_DEMAND de
sw.js, polices référencées par les feuilles de style) sont mesurés à part ; le
service worker ne les met en cache qu'au moment où la page les demande.

    cargo build --release --locked -p grokbot-wrapper
    python3 tools/bench/ui-payload.py target/release/grokbot-wrapper --runs 3

Le chiffre est déterministe : les passes servent à montrer qu'il ne varie pas.
"""
import argparse
import gzip
import http.client
import json
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ACCEPT = "gzip, deflate, br"
TARGET_TOTAL_KIB = 1350
TARGET_JS_KIB = 310


def start(binary, db):
    process = subprocess.Popen([binary, "--port", "0", "--db", db], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    port = int(json.loads(process.stdout.readline())["url"].rsplit(":", 1)[1])
    threading.Thread(target=process.stdout.read, daemon=True).start()
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            if get(port, "/api/ping")[0] == 200:
                return process, port
        except OSError:
            time.sleep(0.05)
    process.kill()
    raise SystemExit("le serveur ne répond pas à /api/ping")


def get(port, path, encoding=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    headers = {"Host": f"127.0.0.1:{port}"}
    if encoding:
        headers["Accept-Encoding"] = encoding
    connection.request("GET", path, headers=headers)
    response = connection.getresponse()
    body = response.read()
    connection.close()
    return response.status, body, {k.lower(): v for k, v in response.getheaders()}


def decode(body, encoding):
    if encoding in ("", "identity"):
        return body
    if encoding == "gzip":
        return gzip.decompress(body)
    if encoding == "br":
        script = "process.stdout.write(require('zlib').brotliDecompressSync(require('fs').readFileSync(0)))"
        return subprocess.run(["node", "-e", script], input=body, capture_output=True, check=True).stdout
    raise ValueError(f"Content-Encoding non décodable ici : {encoding}")


def fetch(port, path):
    status, wire, headers = get(port, path, ACCEPT)
    reference_status, reference, _ = get(port, path)
    encoding = headers.get("content-encoding", "")
    error = None
    try:
        decoded = decode(wire, encoding)
    except Exception as exc:  # noqa: BLE001 - toute réponse illisible est une erreur comptée
        decoded, error = b"", str(exc)
    if status != 200 or reference_status != 200:
        error = f"statut {status}/{reference_status}"
    elif error is None and decoded != reference:
        error = "le contenu décodé diffère de la ressource non compressée"
    return {
        "path": path,
        "status": status,
        "type": headers.get("content-type", "").split(";")[0],
        "encoding": encoding or "identity",
        "transferred": len(wire),
        "decoded": len(decoded),
        "error": error,
        "body": decoded,
    }


def listed(script, name):
    match = re.search(rf"const {name} = \[(.*?)\];", script, re.S)
    return re.findall(r'"(/[^"]*)"', match.group(1)) if match else []


def local_references(text, pattern):
    return [ref for ref in re.findall(pattern, text) if ref.startswith("/") and not ref.startswith("//")]


def measure(binary):
    with tempfile.TemporaryDirectory(prefix="ui-payload-") as directory:
        process, port = start(binary, f"{directory}/w.sqlite")
        try:
            worker = get(port, "/sw.js")[1].decode()
            shell = [fetch(port, path) for path in listed(worker, "ASSETS")]
            index = next(item for item in shell if item["path"] == "/")["body"].decode()
            # iphone.js registers /sw.js, so the browser fetches it at every opening too.
            loaded = set(local_references(index, r'(?:src|href)="([^"?#]+)"')) | {"/sw.js"}
            extra = [fetch(port, path) for path in sorted(loaded - {item["path"] for item in shell})]
            later = set(listed(worker, "ON_DEMAND"))
            for item in shell:
                if item["type"] == "text/css":
                    later.update(local_references(item["body"].decode(), r'url\(\s*["\']?([^"\')]+)'))
            on_demand = [fetch(port, path) for path in sorted(later - {item["path"] for item in shell + extra})]
        finally:
            process.terminate()
            process.wait(10)
    return {"opening": shell + extra, "extra_at_opening": [item["path"] for item in extra], "on_demand": on_demand}


def totals(items):
    return {
        "files": len(items),
        "transferred": sum(item["transferred"] for item in items),
        "decoded": sum(item["decoded"] for item in items),
        "js_transferred": sum(item["transferred"] for item in items if "javascript" in item["type"]),
        "errors": sum(1 for item in items if item["error"]),
    }


def kib(n):
    return f"{n / 1024:,.0f} KiB".replace(",", " ")


def table(title, items):
    print(f"\n{title}")
    print(f"{'chemin':<42}{'type':<26}{'encodage':<10}{'transféré':>12}{'décodé':>12}  erreur")
    for item in sorted(items, key=lambda item: -item["transferred"]):
        print(f"{item['path']:<42}{item['type']:<26}{item['encoding']:<10}{item['transferred']:>12}{item['decoded']:>12}  {item['error'] or ''}")


def spread(values):
    return f"médiane={statistics.median(values):.0f} min={min(values)} max={max(values)} sd={statistics.pstdev(values):.1f} n={len(values)}"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("binary", nargs="?", default="target/release/grokbot-wrapper")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--json", action="store_true", help="une ligne JSON par passe, sans tableau")
    args = parser.parse_args()
    runs = []
    for _ in range(args.runs):
        result = measure(args.binary)
        summary = {"opening": totals(result["opening"]), "on_demand": totals(result["on_demand"]), "extra_at_opening": result["extra_at_opening"]}
        runs.append((result, summary))
        if args.json:
            print(json.dumps(summary))
    if args.json:
        return 1 if any(summary["opening"]["errors"] or summary["on_demand"]["errors"] for _, summary in runs) else 0
    result, summary = runs[-1]
    table("Ouverture : ASSETS de sw.js et tout fichier que / référence", result["opening"])
    if result["on_demand"]:
        table("À la demande : ON_DEMAND de sw.js et polices des feuilles de style", result["on_demand"])
    opening, later = summary["opening"], summary["on_demand"]
    print(f"\nAccept-Encoding: {ACCEPT}")
    print(f"fichiers à l'ouverture        {opening['files']} (hors ASSETS : {', '.join(summary['extra_at_opening']) or 'aucun'})")
    print(f"total transféré à l'ouverture {opening['transferred']} o = {kib(opening['transferred'])} (objectif ≤ {TARGET_TOTAL_KIB} KiB)")
    print(f"total décodé à l'ouverture    {opening['decoded']} o = {kib(opening['decoded'])}")
    print(f"JS transféré à l'ouverture    {opening['js_transferred']} o = {kib(opening['js_transferred'])} (objectif ≤ {TARGET_JS_KIB} KiB)")
    print(f"à la demande, transféré       {later['transferred']} o = {kib(later['transferred'])} sur {later['files']} fichiers")
    print(f"total transféré par passe     {spread([s['opening']['transferred'] for _, s in runs])}")
    errors = sum(s["opening"]["errors"] + s["on_demand"]["errors"] for _, s in runs)
    print(f"erreurs                       {errors}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
