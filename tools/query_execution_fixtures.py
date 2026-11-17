#!/usr/bin/env python3
"""Real query routing on owned loopback Mongo/MySQL containers; no personal data."""
import argparse,json,os,subprocess,time,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def run(*args,**kw):return subprocess.run([str(a) for a in args],check=True,capture_output=True,text=True,timeout=kw.pop('timeout',90),**kw)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 token=uuid.uuid4().hex[:12];owned=[];report={'scope':'own tmpfs and loopback only; existing images, no pull/host mounts/personal profiles','servers':[]};status=1
 try:
  env=dict(os.environ)
  for key,image,port,data,variables,command,health in [
   ('MY','mysql:8.0',3306,'/var/lib/mysql',['MYSQL_DATABASE=codisseu_fixture','MYSQL_ROOT_HOST=%','MYSQL_ROOT_PASSWORD=codisseu_fixture'],['--skip-log-bin','--innodb-buffer-pool-size=64M'],['mysqladmin','--protocol=tcp','-h127.0.0.1','-uroot','-pcodisseu_fixture','ping']),
   ('MONGO','mongo:6.0',27017,'/data/db',[],['--bind_ip_all','--quiet'],['mongosh','--quiet','--eval','db.runCommand({ping:1}).ok'])]:
   image_id=run('docker','image','inspect','--format','{{.Id}}',image).stdout.strip();name=f'codisseu-query-{key.lower()}-{token}'
   args=['docker','run','--detach','--rm','--pull','never','--name',name,'--label','org.codisseu.query-fixture='+token,'--memory','1g','--cpus','2','--publish',f'127.0.0.1::{port}','--tmpfs',f'{data}:rw,size=512m']
   for variable in variables:args+=['--env',variable]
   cid=run(*args,image,*command).stdout.strip();owned.append(cid);end=time.monotonic()+90
   while subprocess.run(['docker','exec',cid,*health],capture_output=True,timeout=10).returncode:
    assert time.monotonic()<end;time.sleep(.3)
   address=run('docker','port',cid,f'{port}/tcp').stdout.strip();assert address.startswith('127.0.0.1:');env[f'CODISSEU_QUERY_{key}_PORT']=address.rsplit(':',1)[1]
   if key=='MONGO':run('docker','exec',cid,'mongosh','--quiet','--eval','db.getSiblingDB("codisseu_fixture").users.insertMany([{_id:1,label:"first"},{_id:2,label:"second"}])')
   version=run('docker','exec',cid,*(['mysql','--version']if key=='MY'else['mongod','--version'])).stdout
   report['servers'].append({'key':key,'container':cid,'image':image,'image_id':image_id,'address':address,'version':version})
  env['CODISSEU_QUERY_EXEC_CAPTURES']=str(out/'ui')
  with (out/'ui-real.log').open('w')as log:
   result=subprocess.run(['cargo','test','-p','codisseu-ui','--lib','query_execution_tests::mysql_and_mongo_real','--locked','--jobs','2','--','--ignored','--test-threads=1'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=300)
  report['test_exit']=result.returncode
  mongo=next(s['container']for s in report['servers']if s['key']=='MONGO')
  count=run('docker','exec',mongo,'mongosh','--quiet','--eval','db.getSiblingDB("codisseu_fixture").users.countDocuments({})').stdout.strip();assert count=='2',count;report['independent_document_count']=2
  assert result.returncode==0,'UI integration failed; see ui-real.log';status=0
 finally:
  report['cleanup']=[]
  for cid in reversed(owned):
   labels=json.loads(run('docker','inspect','--format','{{json .Config.Labels}}',cid).stdout);assert labels.get('org.codisseu.query-fixture')==token
   run('docker','rm','--force',cid);absent=subprocess.run(['docker','inspect',cid],capture_output=True).returncode!=0;report['cleanup'].append({'container':cid,'absent':absent})
  report['exit']=status;(out/'results.json').write_text(json.dumps(report,indent=2)+'\n')
 return status
if __name__=='__main__':raise SystemExit(main())
