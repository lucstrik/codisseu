#!/usr/bin/python3
"""Owned protocol/lifecycle fixture, never a production browser implementation."""
import argparse, json, math, os, socket, struct, sys, time
p=argparse.ArgumentParser();p.add_argument('--socket');p.add_argument('--runtime');p.add_argument('--profile');p.add_argument('--parent-pid');a=p.parse_args()
mode=open(os.path.join(a.runtime,'mode')).read().strip()
with open(os.path.join(a.runtime,'starts'),'a') as f:f.write(str(os.getpid())+'\n')
with open(os.path.join(a.runtime,'info.json'),'w') as f:json.dump({'pid':os.getpid(),'parent':os.getppid(),'socket':a.socket,'profile':a.profile,'dir_mode':os.stat(os.path.dirname(a.socket)).st_mode&0o777,'socket_mode':os.stat(a.socket).st_mode&0o777,'profile_mode':os.stat(a.profile).st_mode&0o777,'env_leak':'CODEX_BROWSER_SECRET' in os.environ},f)
if mode=='crash-before':sys.exit(17)
if mode=='foreign-peer':
    pid=os.fork()
    if pid==0:
        s=socket.socket(socket.AF_UNIX);s.connect(a.socket);s.sendall(struct.pack('<II',1,0)+b'!');s.close();os._exit(0)
    os.waitpid(pid,0);time.sleep(.05)
s=socket.socket(socket.AF_UNIX);s.connect(a.socket)
def send(event,payload=b''):
    raw=json.dumps(event,ensure_ascii=False).encode();s.sendall(struct.pack('<II',len(raw),len(payload))+raw+payload)
def receive():
    def exact(n):
        out=b''
        while len(out)<n:
            v=s.recv(n-len(out))
            if not v:raise EOFError()
            out+=v
        return out
    j,n=struct.unpack('<II',exact(8));return json.loads(exact(j)),exact(n)
if mode=='silent':time.sleep(30);sys.exit(0)
if mode=='oversize':s.sendall(struct.pack('<II',65537,0));time.sleep(30);sys.exit(0)
send({'type':'Ready','protocol':1 if mode=='old-protocol' else 2,'versions':{'cef':'owned-protocol-fixture','chromium':'not-a-browser'}})
if mode=='crash-ready':sys.exit(18)
if mode=='hang':
    pid=os.fork()
    if pid==0:time.sleep(60);os._exit(0)
    open(os.path.join(a.runtime,'child'),'w').write(str(pid))
tabs={};sizes={};visible={};sequences={}
def state(i,url=None):
    if url is not None:tabs[i]=url
    send({'type':'State','id':i,'url':tabs.get(i,'about:blank'),'title':'Owned\nprotocol title','loading':True,'can_go_back':False,'can_go_forward':False,'error':None})
def frame(i,surface='Page',late=False):
    key=(i,surface);w,h,scale=sizes.get(key,(2,1,1));w=math.ceil(w*scale);h=math.ceil(h*scale);seq=sequences.get(key,0)+1;sequences[key]=seq
    if mode=='badframe':send({'type':'Frame','id':i,'surface':surface,'width':w,'height':h,'seq':seq},b'bad');return
    send({'type':'Frame','id':i,'surface':surface,'width':w,'height':h,'seq':seq},bytes([1,2,3,255])*(w*h))
try:
 while True:
    c,payload=receive();method=c['type'];i=c.get('id');surface=c.get('surface','Page')
    with open(os.path.join(a.runtime,'wire.jsonl'),'a') as f:f.write(json.dumps(c)+'\n')
    if method=='Open':
        if mode=='badurl':state(i,'file:///tmp/private');continue
        state(i,c['url'])
        if mode=='native-close':send({'type':'Closed','id':i});break
    elif method=='Navigate':state(i,c['url'])
    elif method=='Resize':sizes[(i,surface)]=(c['width'],c['height'],c['scale'])
    elif method=='Visible':
        visible[(i,surface)]=c['visible']
        if c['visible']:
            for _ in range(100 if mode=='burst' else 1):frame(i,surface)
    elif method=='DevToolsSubscribe':pass
    elif method=='DevToolsMessage':
        msg=json.loads(payload);out={'id':msg.get('id',1),'result':{'result':{'value':42}}}
        send({'type':'DevToolsMessage','id':i,'epoch':c['epoch']},json.dumps(out).encode())
    elif method=='Close':
        state(i);frame(i,late=True);send({'type':'Closed','id':i});tabs.pop(i,None)
    elif method=='Shutdown':
        if mode=='hang':time.sleep(60)
        break
except (EOFError,BrokenPipeError,ConnectionResetError):pass
