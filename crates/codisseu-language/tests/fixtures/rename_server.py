"""Controlled LSP failures for rename; never used as a product provider."""
import json,sys
from pathlib import Path
mode,log_path,source,target,outside=sys.argv[1:]
pending=[]
def log(v):
 with open(log_path,'a')as f:f.write(json.dumps(v)+'\n')
def send(v):
 b=json.dumps(v).encode();sys.stdout.buffer.write(f'Content-Length: {len(b)}\r\n\r\n'.encode()+b);sys.stdout.buffer.flush()
def rng(a,b):return {'start':{'line':0,'character':a},'end':{'line':0,'character':b}}
def edit(a,b,name):return {'range':rng(a,b),'newText':name}
def reply(m):
 method=m['method'];new=m['params'].get('newName','forecast')
 if method=='textDocument/prepareRename':
  result={'range':rng(2,10),'placeholder':'rainfall'}
  if mode=='range':result=rng(2,10)
  if mode=='default':result={'defaultBehavior':True}
  if mode=='prepare_null':result=None
  if mode=='prepare_surrogate':result=rng(1,10)
 else:
  su=Path(source).as_uri();tu=Path(target).as_uri()
  result={'changes':{su:[edit(2,10,new)],tu:[edit(0,8,new)]}}
  if mode=='versioned':result={'documentChanges':[{'textDocument':{'uri':su,'version':2},'edits':[edit(2,10,new)]},{'textDocument':{'uri':tu,'version':0},'edits':[edit(0,8,new)]}]}
  if mode=='stale':result={'documentChanges':[{'textDocument':{'uri':su,'version':1},'edits':[edit(2,10,new)]}]}
  if mode=='mixed':result['documentChanges']=[]
  if mode=='resource':result={'documentChanges':[{'kind':'rename','oldUri':su,'newUri':tu}]}
  if mode=='annotations':result['changeAnnotations']={'c':{'label':'confirm','needsConfirmation':True}}
  if mode=='annotated':result['changes'][su][0]['annotationId']='c'
  if mode=='outside':result['changes'][Path(outside).as_uri()]=[edit(0,8,new)]
  if mode=='remote':result['changes']['file://remote/tmp/a.ts']=[edit(0,8,new)]
  if mode=='invalid_range':result['changes'][su][0]['range']=rng(10,2)
  if mode=='surrogate':result['changes'][su][0]['range']=rng(1,10)
  if mode=='overlap':result['changes'][su].append(edit(4,8,new))
  if mode=='nul':result['changes'][su][0]['newText']='\0'
  if mode=='null':result=None
  if mode=='empty':result={'changes':{}}
 if mode in ('error','on_change_error'):send({'jsonrpc':'2.0','id':m['id'],'error':{'code':-32602,'message':'controlled rename rejection'}})
 else:send({'jsonrpc':'2.0','id':m['id'],'result':result})
 log({'replySent':m['id']})
while True:
 h={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  k,v=line.decode().split(':',1);h[k.lower()]=v.strip()
 m=json.loads(sys.stdin.buffer.read(int(h['content-length'])));log(m);method=m.get('method')
 if method=='initialize':
  cap={'textDocumentSync':1,'positionEncoding':'utf-16'}
  if mode!='missing':cap['renameProvider']=True if mode=='fallback' else ({'prepareProvider':False}if mode=='fallback_options' else {'prepareProvider':True})
  send({'jsonrpc':'2.0','id':m['id'],'result':{'capabilities':cap}})
 elif method in ('textDocument/rename','textDocument/prepareRename'):
  if mode=='crash':sys.exit(7)
  if mode=='silent':continue
  if mode in ('reorder','on_change','on_change_error'):
   pending.append(m)
   if mode.startswith('on_change') or len(pending)<2:continue
   for item in reversed(pending):reply(item)
   pending=[]
  else:
   if mode=='apply':send({'jsonrpc':'2.0','id':'forbidden-apply','method':'workspace/applyEdit','params':{'edit':{'changes':{Path(target).as_uri():[edit(0,8,'FORBIDDEN')]}}}})
   reply(m)
 elif method in ('textDocument/didChange','textDocument/didClose') and mode.startswith('on_change'):
  for item in pending:reply(item)
  pending=[]
 elif method=='shutdown':send({'jsonrpc':'2.0','id':m['id'],'result':None})
 elif method=='exit':sys.exit(0)
