#!/usr/bin/env python3
"""Loopback scripted OpenAI for Ezerbot verification.

Listens on 127.0.0.1 only. Tokens and replies are fixtures, never a ChatGPT account.
The catalogue matches crates/wrapper/tests/support/mod.rs (gpt-test plus a mini model
so automatic memory can see a small model). Prints one JSON line, then serves.
"""

from __future__ import annotations

import json
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
