"""Controlled references protocol fixture, never a product language server."""
import json
from pathlib import Path
import sys
mode,log_path,target,outside=sys.argv[1:]
pending=[]
def log(value):
    with open(log_path,'a',encoding='utf-8') as file:file.write(json.dumps(value)+'\n')
def send(value):
    data=json.dumps(value).encode();sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode()+data);sys.stdout.buffer.flush()
def location(uri,a=0,b=6):return {'uri':uri,'range':{'start':{'line':0,'character':a},'end':{'line':0,'character':b}}}
def reply(request_id):
    result=[location(Path(outside if mode=='outside' else target).as_uri())]
    if mode=='two':result.append(location(Path(outside).as_uri()))
    if mode=='null':result=None
    if mode=='object':result=result[0]
    if mode=='remote':result[0]['uri']='https://example.invalid/ref.ts'
    if mode=='link':result=[{'targetUri':Path(target).as_uri(),'targetRange':result[0]['range'],'targetSelectionRange':result[0]['range']}]
    if mode=='bad_range':result[0]['range']['start']['character']=-1
    if mode=='many':result=[location(Path(target).as_uri(),i,i+1) for i in range(257)]
    if mode.startswith('adapter_'):result=[location(Path(target).as_uri(),0,6),location(Path(target).as_uri(),6,12)]
    if mode=='error':send({'jsonrpc':'2.0','id':request_id,'error':{'code':-32000,'message':'controlled references error'}})
    else:send({'jsonrpc':'2.0','id':request_id,'result':result})
    log({'replySent':request_id})
while True:
    length=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:sys.exit(0)
        if line==b'\r\n':break
        key,_,value=line.partition(b':')
        if key.lower()==b'content-length':length=int(value)
    message=json.loads(sys.stdin.buffer.read(length));log(message);method=message.get('method')
    if method=='initialize':
        capabilities={'positionEncoding':'utf-16','textDocumentSync':1}
        if mode!='unsupported':capabilities['referencesProvider']={} if mode=='options' else True
        if mode.startswith('adapter_') and mode!='adapter_missing':capabilities['definitionProvider']=True
        send({'jsonrpc':'2.0','id':message['id'],'result':{'capabilities':capabilities}})
    elif method=='textDocument/definition':
        if mode=='adapter_error':send({'jsonrpc':'2.0','id':message['id'],'error':{'code':-32000,'message':'private definition failed'}})
        elif mode=='adapter_null':send({'jsonrpc':'2.0','id':message['id'],'result':None})
        elif mode in ('adapter_holddef','adapter_reorderdef'):
            pending.append(message['id'])
            if mode=='adapter_reorderdef' and len(pending)==2:
                for request_id in reversed(pending):send({'jsonrpc':'2.0','id':request_id,'result':[location(Path(target).as_uri())]})
                pending=[]
        else:send({'jsonrpc':'2.0','id':message['id'],'result':[location(Path(target).as_uri())]})
    elif method=='textDocument/references':
        if mode=='silent':continue
        if mode in ('reorder','on_change','adapter_holdrefs'):
            pending.append(message['id'])
            if mode in ('on_change','adapter_holdrefs') or len(pending)<2:continue
            for request_id in reversed(pending):reply(request_id)
            pending=[]
        else:reply(message['id'])
    elif method=='textDocument/didChange' and mode in ('on_change','adapter_holddef','adapter_holdrefs'):
        for request_id in pending:
            if mode=='adapter_holddef':send({'jsonrpc':'2.0','id':request_id,'result':[location(Path(target).as_uri())]})
            else:reply(request_id)
        pending=[]
    elif method=='shutdown':send({'jsonrpc':'2.0','id':message['id'],'result':None})
    elif method=='exit':sys.exit(0)
