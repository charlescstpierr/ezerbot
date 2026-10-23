#!/usr/bin/env python3
"""Loopback scripted OpenAI for Ezerbot verification.

Listens on 127.0.0.1 only. Tokens and replies are fixtures, never a ChatGPT account.
The catalogue matches crates/wrapper/tests/support/mod.rs (gpt-test plus a mini model
so automatic memory can see a small model). Prints one JSON line, then serves.
"""

from __future__ import annotations

import base64
import json
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
    if outputs:
        last = outputs[-1].get("call_id")
        done = "Voici la Page." if last == "verify-page" else "La tâche tourne." if str(last).startswith("verify-task") else "Voici l’image."
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
