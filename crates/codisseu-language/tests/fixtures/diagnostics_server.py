"""Controlled diagnostics transport: tag parsing/version/clear, never product data."""
import json
import sys
log_path = sys.argv[1]
def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        key, _, value = line.partition(b":")
        if key.lower() == b"content-length":
            length = int(value)
    message = json.loads(sys.stdin.buffer.read(length))
    with open(log_path, "a", encoding="utf-8") as output:
        output.write(json.dumps(message) + "\n")
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {
            "positionEncoding": "utf-16", "textDocumentSync": 1}}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        doc = message["params"]["textDocument"]
        diagnostics = []
        if doc["version"] == 1:
            for i, tags in enumerate(([1], [2], [1, 2], [1, "bad"], [1]*9, None, [1, 4294967296], [1.0], [True])):
                diagnostic = {"range": {"start": {"line": 0, "character": 3},
                    "end": {"line": 0, "character": 7}}, "message": str(i), "severity": 4}
                if tags is not None:
                    diagnostic["tags"] = tags
                diagnostics.append(diagnostic)
            diagnostics.append({"range": {"start": {"line": 0, "character": 1},
                "end": {"line": 0, "character": 7}}, "message": "surrogate", "tags": [1]})
            diagnostics.append({"range": {"start": {"line": 0, "character": 7},
                "end": {"line": 0, "character": 3}}, "message": "inverted", "tags": [1]})
        else:
            send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                "uri": doc["uri"], "version": 1, "diagnostics": [{"range": {
                    "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "message": "stale", "tags": [1]}]}})
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": doc["uri"], "version": doc["version"], "diagnostics": diagnostics}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
