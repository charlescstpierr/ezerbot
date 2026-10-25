#!/usr/bin/env python3
"""Loopback scripted OpenAI for Ezerbot verification.

Listens on 127.0.0.1 only. Tokens and replies are fixtures, never a ChatGPT account.
The catalogue matches crates/wrapper/tests/support/mod.rs (gpt-test plus a mini model
so automatic memory can see a small model). Prints one JSON line, then serves.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import struct
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def _sse(events: list[dict]) -> bytes:
    chunks = []
    for event in events:
        chunks.append(f"event: {event['type']}\ndata: {json.dumps(event, ensure_ascii=False)}\n\n")
    return "".join(chunks).encode()


MODELS = {
    "models": [
        {
            "slug": "gpt-test",
            "display_name": "GPT Test",
            "description": "Modèle de vérification",
            "visibility": "list",
            "default_reasoning_level": "medium",
            "default_reasoning_summary": "none",
            "supported_reasoning_levels": [
                {"effort": "low", "description": "Léger"},
                {"effort": "medium", "description": "Équilibré"},
            ],
            "service_tiers": [{"id": "priority", "name": "Fast", "description": "2x"}],
        },
        {
            "slug": "gpt-test-mini",
            "display_name": "GPT Test Mini",
            "description": "Petit modèle de vérification",
            "visibility": "list",
            "supported_reasoning_levels": [{"effort": "low", "description": "Léger"}],
        },
        {"slug": "internal", "display_name": "Internal", "visibility": "hide"},
    ]
}

def _png(width: int, height: int) -> bytes:
    """A small gradient PNG, so the generated picture is visible in a screenshot."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows += bytes((40 + 180 * x // width, 90 + 100 * y // height, 200 - 120 * x // width))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(rows))) + chunk(b"IEND", b"")


# Les prix portent un échappement \u20ac : la Page doit afficher « 9 € », pas « 9u20ac ».
PAGE_SOURCE = r"""export default function Forfaits() {
  const [offre, setOffre] = useState("Essentiel");
  const lignes = [
    ["Essentiel", "9 \u20ac"],
    ["Confort", "19 \u20ac"],
    ["Famille", "29 \u20ac"],
  ];
  return (
    <article>
      <h1>Trois forfaits</h1>
      <p>Tout &lt; 30 &#8364; par mois</p>
      <p>Offre choisie : {offre}</p>
      <table>
        <thead>
          <tr>
            <th>Offre</th>
            <th>Prix</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {lignes.map((ligne) => (
            <tr>
              <td>{ligne[0]}</td>
              <td>{ligne[1]}</td>
              <td>
                <button onClick={() => setOffre(ligne[0])}>Choisir</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </article>
  );
}
"""

# Deux programmes qui attendent une ligne : l'un la lit et la répète, l'autre demande un mot de passe.
TASKS = (
    ("lance le script de saisie", "verify-task-saisie", "printf 'Continuer ? [oui/non] '; read -r reponse; echo \"reponse=$reponse\""),
    ("lance le script du mot de passe", "verify-task-mdp", "printf 'Mot de passe : '; read -r secret; echo fini"),
)

IMAGE_B64 = base64.b64encode(_png(192, 192)).decode()
# gpt-image-1.5 renders 1536x1024 and 1024x1536 for the wide and tall frames; the fixtures keep those ratios.
IMAGE_SIZES = {
    "1536x1024": base64.b64encode(_png(288, 192)).decode(),
    "1024x1536": base64.b64encode(_png(192, 288)).decode(),
}


def _image_ids(output: object) -> list[str]:
    """Image attachment ids found in a list_delegation_context result."""
    catalog = output
    if isinstance(catalog, str):
        # The wrapper fences tool results as <external_data>"<json string>"</external_data>.
        start, end = catalog.find("<external_data"), catalog.rfind("</external_data>")
        if start != -1 and end != -1:
            catalog = catalog[catalog.index(">", start) + 1 : end]
    try:
        while isinstance(catalog, str):
            catalog = json.loads(catalog)
    except json.JSONDecodeError:
        return []
    found: list[str] = []

    def walk(node: object) -> None:
        if isinstance(node, dict):
            if node.get("kind") == "image" and isinstance(node.get("id"), str):
                found.append(node["id"])
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(catalog)
    return found

