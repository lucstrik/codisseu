"""Controlled provider questions. No network, commands, project autoload or PHP."""
import json,sys
from pathlib import Path
mode,log_path,target=sys.argv[1:]
waiting={}
def send(value):
 data=json.dumps(value).encode();sys.stdout.buffer.write(f'Content-Length: {len(data)}\r\n\r\n'.encode()+data);sys.stdout.buffer.flush()
def read():
 length=0
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  name,_,value=line.partition(b':')
  if name.lower()==b'content-length':length=int(value)
 return json.loads(sys.stdin.buffer.read(length))
while True:
 m=read()
 with open(log_path,'a') as f:f.write(json.dumps(m)+'\n')
 method=m.get('method')
 if method=='initialize':send({'jsonrpc':'2.0','id':m['id'],'result':{'capabilities':{'textDocumentSync':1,'positionEncoding':'utf-16','definitionProvider':True}}})
 elif method=='textDocument/definition':
  question='question-'+str(m['id']);waiting[question]=m['id']
  actions=[{'title':'First'},{'title':'Second','opaque':{'command':'never-execute','arguments':['untrusted']}}]
  if mode=='overflow':actions=[{'title':str(i)} for i in range(65)]
  if mode=='malformed':actions=[{'title':123}]
  send({'jsonrpc':'2.0','id':question,'method':'window/showMessageRequest','params':{'type':3,'message':'Choose inferred factory type','actions':actions}})
  if mode=='concurrent':send({'jsonrpc':'2.0','id':'extra-question','method':'window/showMessageRequest','params':{'type':3,'message':'Extra','actions':actions}})
  send({'jsonrpc':'2.0','id':'unknown-request','method':'codisseu/executeArbitrary','params':{}})
 elif method=='shutdown':send({'jsonrpc':'2.0','id':m['id'],'result':None})
 elif method=='exit':sys.exit(0)
 elif method is None and m.get('id') in waiting:
  owner=waiting.pop(m['id']);selection=m.get('result')
  result=None if selection is None else {'uri':Path(target).as_uri(),'range':{'start':{'line':0,'character':13},'end':{'line':0,'character':19}}}
  send({'jsonrpc':'2.0','id':owner,'result':result})
