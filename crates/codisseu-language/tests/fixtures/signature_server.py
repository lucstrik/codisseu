"""Controlled signatureHelp transport failures; never used by the product."""
import json
import sys
mode, log_path = sys.argv[1:]
pending = []
def log(value):
    with open(log_path, 'a', encoding='utf-8') as out:
        out.write(json.dumps(value) + '\n')
def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode() + body)
    sys.stdout.buffer.flush()
def reply(request_id):
    result = {'signatures': [{'label': 'f(🙂: string, n: number)', 'parameters': [{'label': [2, 12]}, {'label': 'n: number'}], 'activeParameter': 1}], 'activeSignature': 0, 'activeParameter': 0}
    if mode == 'null': result = None
    if mode == 'malformed': result['signatures'][0]['parameters'][0]['label'] = [3, 4]
    message = {'jsonrpc': '2.0', 'id': request_id, 'result': result}
    if mode == 'error': message = {'jsonrpc': '2.0', 'id': request_id, 'error': {'code': -32603, 'message': 'controlled failure'}}
    send(message)
    log({'replySent': request_id})
while True:
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line == b'\r\n': break
        key, _, value = line.partition(b':')
        if key.lower() == b'content-length': length = int(value)
    message = json.loads(sys.stdin.buffer.read(length))
    log(message)
    method = message.get('method')
    if method == 'initialize':
        capabilities = {'positionEncoding': 'utf-16', 'textDocumentSync': 1}
        if mode != 'unsupported': capabilities['signatureHelpProvider'] = {'triggerCharacters': ['(', ','], 'retriggerCharacters': [')']}
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'capabilities': capabilities}})
    elif method == 'textDocument/signatureHelp':
        if mode == 'silent': continue
        if mode == 'crash': sys.exit(1)
        if mode in ('reorder', 'on_change'):
            pending.append(message['id'])
            if mode == 'on_change' or len(pending) < 2: continue
            for request_id in reversed(pending): reply(request_id)
            pending = []
        else: reply(message['id'])
    elif method == 'textDocument/didChange' and mode == 'on_change':
        for request_id in pending: reply(request_id)
        pending = []
    elif method == 'shutdown': send({'jsonrpc': '2.0', 'id': message['id'], 'result': None})
    elif method == 'exit': sys.exit(0)