USAGE = {
    "plan_type": "plus",
    "rate_limit": {
        "primary_window": {"used_percent": 35, "limit_window_seconds": 18000, "reset_at": 4102444800},
        "secondary_window": {"used_percent": 10, "limit_window_seconds": 604800, "reset_at": 4102531200},
    },
}


def _last_user_text(items: list) -> str:
    """The text of the newest user message: earlier turns of a thread must not re-trigger a tool."""
    for item in reversed(items):
        if isinstance(item, dict) and item.get("role") == "user":
            return json.dumps(item.get("content"), ensure_ascii=False).lower()
    return ""


def yolo_turn(items: list) -> bytes | None:
    """Mode YOLO : une commande puis une écriture de fichier, dans le dossier de travail jeté du run."""
    work = os.environ.get("EZERBOT_VERIFY_FAKE_WORK")
    if not work or not items or not isinstance(items[-1], dict):
        return None
    last = items[-1]
    if last.get("type") == "function_call_output":
        said = {
            "verify-shell": "Voici le contenu de votre dossier Documents.",
            "verify-write": "Le fichier Notes-du-jour.txt est écrit.",
        }.get(last.get("call_id"))
        if said is None:
            return None
        return _sse([{"type": "response.output_text.delta", "delta": said}, {"type": "response.completed"}])
    text = _last_user_text(items)
    if "liste mon dossier documents" in text:
        name, arguments, call_id = "run_shell", {"command": "ls -1", "cwd": work, "timeout_ms": 5000, "output_limit": 4096}, "verify-shell"
    elif "écris mes notes du jour" in text:
        name, call_id = "write_file", "verify-write"
        arguments = {
            "path": os.path.join(work, "Notes-du-jour.txt"),
            "content": "Réunion du matin : valider le budget.\nAppeler le fournisseur avant midi.\n",
            "expected_sha256": None,
        }
    else:
        return None
    call = {"id": f"fc-{call_id}", "type": "function_call", "status": "completed", "call_id": call_id, "name": name, "arguments": json.dumps(arguments)}
    return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])


def recall_turn(items: list) -> bytes | None:
    """Le modèle cherche dans sa mémoire (`recall_memory`) puis répond ; le contexte d'amorce n'y compte pas."""
    if not items or not isinstance(items[-1], dict):
        return None
    if items[-1].get("type") == "function_call_output":
        if items[-1].get("call_id") != "verify-recall":
            return None
        return _sse([{"type": "response.output_text.delta", "delta": "J’ai retrouvé le souvenir demandé."}, {"type": "response.completed"}])
    if "cherche dans ta mémoire" not in _last_user_text(items):
        return None
    call = {"id": "fc-verify-recall", "type": "function_call", "status": "completed", "call_id": "verify-recall", "name": "recall_memory", "arguments": json.dumps({"query": "port Boréal", "limit": 5})}
    return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])


CAPTURE_MARKER = "PREUVE-CAPTURES"


def _strings(node: object):
    if isinstance(node, str):
        yield node
    elif isinstance(node, dict):
        for value in node.values():
            yield from _strings(value)
    elif isinstance(node, list):
        for value in node:
            yield from _strings(value)


def _call(call_id: str, name: str, arguments: dict) -> bytes:
    call = {"id": f"fc-{call_id}", "type": "function_call", "status": "completed", "call_id": call_id, "name": name, "arguments": json.dumps(arguments)}
    return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])


