#!/usr/bin/env python3
"""Exercise grid edits only on new, labelled loopback MySQL/PostgreSQL fixtures."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True, text=True, timeout=90, **kwargs)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    token = uuid.uuid4().hex[:12]; label = 'org.codisseu.fixture=' + token
    created = []; report = {'scope': 'own tmpfs, loopback, existing images; no host mounts/pull/production', 'servers': []}
    try:
        env = dict(os.environ)
        for key, image, port, variables, command, health in [
            ('MY','mysql:8.0',3306, ['MYSQL_DATABASE=codisseu','MYSQL_ROOT_HOST=%','MYSQL_ROOT_PASSWORD=codisseu_fixture'], ['--skip-log-bin','--innodb-buffer-pool-size=64M'], ['mysqladmin','--protocol=tcp','-h127.0.0.1','-uroot','-pcodisseu_fixture','ping']),
            ('PG','postgres:18',5432,['POSTGRES_DB=codisseu','POSTGRES_PASSWORD=codisseu_fixture','PGDATA=/var/lib/postgresql/data/pgdata'], [], ['pg_isready','-h127.0.0.1','-U','postgres'])]:
            image_id = run('docker','image','inspect','--format','{{.Id}}',image).stdout.strip()
            name = 'codisseu-grid-' + key.lower() + '-' + token
            data = '/var/lib/mysql' if key == 'MY' else '/var/lib/postgresql/data'
            options = ['docker','run','--detach','--rm','--pull','never','--name',name,'--label',label,'--memory','1g','--cpus','2','--publish',f'127.0.0.1::{port}','--tmpfs',f'{data}:rw,size=512m']
            for variable in variables: options += ['--env', variable]
            cid = run(*options,image,*command).stdout.strip(); created.append(cid)
            end = time.monotonic()+90
            while subprocess.run(['docker','exec',cid,*health], capture_output=True, timeout=5).returncode:
                assert time.monotonic()<end, f'{key} fixture startup failed'; time.sleep(.3)
            address=run('docker','port',cid,f'{port}/tcp').stdout.strip(); assert address.startswith('127.0.0.1:')
            env[f'CODISSEU_GRID_{key}_PORT']=address.rsplit(':',1)[1]
            sql="CREATE TABLE grid_inspection(id BIGINT PRIMARY KEY,name VARCHAR(100),note TEXT,amount BIGINT); INSERT INTO grid_inspection VALUES(1,'John','SECRET1',9223372036854775807),(2,'Jane','SECRET2',0),(3,'JOHNNY',NULL,-1),(4,'acao','SECRET4',7);"
            client=['mysql','-uroot','-pcodisseu_fixture','codisseu','-N','-e'] if key=='MY' else ['psql','-v','ON_ERROR_STOP=1','-U','postgres','-d','codisseu','-At','-c']
            run('docker','exec',cid,*client,sql)
            version=run('docker','exec',cid,*client,'SELECT VERSION();').stdout.strip()
            report['servers'].append(dict(engine=key,image=image,image_id=image_id,container=cid,label=label,address=address,version=version,client=client))
        env['CODISSEU_GRID_CAPTURES']=str(out/'ui')
        with (out/'ui-tests.log').open('w') as log:
            result=subprocess.run(['cargo','test','-p','codisseu-ui','--lib','database_grid_tests::grid_remote','--locked','--jobs','2','--','--ignored','--test-threads=1'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=240)
        report['test_exit']=result.returncode
        for server in report['servers']:
            values=run('docker','exec',server['container'],*server['client'],'SELECT id,name FROM grid_inspection ORDER BY id;').stdout.strip()
            assert 'updated' not in values and '2\tJane' in values.replace('|','\t'), values
            server['independent_final_rows']=values
            server['final_rows_unchanged']=True
            server['rollback_verified']=result.returncode==0
        assert result.returncode==0, 'Grid test failed; see ui-tests.log'
        report['result']='passed'
    finally:
        for cid in reversed(created):
            labels=json.loads(run('docker','inspect','--format','{{json .Config.Labels}}',cid).stdout)
            assert labels.get('org.codisseu.fixture')==token
            run('docker','rm','--force',cid)
        report['containers_removed']=created
        (out/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    print(out/'manifest.json')

if __name__=='__main__': main()
