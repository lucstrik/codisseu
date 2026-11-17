"""Controlled Phpactor startup progress test, never a product provider."""
import json,sys,threading,time
from pathlib import Path
mode,log_path,release_path=sys.argv[1:]
lock=threading.Lock()
def send(v):
 b=json.dumps(v).encode()
 with lock:
  sys.stdout.buffer.write(f'Content-Length: {len(b)}\r\n\r\n'.encode()+b);sys.stdout.buffer.flush()
def log(v):
 with open(log_path,'a')as f:f.write(json.dumps(v)+'\n')
def progress(token,value):send({'jsonrpc':'2.0','method':'$/progress','params':{'token':token,'value':value}})
def release():
 while not Path(release_path).exists():
  if mode=='progress':progress('index-real',{'kind':'report','message':'500/1000'})
  time.sleep(1 if mode=='progress' else .005)
 progress('index-real',{'kind':'end','message':'fixture indexing completed'})
 log({'released':True})
while True:
 h={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  k,v=line.decode().split(':',1);h[k.lower()]=v.strip()
 m=json.loads(sys.stdin.buffer.read(int(h['content-length'])));log(m)
 if m.get('method')=='initialize':
  result={'capabilities':{'textDocumentSync':1,'referencesProvider':True}}
  if mode!='generic':result['serverInfo']={'name':'phpactor/phpactor','version':'controlled'}
  send({'jsonrpc':'2.0','id':m['id'],'result':result})
 elif m.get('method')=='initialized' and mode!='generic':
  send({'jsonrpc':'2.0','id':'progress-create','method':'window/workDoneProgress/create','params':{'token':'index-real'}})
 elif m.get('id')=='progress-create':
  progress('index-real',{'kind':'end'}) # out of order, must not release
  progress('other',{'kind':'begin','title':'unrelated work'})
  progress('index-real',{'kind':'begin','title':'Indexing workspace'})
  progress('other',{'kind':'end'}) # mismatched token, must not release
  log({'waiting':True})
  if mode!='timeout':threading.Thread(target=release,daemon=True).start()
 elif m.get('method')=='shutdown':send({'jsonrpc':'2.0','id':m['id'],'result':None})
 elif m.get('method')=='exit':sys.exit(0)
