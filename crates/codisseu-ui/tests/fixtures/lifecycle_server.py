#!/usr/bin/env python3
"""Explicit local LSP peer: an initialize or hover may deliberately never reply."""
import json, os, sys
from pathlib import Path

mode, log = sys.argv[1:3]
def send(message):
    data = json.dumps(message).encode()
    sys.stdout.buffer.write(b'Content-Length: '+str(len(data)).encode()+b'\r\n\r\n'+data)
    sys.stdout.buffer.flush()
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line in (b'\r\n', b'\n'): break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    message = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method = message.get('method')
    with Path(log).open('a') as output:
        output.write(json.dumps({'pid':os.getpid(),'method':method})+'\n')
    if method == 'exit': sys.exit(0)
    if method == 'initialize':
        if mode == 'initialize': continue
        result = {'capabilities':{'textDocumentSync':1,'hoverProvider':True}}
    elif method == 'textDocument/hover':
        if mode == 'hover': continue
        result = {'contents':{'kind':'plaintext','value':'Lifecycle fixture'}}
    elif method == 'shutdown':
        if mode == 'initialize': continue
        result = None
    else: continue
    send({'jsonrpc':'2.0','id':message['id'],'result':result})
