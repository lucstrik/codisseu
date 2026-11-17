"""Controlled protocol fixture: silent hover or absent capability, never product data."""
import json
import sys

mode, log_path = sys.argv[1:]

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
            "positionEncoding": "utf-16", "textDocumentSync": 1,
            "hoverProvider": mode != "unsupported"
        }}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
    # Hover is deliberately unanswered. Tests must observe a real cancel frame
    # and the real worker deadline, not manufacture a timeout event.
