"""Controlled PHP-family references transport fixture, never a product provider."""
import json,sys
from pathlib import Path
mode,log_path,base,usage,usage2=sys.argv[1:]
held=None
def log(m):
 with open(log_path,'a') as f:f.write(json.dumps(m)+'\n')
def send(m):
 b=json.dumps(m).encode();sys.stdout.buffer.write(f'Content-Length: {len(b)}\r\n\r\n'.encode()+b);sys.stdout.buffer.flush()
def reply(m,result):send({'jsonrpc':'2.0','id':m['id'],'result':result})
def location(path,line=0,start=0,end=6):return {'uri':Path(path).as_uri(),'range':{'start':{'line':line,'character':start},'end':{'line':line,'character':end}}}
def base_reply(m):reply(m,[location(usage),location(usage2)])
while True:
 h={}
 while True:
  l=sys.stdin.buffer.readline()
  if not l:sys.exit(0)
  if l==b'\r\n':break
  k,v=l.decode().split(':',1);h[k.lower()]=v.strip()
 m=json.loads(sys.stdin.buffer.read(int(h['content-length'])));log(m);method=m.get('method')
 if method=='initialize':reply(m,{'serverInfo':{'name':'phpactor/phpactor'},'capabilities':{'textDocumentSync':1,'definitionProvider':True,'referencesProvider':True}})
 elif method=='initialized':
  for kind in ['begin','end']:send({'jsonrpc':'2.0','method':'$/progress','params':{'token':'index','value':{'kind':kind,'title':'Indexing workspace'}}})
 elif method=='textDocument/definition':
  if mode=='ambiguous':reply(m,[location(base,1,0,56),location(base,1,1,56)])
  else:reply(m,location(base,1,0,5 if mode=='wrong_range' else 56))
 elif method=='textDocument/references':
  if str(m['id']).startswith('references-family-base-'):
   if mode in ['hold_base','choice']:held=m
   if mode=='choice':send({'jsonrpc':'2.0','id':'family-choice','method':'window/showMessageRequest','params':{'type':3,'message':'Continue family references?','actions':[{'title':'Continue','opaque':'preserve'}]}})
   elif mode=='base_error':send({'jsonrpc':'2.0','id':m['id'],'error':{'code':-32000,'message':'base failure'}})
   elif mode!='hold_base':base_reply(m)
  else:reply(m,[location(usage)])
 elif method=='$/cancelRequest' and held:base_reply(held);held=None
 elif m.get('id')=='family-choice' and 'method' not in m:
  if held:base_reply(held);held=None
 elif method=='shutdown':reply(m,None)
 elif method=='exit':sys.exit(0)
