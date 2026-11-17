"""Controlled LSP transport for the valid class-method UI fixture; not a provider."""
import json,sys
from pathlib import Path
mode,log_path,source,target,outside=sys.argv[1:]
texts={}
def send(v):
 b=json.dumps(v).encode();sys.stdout.buffer.write(f'Content-Length: {len(b)}\r\n\r\n'.encode()+b);sys.stdout.buffer.flush()
def position(text,offset):
 before=text[:offset];return {'line':before.count('\n'),'character':len(before.rsplit('\n',1)[-1].encode('utf-16-le'))//2}
def edit(text,offset,name):return {'range':{'start':position(text,offset),'end':position(text,offset+8)},'newText':name}
def occurrences(text):
 start=0
 while True:
  at=text.find('rainfall()',start)
  if at<0:return
  yield at;start=at+10
while True:
 h={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  k,v=line.decode().split(':',1);h[k.lower()]=v.strip()
 m=json.loads(sys.stdin.buffer.read(int(h['content-length'])))
 with open(log_path,'a')as log:log.write(json.dumps(m)+'\n')
 method=m.get('method');params=m.get('params',{})
 if method=='initialize':send({'jsonrpc':'2.0','id':m['id'],'result':{'capabilities':{'positionEncoding':'utf-16','textDocumentSync':1,'renameProvider':{'prepareProvider':True}}}})
 elif method=='textDocument/didOpen':texts[params['textDocument']['uri']]=params['textDocument']['text']
 elif method=='textDocument/didChange':texts[params['textDocument']['uri']]=params['contentChanges'][0]['text']
 elif method=='textDocument/prepareRename':
  text=texts[params['textDocument']['uri']];at=next(occurrences(text));r=edit(text,at,'')['range']
  send({'jsonrpc':'2.0','id':m['id'],'result':{'range':r,'placeholder':'rainfall'}})
 elif method=='textDocument/rename':
  changes={}
  for p in [source,target]:
   uri=Path(p).as_uri();text=texts.get(uri,Path(p).read_text());edits=[edit(text,at,params['newName'])for at in occurrences(text)]
   if edits:changes[uri]=edits
  send({'jsonrpc':'2.0','id':m['id'],'result':{'changes':changes}})
 elif method=='shutdown':send({'jsonrpc':'2.0','id':m['id'],'result':None})
 elif method=='exit':sys.exit(0)
