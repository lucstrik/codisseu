"""Own deterministic LSP fixture, not evidence of Phpactor behavior."""
import json, sys, pathlib, urllib.parse
mode, root, logfile = sys.argv[1:]
root = pathlib.Path(root)
texts = {}
pending = None
def send(value):
    data=json.dumps(value).encode()
    sys.stdout.buffer.write(b'Content-Length: '+str(len(data)).encode()+b'\r\n\r\n'+data);sys.stdout.buffer.flush()
def location(name):
    path=root/(name+'.php')
    text=texts.get(path.as_uri(),path.read_text())
    line=text.splitlines()[1]
    start=line.index(name)
    return {'uri':path.as_uri(),'range':{'start':{'line':1,'character':start},'end':{'line':1,'character':start+len(name)}}}
while True:
    line=sys.stdin.buffer.readline()
    if not line: break
    size=int(line.split(b':')[1]);sys.stdin.buffer.readline();msg=json.loads(sys.stdin.buffer.read(size))
    with open(logfile,'a') as f:f.write(json.dumps(msg)+'\n')
    method=msg.get('method');p=msg.get('params',{});ident=msg.get('id')
    if method=='initialize':send({'jsonrpc':'2.0','id':ident,'result':{'capabilities':{'textDocumentSync':1,'definitionProvider':True,'typeDefinitionProvider':mode!='unsupported'}}})
    elif method=='textDocument/didOpen':texts[p['textDocument']['uri']]=p['textDocument']['text']
    elif method=='textDocument/didChange':
        texts[p['textDocument']['uri']]=p['contentChanges'][0]['text']
        if pending is not None:
            send({'jsonrpc':'2.0','id':pending,'result':location('Base')})
            pending=None
    elif method=='textDocument/definition':
        if mode=='delay':
            pending=ident
            continue
        text=texts[p['textDocument']['uri']];line=text.splitlines()[p['position']['line']];start=p['position']['character']
        word=''
        for ch in line[start:]:
            if not ch.isalnum():break
            word+=ch
        if mode=='outside':result={'uri':(root.parent/'escape.php').as_uri(),'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}
        elif word=='Base':result=location('Base')
        elif word=='Port':result=location('Port')
        elif word=='Contract':result=location('Contract')
        else:result=[]
        send({'jsonrpc':'2.0','id':ident,'result':result})
    elif method=='textDocument/typeDefinition':send({'jsonrpc':'2.0','id':ident,'result':location('Port')})
    elif method=='shutdown':send({'jsonrpc':'2.0','id':ident,'result':None})
    elif method=='exit':break