def capture_turn(payload: dict, instructions: str, items: list) -> bytes | None:
    """Un sous-bot dessine une image avant puis une après, tente une capture inventée, puis termine avec les vraies."""
    asked = CAPTURE_MARKER in json.dumps(items, ensure_ascii=False)
    outputs = [item for item in items if isinstance(item, dict) and item.get("type") == "function_call_output"]
    last = outputs[-1].get("call_id") if outputs else None
    if "You are executing a Task" in instructions and asked:
        shot = lambda step: next((m for o in outputs if o.get("call_id") == step for m in re.findall(r"attachment_id[^A-Za-z0-9]+([A-Za-z0-9_.-]+)", str(o.get("output")))), "")
        if last is None:
            return _call("verify-cap-before", "generate_image", {"description": "la page avant le changement", "aspect_ratio": "16:9"})
        if last == "verify-cap-before":
            return _call("verify-cap-after", "generate_image", {"description": "la page après le changement", "aspect_ratio": "9:16"})
        if last == "verify-cap-after":
            return _call("verify-cap-bad", "complete_task", {"summary": "Page modifiée.", "captures": [{"id": "image-inventee", "label": "avant", "caption": "Capture inventée"}]})
        if last == "verify-cap-bad":
            captures = [
                {"id": shot("verify-cap-before"), "label": "avant", "caption": "Page avant le changement"},
                {"id": shot("verify-cap-after"), "label": "après", "caption": "Page après le changement"},
            ]
            return _call("verify-cap-done", "complete_task", {"summary": "Page modifiée, voir les captures.", "captures": captures})
        return _sse([{"type": "response.output_text.delta", "delta": "Terminé."}, {"type": "response.completed"}])
    if last == "verify-cap-delegate":
        received = next((line for text in _strings(payload) for line in text.splitlines() if "child_results" in line), "")
        return _sse([{"type": "response.output_text.delta", "delta": f"Résultat reçu du sous-bot : {received}"}, {"type": "response.completed"}])
    if last is None and "délègue avec captures" in _last_user_text(items):
        args = {"child_bot": "child", "objective": f"{CAPTURE_MARKER} : photographie l'écran avant puis après", "selected_context": {"texts": [], "entry_ids": [], "fact_ids": [], "skill_ids": []}}
        return _call("verify-cap-delegate", "delegate", args)
    return None


def routine_turn(items: list) -> bytes | None:
    """Passage de routine : « rien_a_signaler » puis complete_task, ou un résultat tout de suite."""
    if not items or not isinstance(items[-1], dict):
        return None
    usage = {"input_tokens": 1, "output_tokens": 2}
    last = items[-1]
    if last.get("type") == "function_call_output":
        if last.get("call_id") != "verify-quiet":
            return None
        name, call_id, arguments = "complete_task", "verify-quiet-done", {"summary": "Rien de neuf."}
    elif "preuve-rien-a-signaler" in _last_user_text(items):
        name, call_id, arguments = "rien_a_signaler", "verify-quiet", {}
    elif "preuve-resultat" in _last_user_text(items):
        name, call_id, arguments = "complete_task", "verify-result", {"summary": "Trois nouveaux courriels."}
    else:
        return None
    call = {"id": f"fc-{call_id}", "type": "function_call", "status": "completed", "call_id": call_id, "name": name, "arguments": json.dumps(arguments)}
    return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed", "response": {"usage": usage}}])


def parent_message(instructions: str) -> str:
    """Le message parent d'un sous-fil : bloc de données externes JSON dans les instructions."""
    for found in re.finditer(r"<external_data[^>]*>\n(.*?)\n</external_data>", instructions, re.S):
        try:
            text = json.loads(found.group(1))
        except json.JSONDecodeError:
            continue
        if isinstance(text, str) and "Message parent" in text and "pour répondre :\n" in text:
            return text.split("pour répondre :\n", 1)[1].strip()
    return ""


