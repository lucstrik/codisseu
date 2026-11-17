"""Controlled wire peer for closed-snapshot lease lifecycle, never used by product."""
import json
import sys
from pathlib import Path

mode, log_path = sys.argv[1:]
documents = {}
held = None
did_hold = False


def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode() + body)
    sys.stdout.buffer.flush()


while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b'\r\n':
            break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    with Path(log_path).open('a') as log:
        log.write(json.dumps(message) + '\n')
    method = message.get('method')
    params = message.get('params', {})
    if method == 'initialize':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'capabilities': {'textDocumentSync': 1, 'renameProvider': True}}})
    elif method == 'textDocument/didOpen':
        document = params['textDocument']
        documents[document['uri']] = document
        send({'jsonrpc': '2.0', 'method': 'textDocument/publishDiagnostics', 'params': {'uri': document['uri'], 'version': document['version'], 'diagnostics': [{'range': {'start': {'line': 0, 'character': 0}, 'end': {'line': 0, 'character': 1}}, 'message': 'lease diagnostic must remain private', 'severity': 1}]}})
    elif method == 'textDocument/didChange':
        documents[params['textDocument']['uri']] = {**params['textDocument'], 'text': params['contentChanges'][0]['text']}
    elif method == 'textDocument/didClose':
        documents.pop(params['textDocument']['uri'], None)
    elif method == 'textDocument/rename':
        document = documents[params['textDocument']['uri']]
        position = document['text'].index('rainfall')
        response = {'jsonrpc': '2.0', 'id': message['id'], 'result': {'documentChanges': [{'textDocument': {'uri': document['uri'], 'version': 9 if mode == 'version' else document['version']}, 'edits': [{'range': {'start': {'line': 0, 'character': position}, 'end': {'line': 0, 'character': position + 8}}, 'newText': params['newName']}]}]}}
        if mode == 'error':
            response = {'jsonrpc': '2.0', 'id': message['id'], 'error': {'code': -32603, 'message': 'controlled refusal'}}
        if mode == 'hold' and not did_hold:
            held = response
            did_hold = True
        else:
            send(response)
    elif method == '$/cancelRequest' and held:
        # Deliberately violate timely cancellation: the client must discard this.
        send(held)
        held = None
    elif method == 'shutdown':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': None})
    elif method == 'exit':
        break
