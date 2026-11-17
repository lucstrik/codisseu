"""Empty controlled LSP: exercises Codisseu's Rust Laravel adapter, not PHP semantics."""
import json
import sys


def send(request_id, result):
    body = json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}).encode()
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
    if len(sys.argv)>1:
        with open(sys.argv[1], 'a', encoding='utf-8') as log:
            log.write(json.dumps(message)+'\n')
    method = message.get("method")
    if method == "initialize":
        send(message["id"], {"capabilities": {
            "positionEncoding": "utf-16", "textDocumentSync": 1,
            "completionProvider": {}, "definitionProvider": True,
        }})
    elif method in ("textDocument/completion", "textDocument/definition"):
        send(message["id"], [])
    elif method == "shutdown":
        send(message["id"], None)
    elif method == "exit":
        sys.exit(0)
