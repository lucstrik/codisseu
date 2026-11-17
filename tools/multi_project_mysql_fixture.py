#!/usr/bin/env python3
"""One owned MySQL fixture for aggregate-workspace tests; existing image only."""
import argparse, json, os, subprocess, time, uuid
from pathlib import Path

p=argparse.ArgumentParser();p.add_argument('--out',type=Path,required=True);a=p.parse_args()
out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
docker='/usr/bin/docker';context=subprocess.check_output([docker,'context','show'],text=True).strip()
def run(*args,**kw):return subprocess.run([docker,'--context',context,*args],check=True,capture_output=True,text=True,timeout=45,**kw)
image='mysql:8.0';image_id=run('image','inspect','--format','{{.Id}}',image).stdout.strip()
token=uuid.uuid4().hex[:12];label='org.codisseu.fixture='+token;container=None
record={'image':image,'image_id':image_id,'context':context,'label':label,'scope':'own tmpfs database, loopback port, no pull or host mounts','result':'in_progress'}
try:
    container=run('run','--detach','--rm','--pull','never','--name','codisseu-multi-mysql-'+token,'--label',label,'--memory','1g','--cpus','2','--publish','127.0.0.1::3306','--tmpfs','/var/lib/mysql:rw,size=512m','--env','MYSQL_DATABASE=codisseu','--env','MYSQL_ROOT_HOST=%','--env','MYSQL_ROOT_PASSWORD=codisseu_fixture',image,'--skip-log-bin','--innodb-buffer-pool-size=64M').stdout.strip()
    record['container']=container
    start=time.monotonic()
    while True:
        health=subprocess.run([docker,'--context',context,'exec',container,'mysqladmin','--protocol=tcp','-h127.0.0.1','-uroot','-pcodisseu_fixture','ping'],capture_output=True,timeout=5)
        if health.returncode==0:break
        assert time.monotonic()-start<90,'owned MySQL fixture did not become ready'
        time.sleep(.25)
    address=run('port',container,'3306/tcp').stdout.strip();assert address.startswith('127.0.0.1:')
    record['address']=address
    env=os.environ.copy();env['CODISSEU_MULTI_MYSQL_PORT']=address.rsplit(':',1)[1]
    with (out/'mysql-workspace-test.log').open('w') as log:
        result=subprocess.run(['cargo','test','-p','codisseu-core','--test','multi_project_tools','aggregate_workspace_mysql','--locked','--jobs','1','--','--ignored','--test-threads=1'],env=env,stdout=log,stderr=subprocess.STDOUT)
    record['exit_code']=result.returncode;record['result']='passed' if result.returncode==0 else 'failed'
finally:
    try:
        if container:
            probe=subprocess.run([docker,'--context',context,'container','inspect','--format','{{json .Config.Labels}}',container],capture_output=True,text=True,timeout=15)
            if probe.returncode==0:
                labels=json.loads(probe.stdout);key,value=label.split('=',1);assert labels.get(key)==value
                run('container','rm','--force',container)
            elif 'No such' not in probe.stderr:
                raise RuntimeError('could not establish owned fixture state during cleanup')
            probe=subprocess.run([docker,'--context',context,'container','inspect',container],capture_output=True,text=True,timeout=15)
            record['container_removed']=probe.returncode!=0 and 'No such' in probe.stderr;assert record['container_removed']
    except Exception as error:
        record['cleanup_error']=str(error);record['result']='cleanup_failed';raise
    finally:
        (out/'mysql-fixture-manifest.json').write_text(json.dumps(record,indent=2)+'\n')
raise SystemExit(record.get('exit_code',1))
