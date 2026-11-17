"""Controlled LSP peer: records cwd/rootUri/open documents, no project execution."""
import json
import os
import sys
from urllib.parse import urlparse, unquote
log_path = sys.argv[1]

def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()

while True:
    size = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        key, _, value = line.partition(b":")
        if key.lower() == b"content-length":
            size = int(value)
    message = json.loads(sys.stdin.buffer.read(size))
    with open(log_path, "a", encoding="utf-8") as output:
        output.write(json.dumps({"cwd": os.getcwd(), "message": message}) + "\n")
    method = message.get("method")
    if method == "initialize":
        assert unquote(urlparse(message["params"]["rootUri"]).path) == os.getcwd()
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {
            "positionEncoding": "utf-16", "textDocumentSync": 1, "definitionProvider": True}}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        doc = message["params"]["textDocument"]
        assert unquote(urlparse(doc["uri"]).path).startswith(os.getcwd() + "/")
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": doc["uri"], "version": doc["version"], "diagnostics": [{"range": {
                "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                "message": "controlled-project=" + os.getcwd(), "severity": 4}]}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
