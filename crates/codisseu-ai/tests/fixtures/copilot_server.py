#!/usr/bin/env python3
"""Controlled LSP peer; never contacts GitHub or reads user credentials."""
import json,sys,threading,time,os,subprocess
from pathlib import Path
base=Path(__file__).resolve();options=base.with_suffix('.json');opts=json.loads(options.read_text()) if options.exists() else {}
log=base.with_suffix('.log');lock=threading.Lock();docs={};signed_in=not opts.get('login');pending_finish=None;sign_in_count=0
def record(v):
    with log.open('a') as f:f.write(json.dumps(v,ensure_ascii=False)+'\n')
def send(v):
    body=json.dumps(v,ensure_ascii=False).encode()
    with lock:sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode()+body);sys.stdout.buffer.flush()
def reply(i,v):send({'jsonrpc':'2.0','id':i,'result':v})
def read():
    size=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:return None
        if line==b'\r\n':break
        if line.lower().startswith(b'content-length:'):size=int(line.split(b':',1)[1])
    if size is None:return None
    return json.loads(sys.stdin.buffer.read(size))
def offset(text,pos):
    lines=text.splitlines(keepends=True);start=sum(len(s) for s in lines[:pos['line']]);line=''.join(lines[pos['line']:]);n=0
    for i,c in enumerate(line):
        if n==pos['character']:return start+i
        n+=len(c.encode('utf-16-le'))//2
    return len(text)
record({'pid':os.getpid()})
if opts.get('spawn_child'):record({'child':subprocess.Popen(['sleep','600']).pid})
while True:
    v=read()
    if v is None:break
    record(v);method=v.get('method');p=v.get('params') or {};i=v.get('id')
    if method is None:
        if pending_finish is not None and i=='auth-url':
            signed_in=not opts.get('finish_no_auth');reply(pending_finish,None);pending_finish=None
        continue
    if method=='initialize':
        reply(i,{'capabilities':{'textDocumentSync':2},'serverInfo':{'name':'Owned Copilot fixture','version':'1'}})
        if opts.get('message'):send({'jsonrpc':'2.0','id':'account-message','method':'window/showMessageRequest','params':{'message':'Controlled account notice','actions':[{'title':'OK'}]}})
    elif method=='signIn':
        sign_in_count+=1
        if opts.get('login_error') or (sign_in_count>1 and opts.get('recovery_login_error')):
            send({'jsonrpc':'2.0','id':i,'error':{'code':-32000,'message':'Controlled account refusal'}})
        else:
            result=opts['recovery_login_result'] if sign_in_count>1 and 'recovery_login_result' in opts else (opts['login_result'] if 'login_result' in opts else ({'status':'AlreadySignedIn'} if signed_in else {'userCode':'TEST-CODE','command':{'command':'github.copilot.finishDeviceFlow','arguments':[]}}))
            if sign_in_count>1 and opts.get('recovery_delay'):
                threading.Timer(opts['recovery_delay'],lambda i=i,result=result:reply(i,result)).start()
            else:reply(i,result)
    elif method=='signOut':reply(i,None)
    elif method=='workspace/executeCommand':
        if p.get('command')=='github.copilot.finishDeviceFlow':
            if opts.get('finish_after_show_document_reply'):pending_finish=i
            else:signed_in=not opts.get('finish_no_auth');reply(i,None)
            send({'jsonrpc':'2.0','id':'auth-url','method':'window/showDocument','params':{'uri':'https://github.com/login/device','external':True}})
        elif opts.get('accept_error') and p.get('command')=='github.copilot.didAcceptCompletionItem':
            threading.Timer(opts.get('accept_error_delay',0.1),lambda i=i:send({'jsonrpc':'2.0','id':i,'error':{'code':-32000,'message':'Controlled acceptance notification failure'}})).start()
        else:reply(i,None)
    elif method=='textDocument/didOpen':docs[p['textDocument']['uri']]=p['textDocument']['text']
    elif method=='textDocument/didChange':
        uri=p['textDocument']['uri'];text=docs[uri]
        for change in p['contentChanges']:
            r=change['range'];a,b=offset(text,r['start']),offset(text,r['end']);text=text[:a]+change['text']+text[b:]
        docs[uri]=text;record({'synced':uri,'text':text})
    elif method=='textDocument/didClose':docs.pop(p['textDocument']['uri'],None)
    elif method=='textDocument/inlineCompletion':
        version=p['textDocument']['version'];delay=opts.get('old_delay',0.2) if version==1 else opts.get('delay',0.03)
        at=p['position'];items=[{'insertText':text,'range':{'start':at,'end':at},'command':{'command':'github.copilot.didAcceptCompletionItem','arguments':[str(version),index]}} for index,text in enumerate(opts.get('suggestions',[opts.get('suggestion','return 42;')]))]
        items=opts.get('items',items)
        def complete(i=i,items=items,version=version):
            if version in opts.get('completion_error_versions',[]):
                send({'jsonrpc':'2.0','id':i,'error':{'code':-32001,'message':'Controlled completion failure'}})
            else:reply(i,{'items':items})
            if opts.get('revoke_after_completion'):send({'jsonrpc':'2.0','method':'didChangeStatus','params':{'kind':'Error','message':'Controlled expired account'}})
            for kind in opts.get('statuses_after_completion',[]):send({'jsonrpc':'2.0','method':'didChangeStatus','params':{'kind':kind,'message':'Controlled account state'}})
        threading.Timer(delay,complete).start()
    elif method=='workspace/configuration':reply(i,[None])
    elif method=='shutdown':reply(i,None)
    elif method=='exit':break
    elif i is not None:reply(i,None)