def response_body(raw: bytes) -> bytes:
    text = raw.decode(errors="replace")
    try:
        payload = json.loads(text) if text else {}
    except json.JSONDecodeError:
        payload = {}
    instructions = str(payload.get("instructions") or "")
    tools = payload.get("tools") if isinstance(payload.get("tools"), list) else []
    drawing = next((tool for tool in tools if isinstance(tool, dict) and tool.get("type") == "image_generation"), None)
    if drawing:
        item = {
            "type": "image_generation_call",
            "status": "completed",
            "result": IMAGE_SIZES.get(drawing.get("size"), IMAGE_B64),
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": item},
                {"type": "response.completed", "response": {"output": [item]}},
            ]
        )
    # Avant le titre : les instructions d'un passage de routine contiennent le mot « titre ».
    routine = routine_turn(payload.get("input") if isinstance(payload.get("input"), list) else [])
    if routine is not None:
        return routine
    if instructions.startswith("Extraction de mémoire automatique."):
        return _sse(
            [
                {"type": "response.output_text.delta", "delta": '{"facts":[]}'},
                {"type": "response.completed"},
            ]
        )
    if "titre" in instructions.lower():
        return _sse(
            [
                {"type": "response.output_text.delta", "delta": "« Vérification visuelle. »"},
                {"type": "response.completed"},
            ]
        )
    parent = parent_message(instructions)
    if parent:
        return _sse(
            [
                {"type": "response.created"},
                {"type": "response.output_text.delta", "delta": f"En tenant compte du message parent : « {parent[:80]} »"},
                {"type": "response.completed", "response": {"usage": {"input_tokens": 1, "output_tokens": 2}}},
            ]
        )
    items = payload.get("input") if isinstance(payload.get("input"), list) else []
    yolo = yolo_turn(items)
    if yolo is not None:
        return yolo
    captured = capture_turn(payload, instructions, items)
    if captured is not None:
        return captured
    recall = recall_turn(items)
    if recall is not None:
        return recall
    outputs = [item for item in items if isinstance(item, dict) and item.get("type") == "function_call_output"]
    if outputs and outputs[-1].get("call_id") == "verify-context":
        call = {
            "id": "fc-verify-reference",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-reference",
            "name": "generate_image",
            "arguments": json.dumps(
                {
                    "description": "le même style, en large",
                    "aspect_ratio": "16:9",
                    "reference_image_paths": _image_ids(outputs[-1].get("output"))[:1],
                }
            ),
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": call},
                {"type": "response.completed"},
            ]
        )
    if outputs and outputs[-1].get("call_id") == "verify-shotpage-write":
        # La Page vient d'être écrite : la photographier avec l'identifiant que write_page a rendu.
        found = re.search(r"page_[0-9a-f]{64}", str(outputs[-1].get("output")))
        call = {
            "id": "fc-verify-shotpage",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-shotpage",
            "name": "screenshot_page",
            "arguments": json.dumps({"target": found.group(0) if found else "page_introuvable", "theme": "light" if "en clair" in _last_user_text(items) else "dark"}),
        }
        return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])
    # Petit changement d'une Page déjà écrite : edit_page, l'id étant celui de write_page pour ce bot et ce fichier.
    page_id = "page_" + hashlib.sha256(b"grokbot\0forfaits.page.tsx").hexdigest()
    last_text = "" if items and isinstance(items[-1], dict) and items[-1].get("type") == "function_call_output" else _last_user_text(items)
    for needle, call_id, before, after in (
        ("change le prix de confort", "verify-edit", '["Confort", "19 \\u20ac"]', '["Confort", "21 \\u20ac"]'),
        ("change tous les prix", "verify-edit-double", "\\u20ac", "euros"),
    ):
        if needle in last_text:
            call = {
                "id": f"fc-{call_id}",
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": "edit_page",
                "arguments": json.dumps({"id": page_id, "before": before, "after": after}),
            }
            return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])
    if outputs:
        last = outputs[-1].get("call_id")
        if last == "verify-late":
            # Tour qui continue sans attendre (« En attente »), ou tour de suite d'une décision tardive.
            waiting = "En attente" in str(outputs[-1].get("output"))
            done = "J’attends son accord pour envoyer le résumé." if waiting else "Tu as approuvé : envoyer le résumé. C’est parti. Le résumé est envoyé."
        else:
            done = {
                "verify-page": "Voici la Page.",
                "verify-edit": "La Page est modifiée.",
                "verify-edit-double": "Le passage n’est pas unique.",
                "verify-shotpage": "Voici la photo de la Page.",
                "verify-shotnet": "La photo a été refusée.",
            }.get(last) or ("La tâche tourne." if str(last).startswith("verify-task") else "Voici l’image.")
        return _sse(
            [
                {"type": "response.output_text.delta", "delta": done},
                {"type": "response.completed"},
            ]
        )
    for needle, call_id, command in TASKS:
        if needle in json.dumps(items, ensure_ascii=False).lower():
            call = {
                "id": f"fc-{call_id}",
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": "run_shell",
                "arguments": json.dumps({"command": command, "cwd": "/tmp", "timeout_ms": 30000, "output_limit": 131072, "background": True}),
            }
            return _sse(
                [
                    {"type": "response.output_item.done", "item": call},
                    {"type": "response.completed"},
                ]
            )
    if "envoie le résumé" in json.dumps(items, ensure_ascii=False).lower():
        call = {
            "id": "fc-verify-late",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-late",
            "name": "run_shell",
            "arguments": json.dumps({"command": "printf 'Résumé envoyé'", "cwd": "/tmp", "timeout_ms": 30000, "output_limit": 4096}),
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": call},
                {"type": "response.completed"},
            ]
        )
    if "photographie internet" in _last_user_text(items):
        call = {
            "id": "fc-verify-shotnet",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-shotnet",
            "name": "screenshot_page",
            "arguments": json.dumps({"target": "https://example.com/"}),
        }
        return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])
    if "photographie la page des forfaits" in _last_user_text(items):
        call = {
            "id": "fc-verify-shotpage-write",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-shotpage-write",
            "name": "write_page",
            "arguments": json.dumps({"name": "forfaits.page.tsx", "title": "Trois forfaits", "source": PAGE_SOURCE}),
        }
        return _sse([{"type": "response.output_item.done", "item": call}, {"type": "response.completed"}])
    if "une page des forfaits" in json.dumps(items, ensure_ascii=False).lower():
        call = {
            "id": "fc-verify-page",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-page",
            "name": "write_page",
            "arguments": json.dumps({"name": "forfaits.page.tsx", "title": "Trois forfaits", "source": PAGE_SOURCE}),
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": call},
                {"type": "response.completed"},
            ]
        )
    if "le même style" in json.dumps(items, ensure_ascii=False).lower():
        call = {
            "id": "fc-verify-context",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-context",
            "name": "list_delegation_context",
            "arguments": "{}",
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": call},
                {"type": "response.completed"},
            ]
        )
    if "dessine" in json.dumps(items, ensure_ascii=False).lower():
        call = {
            "id": "fc-verify-image",
            "type": "function_call",
            "status": "completed",
            "call_id": "verify-image",
            "name": "generate_image",
            "arguments": json.dumps({"description": "un phare au crépuscule", "aspect_ratio": "1:1"}),
        }
        return _sse(
            [
                {"type": "response.output_item.done", "item": call},
                {"type": "response.completed"},
            ]
        )
    if "cite tes sources" in json.dumps(items, ensure_ascii=False).lower():
        # Trois lectures de la bibliothèque, citées par leur numéro, et un [7] que rien ne soutient.
        delta = "Le loyer est de 1 200 dollars par mois [1], et le budget le confirme [2]. Le bail fixe le paiement au premier du mois [3]. Un ancien chiffre circule encore sans source [7]."
        return _sse(
            [
                {"type": "response.created"},
                {"type": "response.output_text.delta", "delta": delta},
                {"type": "response.completed", "response": {"usage": {"input_tokens": 1, "output_tokens": 2}}},
            ]
        )
    return _sse(
        [
            {"type": "response.created"},
            {"type": "response.output_text.delta", "delta": "Bon"},
            {"type": "response.output_text.delta", "delta": "jour **vous**"},
            {
                "type": "response.completed",
                "response": {"usage": {"input_tokens": 1, "output_tokens": 2}},
            },
        ]
    )


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        return

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def _read(self) -> bytes:
        length = int(self.headers.get("Content-Length") or "0")
        if length > 2_000_000:
            return b""
        return self.rfile.read(length) if length else b""

    def do_GET(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0]
        if path in ("/v1/models", "/backend-api/codex/models"):
            self._send(200, json.dumps(MODELS).encode(), "application/json")
        elif path == "/backend-api/wham/usage":
            self._send(200, json.dumps(USAGE).encode(), "application/json")
        elif path == "/jwks":
            self._send(200, b'{"keys":[]}', "application/json")
        else:
            self._send(404, b"", "text/plain")

    def do_POST(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0]
        raw = self._read()
        if path in ("/v1/responses", "/backend-api/codex/responses"):
            self._send(200, response_body(raw), "text/event-stream")
        elif path == "/token":
            body = json.dumps(
                {
                    "access_token": "fake-access-verify",
                    "refresh_token": "fake-refresh-verify",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                }
            ).encode()
            self._send(200, body, "application/json")
        elif path == "/revoke":
            self._send(200, b"", "application/json")
        else:
            self._send(404, b"", "text/plain")


def main() -> None:
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    host, port = server.server_address
    if host != "127.0.0.1" or port == 0:
        raise SystemExit("le faux OpenAI doit écouter 127.0.0.1 sur un port éphémère")
    print(json.dumps({"base": f"http://127.0.0.1:{port}"}), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
