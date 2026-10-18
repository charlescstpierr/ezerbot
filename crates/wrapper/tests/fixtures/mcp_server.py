#!/usr/bin/env python3
"""Deterministic local stdio MCP fixture. No network or file effects."""
import json
import os
import sys
import time

mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
initialized = False
changed = False
if mode == "no-read":
    time.sleep(20)
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if mode == "crash":
        sys.exit(7)
    if mode == "timeout":
        time.sleep(20)
    if mode == "bad-json":
        print("invalid-json", flush=True)
        continue
    if method == "notifications/initialized":
        initialized = True
        continue
    if "id" not in request:
        continue
    response = {"jsonrpc": "2.0", "id": request["id"]}
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {"listChanged": True}}, "serverInfo": {"name": "Atelier local", "version": "1"}}
    elif method == "tools/list" and initialized:
        result = {"tools": [
            {"name": "echo", "description": "Catalogue actualisé." if changed else "Répéter un texte sans effet externe.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
            {"name": "addition", "description": "Additionner deux nombres.", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}}
        ]}
    elif method == "tools/call" and initialized:
        name = request["params"]["name"]
        args = request["params"]["arguments"]
        if name == "echo":
            text = args.get("text", "")
            if text == "crash":
                sys.exit(9)
            if text == "timeout":
                time.sleep(20)
            if text == "secret":
                text = os.environ.get("TEST_TOKEN", "absent")
            result = {"content": [{"type": "text", "text": text}], "isError": False}
        elif name == "addition":
            result = {"content": [{"type": "text", "text": str(args.get("a", 0) + args.get("b", 0))}]}
        else:
            response["error"] = {"code": -32602, "message": "Outil inconnu"}
            result = None
        if mode == "notifications":
            changed = True
            print(json.dumps({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "Appel terminé"}}), flush=True)
    else:
        response["error"] = {"code": -32601, "message": "Méthode inconnue"}
        result = None
    if "error" not in response:
        response["result"] = result
    print(json.dumps(response), flush=True)
