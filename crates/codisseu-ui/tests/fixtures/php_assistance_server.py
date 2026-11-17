"""Controlled editor transport; synthetic docs are test data, not PHP analysis."""
import json
import sys

log_path = sys.argv[1]
documents = {}


def send(message):
    body = json.dumps(message).encode()
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
    result = None
    if method == "initialize":
        result = {"capabilities": {
            "positionEncoding": "utf-16", "textDocumentSync": 1,
            "hoverProvider": True, "completionProvider": {},
            "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
        }}
    elif method == "textDocument/hover":
        position = message["params"]["position"]
        result = {"contents": {"kind": "plaintext", "value":
            f"Fixture method documentation at {position['line']}:{position['character']}\n"
            "public function greet(string $name): string"}}
    elif method == "textDocument/completion":
        text = documents[message["params"]["textDocument"]["uri"]]
        position = message["params"]["position"]
        line = text.split("\n")[position["line"]]
        prefix = line.encode("utf-16-le")[:position["character"] * 2].decode("utf-16-le")
        result = [{"label": "Stormy", "insertText": "Stormy", "kind": 6}] if prefix.endswith("'St") else []
    elif method == "textDocument/signatureHelp":
        result = {"signatures": [{"label": "greet(string $name): string",
            "parameters": [{"label": "string $name"}]}],
            "activeSignature": 0, "activeParameter": 0}
    elif method == "exit":
        sys.exit(0)
    elif method == "textDocument/didOpen":
        documents[message["params"]["textDocument"]["uri"]] = message["params"]["textDocument"]["text"]
        continue
    elif method == "textDocument/didChange":
        documents[message["params"]["textDocument"]["uri"]] = message["params"]["contentChanges"][-1]["text"]
        continue
    elif method != "shutdown":
        continue
    send({"jsonrpc": "2.0", "id": message["id"], "result": result})
