"""Owned formatting transport fixture. No filesystem modifications or external tools."""
import json, sys, time
mode, log_path = sys.argv[1:]
def send(value):
    body=json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode()+body);sys.stdout.buffer.flush()
def edit(sl,sc,el,ec,text):return {"range":{"start":{"line":sl,"character":sc},"end":{"line":el,"character":ec}},"newText":text}
while True:
    length=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:sys.exit(0)
        if line==b"\r\n":break
        key,_,value=line.partition(b":")
        if key.lower()==b"content-length":length=int(value)
    msg=json.loads(sys.stdin.buffer.read(length))
    with open(log_path,"a",encoding="utf8") as out:out.write(json.dumps(msg)+"\n")
    method=msg.get("method")
    if method=="initialize":
        send({"jsonrpc":"2.0","id":msg["id"],"result":{"capabilities":{"positionEncoding":"utf-16","textDocumentSync":1,"documentFormattingProvider":mode!="unsupported","documentRangeFormattingProvider":{} if mode!="unsupported" else False}}})
    elif method in ["textDocument/formatting","textDocument/rangeFormatting"]:
        if mode=="silent":continue
        if mode=="crash":sys.exit(7)
        if mode=="delay":time.sleep(.15)
        result=[edit(1,5,1,6," = ")]
        if method=="textDocument/formatting":result.append(edit(2,5,2,6," = "))
        if mode=="null":result=None
        elif mode=="surrogate":result=[edit(0,4,0,4,"bad")]
        elif mode=="overlap":result=[edit(1,5,1,7,"x"),edit(1,6,1,6,"y")]
        elif mode=="scope":result={"changes":{"file:///outside.ts":[]}}
        elif mode=="large":result=[edit(1,5,1,6,"x"*(1024*1024+1))]
        response={"jsonrpc":"2.0","id":msg["id"],"result":result}
        if mode=="error":response={"jsonrpc":"2.0","id":msg["id"],"error":{"code":-32603,"message":"Owned failure"}}
        if mode=="missing":response={"jsonrpc":"2.0","id":msg["id"]}
        send(response)
    elif method=="shutdown":send({"jsonrpc":"2.0","id":msg["id"],"result":None})
    elif method=="exit":sys.exit(0)
