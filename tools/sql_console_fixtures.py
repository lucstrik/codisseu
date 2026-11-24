#!/usr/bin/env python3
"""Test the SQL console using owned loopback Docker servers with tmpfs storage."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]

def run(*args):
    return subprocess.run(list(map(str,args)),check=True,capture_output=True,text=True,timeout=90)

def inventory():
    return {key:run(*command).stdout.splitlines() for key,command in {
        'containers':['docker','ps','-aq'], 'volumes':['docker','volume','ls','-q'],
        'images':['docker','image','ls','-q','--no-trunc'], 'networks':['docker','network','ls','-q']}.items()}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--ui',action='store_true')
    args=parser.parse_args(); out=args.out.resolve(); out.mkdir(parents=True,exist_ok=True)
    token=uuid.uuid4().hex[:12]; created=[]
    report={'scope':'owned loopback tmpfs fixtures, existing images, no production/profile access','before':inventory(),'servers':[]}
    try:
        env=dict(os.environ)
        env['CODISSEU_CONSOLE_SCREENSHOTS']=str(out/'screenshots')
        for key,image,port,variables,health,options in [
            ('PG','postgres:18',5432,['POSTGRES_DB=codisseu','POSTGRES_PASSWORD=codisseu_fixture','PGDATA=/var/lib/postgresql/data/pgdata'],['pg_isready','-h127.0.0.1','-Upostgres'],[]),
            ('MY','mysql:8.0',3306,['MYSQL_DATABASE=codisseu','MYSQL_ROOT_PASSWORD=codisseu_fixture','MYSQL_ROOT_HOST=%'],['mysqladmin','--protocol=tcp','-h127.0.0.1','-uroot','-pcodisseu_fixture','ping'],['--skip-log-bin','--innodb-buffer-pool-size=64M']),
            ('MA','mariadb:11.4.10',3306,['MARIADB_DATABASE=codisseu','MARIADB_ROOT_PASSWORD=codisseu_fixture','MARIADB_ROOT_HOST=%'],['mariadb-admin','--protocol=tcp','-h127.0.0.1','-uroot','-pcodisseu_fixture','ping'],['--skip-log-bin','--innodb-buffer-pool-size=64M'])]:
            image_id=run('docker','image','inspect','--format','{{.Id}}',image).stdout.strip()
            path='/var/lib/postgresql' if key=='PG' else '/var/lib/mysql'
            command=['docker','run','--detach','--rm','--pull','never','--name',f'codisseu-console-{key.lower()}-{token}','--label',f'org.codisseu.fixture={token}','--memory','1g','--cpus','2','--publish',f'127.0.0.1::{port}','--tmpfs',f'{path}:rw,size=512m']
            for value in variables: command+=['--env',value]
            cid=run(*command,image,*options).stdout.strip(); created.append(cid)
            end=time.monotonic()+90
            while subprocess.run(['docker','exec',cid,*health],capture_output=True,timeout=5).returncode:
                assert time.monotonic()<end,f'{key} startup failed'; time.sleep(.3)
            address=run('docker','port',cid,f'{port}/tcp').stdout.strip(); assert address.startswith('127.0.0.1:')
            env[f'CODISSEU_CONSOLE_{key}_PORT']=address.rsplit(':',1)[1]
            report['servers'].append({'key':key,'container':cid,'image':image,'image_id':image_id,'address':address,'storage':'tmpfs'})
        commands=[['cargo','test','-p','codisseu-db','--test','sql_console','--locked','--jobs','2','--','--ignored','--nocapture','--test-threads=1']]
        env['CODISSEU_SQL_PG_PORT']=env['CODISSEU_CONSOLE_PG_PORT']; env['CODISSEU_SQL_MY_PORT']=env['CODISSEU_CONSOLE_MY_PORT']
        commands.append(['cargo','test','-p','codisseu-db','--test','remote_sql','--locked','--jobs','2','--','--ignored','--nocapture','--test-threads=1'])
        if args.ui:
            commands.append(['cargo','test','-p','codisseu-ui','--lib','native_sql','--locked','--jobs','2','--','--include-ignored','--nocapture','--test-threads=1'])
        report['tests']=[]
        for i,command in enumerate(commands):
            with (out/f'tests-{i}.log').open('w') as log:
                result=subprocess.run(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
            report['tests'].append({'command':command,'exit':result.returncode,'log':f'tests-{i}.log'})
            assert result.returncode==0, f'test failed: {out}/tests-{i}.log'
        report['result']='passed'
    finally:
        report['removed']=[]
        for cid in reversed(created):
            labels=json.loads(run('docker','inspect','--format','{{json .Config.Labels}}',cid).stdout)
            assert labels.get('org.codisseu.fixture')==token
            run('docker','rm','--force','--volumes',cid); report['removed'].append(cid)
        report['after']=inventory()
        report['existing_resources_preserved']=all(set(report['before'][key])==set(report['after'][key]) for key in report['before'])
        (out/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    print(out/'manifest.json')

if __name__=='__main__': main()
