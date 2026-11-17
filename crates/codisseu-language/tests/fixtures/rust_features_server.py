"""Owned Rust request fixture. Never execute commands from JSON metadata."""
import json, sys, time
mode, log_path, outside_uri = sys.argv[1:]
uri=None;version=7

def send(v):
    b=json.dumps(v).encode();sys.stdout.buffer.write(f'Content-Length: {len(b)}\r\n\r\n'.encode()+b);sys.stdout.buffer.flush()
def edit(char=0):return {'range':{'start':{'line':0,'character':char},'end':{'line':0,'character':char}},'newText':'use std::collections::HashMap;\n'}
def action():
    value={'title':'Import HashMap','kind':'quickfix','isPreferred':True,'data':{'opaque':'same value','nested':[1,2,3]}}
    if mode=='command':value['command']={'command':'malicious','arguments':['ignored']}
    if mode=='disabled':value['disabled']={'reason':'Owned unavailable'}
    return value
while True:
    length=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:sys.exit(0)
        if line==b'\r\n':break
        k,_,v=line.partition(b':')
        if k.lower()==b'content-length':length=int(v)
    msg=json.loads(sys.stdin.buffer.read(length))
    with open(log_path,'a',encoding='utf8')as log:log.write(json.dumps(msg)+'\n')
    method=msg.get('method')
    if method=='initialize':
        send({'jsonrpc':'2.0','id':msg['id'],'result':{'capabilities':{'positionEncoding':'utf-16','textDocumentSync':1,'codeActionProvider':False if mode=='unsupported'else{'resolveProvider':True},'inlayHintProvider':mode!='unsupported'}}})
    elif method=='textDocument/didOpen':
        uri=msg['params']['textDocument']['uri'];version=msg['params']['textDocument']['version']
        send({'jsonrpc':'2.0','method':'textDocument/publishDiagnostics','params':{'uri':uri,'version':version,'diagnostics':[{'range':{'start':{'line':1,'character':0},'end':{'line':1,'character':3}},'message':'owned diagnostic','code':2339,'source':'owned','data':{'opaque':'kept'}}]}})
    elif method=='textDocument/didChange':version=msg['params']['textDocument']['version']
    elif method=='textDocument/codeAction':
        if mode=='silent':continue
        if mode=='delay':time.sleep(.15)
        if mode=='crash':sys.exit(8)
        result=[action()]
        if mode=='null':result=None
        send({'jsonrpc':'2.0','id':msg['id'],'result':result})
    elif method=='codeAction/resolve':
        if mode=='delay-resolve':time.sleep(.15)
        if mode=='silent-resolve':continue
        value=msg['params'];assert value['data']['opaque']=='same value'
        value['edit']={'documentChanges':[{'textDocument':{'uri':uri,'version':version},'edits':[edit()]}]}
        if mode=='escape':value['edit']={'changes':{outside_uri:[edit()]}}
        elif mode=='resource':value['edit']={'documentChanges':[{'kind':'create','uri':uri}]}
        elif mode=='overlap':value['edit']['documentChanges'][0]['edits']=[edit(),edit()]
        elif mode=='surrogate':value['edit']['documentChanges'][0]['edits']=[edit(4)]
        elif mode=='wrong-version':value['edit']['documentChanges'][0]['textDocument']['version']=version-1
        elif mode=='resolve-command':value['command']={'command':'evil'}
        elif mode=='identity':value['title']='Different action'
        send({'jsonrpc':'2.0','id':msg['id'],'result':value})
    elif method=='textDocument/inlayHint':
        if mode=='silent':continue
        if mode=='delay':time.sleep(.15)
        if mode=='crash':sys.exit(8)
        result=[{'position':{'line':1,'character':10},'label':[{'value':': i32','command':{'command':'evil'},'location':{'uri':'https://invalid'}}],'kind':1,'paddingLeft':True}]
        if mode=='hint-surrogate':result[0]['position']={'line':0,'character':4}
        elif mode=='hint-outside':result[0]['position']={'line':2,'character':0}
        elif mode=='hint-control':result[0]['label']='\ninvalid'
        elif mode=='null':result=None
        send({'jsonrpc':'2.0','id':msg['id'],'result':result})
    elif method=='shutdown':send({'jsonrpc':'2.0','id':msg['id'],'result':None})
    elif method=='exit':sys.exit(0)
