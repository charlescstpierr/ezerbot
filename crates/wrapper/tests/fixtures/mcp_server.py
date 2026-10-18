#!/usr/bin/env python3
"""Local stdio fixture. No network; optional release-file concurrency barrier."""
import json
import os
import sys
import time
import threading

mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
initialized = False
changed = False
failed_list_once = False
call_count = 0
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
        if mode == "slow-start":
            time.sleep(2)
        result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {"listChanged": True}}, "serverInfo": {"name": "Atelier local", "version": "1"}}
    elif method == "tools/list" and initialized:
        if mode == "rpc-list" and changed and not failed_list_once:
            failed_list_once = True
            response["error"] = {"code": -32001, "message": "Catalogue provisoirement indisponible"}
            print(json.dumps(response), flush=True)
            continue
        if mode == "slow-list" and changed:
            time.sleep(2)
        result = {"tools": [
            {"name": "echo", "description": "Catalogue actualisé." if changed else "Répéter un texte sans effet externe.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
            {"name": "addition", "description": "Additionner deux nombres.", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}}
        ]}
        if mode == "change-during-list" and not changed:
            changed = True
            print(json.dumps({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}), flush=True)
    elif method == "tools/call" and initialized:
        call_count += 1
        name = request["params"]["name"]
        args = request["params"]["arguments"]
        if name == "echo":
            text = args.get("text", "")
            if not isinstance(text, str):
                response["error"] = {"code": -32602, "message": "Texte requis"}
                print(json.dumps(response), flush=True)
                continue
            if text == "wait-for-release":
                deadline = time.monotonic() + 8
                while not os.path.exists(os.environ["CALL_GATE"]) and time.monotonic() < deadline:
                    time.sleep(0.01)
            if text == "call-count":
                text = str(call_count)
            if text == "slow-two-seconds":
                time.sleep(2)
            if text == "crash":
                sys.exit(9)
            if text == "crash-after":
                def late_exit():
                    time.sleep(0.05)
                    os._exit(9)
                threading.Thread(target=late_exit, daemon=True).start()
            if text == "timeout":
                time.sleep(20)
            if text.startswith("slow-"):
                time.sleep(0.15)
            if text.startswith("env:"):
                text = os.environ.get(text[4:], "absent")
            if text == "secret":
                text = "private-value-56789" if mode == "foreign" else os.environ.get("TEST_TOKEN", "absent")
            result = {"content": [None] if mode == "invalid-content" else [{"type": "text", "text": text}], "isError": text == "erreur"}
            if text == "typed":
                result["structuredContent"] = {"port": 3000, "flag": False}
            if text == "number-secret":
                result["structuredContent"] = {"value": int(os.environ["NUMERIC_TOKEN"])}
        elif name == "addition":
            result = {"content": [{"type": "text", "text": str(args.get("a", 0) + args.get("b", 0))}]}
        else:
            response["error"] = {"code": -32602, "message": "Outil inconnu"}
            result = None
        if mode in ("notifications", "slow-list", "rpc-list"):
            changed = True
            print(json.dumps({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "Appel terminé"}}), flush=True)
    else:
        response["error"] = {"code": -32601, "message": "Méthode inconnue"}
        result = None
    if "error" not in response:
        response["result"] = result
    print(json.dumps(response), flush=True)
