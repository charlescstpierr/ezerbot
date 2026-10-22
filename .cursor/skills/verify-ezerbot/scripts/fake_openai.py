#!/usr/bin/env python3
"""Loopback scripted OpenAI for Ezerbot verification.

Listens on 127.0.0.1 only. Tokens and replies are fixtures, never a ChatGPT account.
The catalogue matches crates/wrapper/tests/support/mod.rs (gpt-test plus a mini model
so automatic memory can see a small model). Prints one JSON line, then serves.
"""

from __future__ import annotations

import base64
import json
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

IMAGE_B64 = base64.b64encode(_png(192, 192)).decode()

USAGE = {
    "plan_type": "plus",
    "rate_limit": {
        "primary_window": {"used_percent": 35, "limit_window_seconds": 18000, "reset_at": 4102444800},
        "secondary_window": {"used_percent": 10, "limit_window_seconds": 604800, "reset_at": 4102531200},
    },
}


def response_body(raw: bytes) -> bytes:
    text = raw.decode(errors="replace")
    try:
        payload = json.loads(text) if text else {}
    except json.JSONDecodeError:
        payload = {}
    instructions = str(payload.get("instructions") or "")
    tools = payload.get("tools") if isinstance(payload.get("tools"), list) else []
    if any(isinstance(tool, dict) and tool.get("type") == "image_generation" for tool in tools):
        item = {"type": "image_generation_call", "status": "completed", "result": IMAGE_B64}
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
    items = payload.get("input") if isinstance(payload.get("input"), list) else []
    outputs = [item for item in items if isinstance(item, dict) and item.get("type") == "function_call_output"]
    if outputs:
        done = "Voici la Page." if outputs[-1].get("call_id") == "verify-page" else "Voici l’image."
        return _sse(
            [
                {"type": "response.output_text.delta", "delta": done},
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
