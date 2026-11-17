"""Controlled LSP definition transport fixture. Never used as a product provider."""
import json
from pathlib import Path
import sys
mode, log_path, target, outside = sys.argv[1:]
pending = []
def log(value):
    with open(log_path, 'a', encoding='utf-8') as file:
        file.write(json.dumps(value)+'\n')
def send(value):
    body=json.dumps(value).encode()
    sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode()+body)
    sys.stdout.buffer.flush()
def location(uri):
    return {'uri':uri, 'range':{'start':{'line':0,'character':13},'end':{'line':0,'character':19}}}
while True:
    length=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,_,value=line.partition(b':')
        if key.lower()==b'content-length': length=int(value)
    message=json.loads(sys.stdin.buffer.read(length));log(message)
    method=message.get('method')
    if method=='initialize':
        send({'jsonrpc':'2.0','id':message['id'],'result':{'capabilities':{'positionEncoding':'utf-16','textDocumentSync':1,'definitionProvider':mode!='unsupported','implementationProvider':mode!='unsupported'}}})
    elif method in ('textDocument/definition', 'textDocument/implementation'):
        if mode=='silent': continue
        if mode=='reorder':
            pending.append(message['id'])
            if len(pending)<2: continue
            for request_id in reversed(pending):
                send({'jsonrpc':'2.0','id':request_id,'result':location(Path(target).as_uri())})
            pending=[]
        else:
            result=location(Path(outside if mode=='outside' else target).as_uri())
            if mode=='multi': result=[location(Path(target).as_uri()),location(Path(outside).as_uri())]
            if mode=='remote': result['uri']='https://example.invalid/target.ts'
            if mode=='link':
                result=[{'targetUri':Path(target).as_uri(),'targetRange':{'start':{'line':0,'character':0},'end':{'line':0,'character':25}},'targetSelectionRange':result['range']}]
            if mode=='null': result=None
            send({'jsonrpc':'2.0','id':message['id'],'result':result})
    elif method=='shutdown': send({'jsonrpc':'2.0','id':message['id'],'result':None})
    elif method=='exit': sys.exit(0)
