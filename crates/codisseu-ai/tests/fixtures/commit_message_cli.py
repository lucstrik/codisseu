#!/usr/bin/python3
"""Controlled CLI protocol fixture; this does not run a model.

Copy this executable into a temporary directory. Sibling ``mode`` selects
normal/delay/error/hang/ask/empty/invalid/oversized/fenced/claude-tool. Delay
publishes ``started`` then waits for sibling ``release``. Request/pid/cwd/args
records contain only the caller's disposable fixture inputs.
"""
import json,os,sys,time
from pathlib import Path
base=Path(__file__).parent
(base/'pid').write_text(str(os.getpid()))
(base/'cwd').write_text(os.getcwd())
(base/'args').write_text(json.dumps(sys.argv[1:]))
mode=(base/'mode').read_text() if (base/'mode').exists() else 'normal'
answer='Corrija o desconto para valores negativos\n\nLimite a base do cálculo a zero.'
def send(v):print(json.dumps(v),flush=True)
if '--output-format' in sys.argv:
 (base/'prompt').write_text(sys.stdin.read())
 if mode=='claude-tool':send({'type':'assistant','message':{'id':'tool','content':[{'type':'tool_use','id':'read-fixture','name':'Read','input':{'file_path':'marker.ts'}}]}})
 send({'type':'assistant','message':{'id':'answer','content':[{'type':'text','text':answer}]}})
 send({'type':'result','result':answer,'session_id':'fixture-claude','is_error':False})
 sys.exit(0)
for line in sys.stdin:
 v=json.loads(line)
 with (base/'requests').open('a') as f:f.write(json.dumps(v)+'\n')
 m=v.get('method')
 if m=='initialize':send({'id':v['id'],'result':{'userAgent':'fixture'}})
 elif m in ['thread/start','thread/resume']:send({'id':v['id'],'result':{'thread':{'id':'fixture-thread'}}})
 elif m=='turn/start':
  send({'id':v['id'],'result':{'turn':{'id':'fixture-turn','status':'inProgress'}}})
  (base/'started').touch()
  if mode=='delay':
   while not (base/'release').exists():time.sleep(.01)
  if mode=='ask':
   send({'id':'interactive-fixture','method':'item/commandExecution/requestApproval','params':{'command':'touch NEVER'}})
   continue
  if mode=='hang':
   (base/'ready').touch()
   time.sleep(1000)
  if mode=='error':send({'method':'turn/completed','params':{'turn':{'id':'fixture-turn','status':'failed','error':{'message':'fixture generation rejected'}}}});continue
  text=answer
  if mode=='empty':text=''
  if mode=='oversized':text='x'*(65536+1)
  if mode=='invalid':text='invalid\u0000message'
  if mode=='fenced':text='```text\n'+answer+'\n```'
  send({'method':'item/completed','params':{'item':{'id':'preface','type':'agentMessage','text':'Preliminary analysis should not enter the commit'}}})
  send({'method':'item/agentMessage/delta','params':{'itemId':'answer','delta':text[:20]}})
  send({'method':'item/completed','params':{'item':{'id':'answer','type':'agentMessage','text':text}}})
  send({'method':'turn/completed','params':{'turn':{'id':'fixture-turn','status':'completed'}}})
 elif v.get('id')=='interactive-fixture' and m is None:
  (base/'approval-response').write_text(json.dumps(v))
  send({'method':'turn/completed','params':{'turn':{'id':'fixture-turn','status':'failed','error':{'message':'interaction not allowed'}}}})
