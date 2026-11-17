"""Controlled peer checking the actual UTF-16 prepareRename origin, not a language engine."""
import json
import re
import sys
from pathlib import Path

texts = {}
log_path = Path(sys.argv[1])

def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()

def position(text, offset):
    before = text[:offset]
    return {"line": before.count("\n"), "character": len(before.rsplit("\n", 1)[-1].encode("utf-16-le")) // 2}

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    with log_path.open("a") as log:
        log.write(json.dumps(message) + "\n")
    method, params = message.get("method"), message.get("params", {})
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {"positionEncoding": "utf-16", "textDocumentSync": 1, "renameProvider": {"prepareProvider": True}}}})
    elif method == "textDocument/didOpen":
        texts[params["textDocument"]["uri"]] = params["textDocument"]["text"]
    elif method == "textDocument/didChange":
        texts[params["textDocument"]["uri"]] = params["contentChanges"][0]["text"]
    elif method == "textDocument/prepareRename":
        text = texts[params["textDocument"]["uri"]]
        pos = params["position"]
        result = None
        for match in re.finditer(r"[A-Za-z_][A-Za-z_0-9]*", text):
            start, end = position(text, match.start()), position(text, match.end())
            if start["line"] == end["line"] == pos["line"] and start["character"] <= pos["character"] < end["character"]:
                result = {"range": {"start": start, "end": end}, "placeholder": match.group()}
                break
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
