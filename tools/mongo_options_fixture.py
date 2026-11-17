#!/usr/bin/env python3
"""Owned loopback-only TLS Mongo replica set. Never reads corporate credentials."""
import base64, json, os, secrets, shutil, socket, subprocess, tempfile, time, uuid
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
OUT = Path(os.environ.get('CODISSEU_MONGO_FIXTURE_OUTPUT', str(ROOT / 'artifacts/mongo-compatibility-20261029'))).absolute()

def run(args, **kw):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=kw.pop('timeout', 30), **kw)

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(prefix='codisseu-mongo-tls-'))
    names = []
    report = {'image': 'mongo:6.0', 'synthetic_only': True, 'bind': '127.0.0.1', 'nodes': 2}
    status = 1
    try:
        report['image_id'] = run(['docker', 'image', 'inspect', 'mongo:6.0', '--format', '{{.Id}}']).stdout.strip()
        for prefix in ['ca', 'wrong-ca']:
            run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','2','-subj',f'/CN=Codisseu Fixture {prefix}','-keyout',str(home/f'{prefix}.key'),'-out',str(home/f'{prefix}.pem')])
        run(['openssl','req','-newkey','rsa:2048','-nodes','-subj','/CN=localhost','-keyout',str(home/'server.key'),'-out',str(home/'server.csr')])
        (home/'server.ext').write_text('subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth,clientAuth\n')
        run(['openssl','x509','-req','-in',str(home/'server.csr'),'-CA',str(home/'ca.pem'),'-CAkey',str(home/'ca.key'),'-CAcreateserial','-days','2','-extfile',str(home/'server.ext'),'-out',str(home/'server.crt')])
        (home/'server.pem').write_bytes((home/'server.key').read_bytes()+(home/'server.crt').read_bytes())
        (home/'keyfile').write_text(base64.b64encode(secrets.token_bytes(300)).decode()); (home/'keyfile').chmod(0o400)
        ports=[]
        for _ in range(2):
            with socket.socket() as probe:
                probe.bind(('127.0.0.1',0));ports.append(probe.getsockname()[1])
        token=uuid.uuid4().hex[:10]
        for i,port in enumerate(ports):
            name=f'codisseu-mongo-options-{token}-{i}';names.append(name)
            run(['docker','run','--pull','never','-d','--name',name,'--network','host','--read-only','--tmpfs',f'/data/db:rw,nosuid,size=512m,uid={os.getuid()},gid={os.getgid()},mode=700','--tmpfs','/tmp:rw,nosuid,size=32m','--memory','768m','--cpus','1','--pids-limit','256','--cap-drop','ALL','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}','--mount',f'type=bind,src={home},dst=/fixture,readonly','--entrypoint','mongod','mongo:6.0','--bind_ip','127.0.0.1','--port',str(port),'--dbpath','/data/db','--wiredTigerCacheSizeGB','0.25','--replSet','codisseu-rs','--keyFile','/fixture/keyfile','--tlsMode','requireTLS','--tlsAllowConnectionsWithoutCertificates','--tlsCAFile','/fixture/ca.pem','--tlsCertificateKeyFile','/fixture/server.pem'])
        def shell(index, script, auth=False):
            args=['docker','exec','-e','HOME=/tmp',names[index],'mongosh','--quiet','--norc','--host','127.0.0.1','--port',str(ports[index]),'--tls','--tlsCAFile','/fixture/ca.pem']
            if auth:args+=['--username','fixture_admin','--password','fixture-password','--authenticationDatabase','admin','--authenticationMechanism','SCRAM-SHA-1']
            return subprocess.run(args+['--eval',script],capture_output=True,text=True,timeout=10)
        def wait(check):
            end=time.monotonic()+70
            while True:
                result=check()
                if result.returncode==0:return
                if time.monotonic()>end or 'is not running' in result.stderr:raise RuntimeError('Owned Mongo fixture not ready: '+result.stderr[-600:])
                time.sleep(.3)
        for i in range(2):wait(lambda i=i:shell(i,'if(db.adminCommand({ping:1}).ok!==1)quit(1)'))
        result=shell(0,'rs.initiate({_id:"codisseu-rs",members:[{_id:0,host:"127.0.0.1:'+str(ports[0])+'",priority:2},{_id:1,host:"127.0.0.1:'+str(ports[1])+'",priority:0}]})')
        if result.returncode:raise RuntimeError('Owned replica set init failed')
        wait(lambda:shell(0,'if(!db.hello().isWritablePrimary)quit(1)'))
        result=shell(0,'db.getSiblingDB("admin").createUser({user:"fixture_admin",pwd:"fixture-password",roles:["root"],mechanisms:["SCRAM-SHA-1"]})')
        if result.returncode:raise RuntimeError('Owned fixture admin creation failed')
        result=shell(0,'db.getSiblingDB("admin").createUser({user:"fixture_reader",pwd:"fixture-password",roles:[{role:"read",db:"codisseu_fixture"}],mechanisms:["SCRAM-SHA-1"]});db.getSiblingDB("codisseu_fixture").users.insertOne({_id:1,name:"synthetic fixture"})',True)
        if result.returncode:raise RuntimeError('Owned fixture seed failed: '+result.stderr[-400:])
        wait(lambda:shell(1,'db.getMongo().setReadPref("secondary");if(db.getSiblingDB("codisseu_fixture").users.countDocuments({_id:1})!==1)quit(1)',True))
        env={**os.environ,'CODISSEU_MONGO_FIXTURE_PORT':str(ports[0]),'CODISSEU_MONGO_FIXTURE_SECONDARY_PORT':str(ports[1]),'CODISSEU_MONGO_FIXTURE_CA':str(home/'ca.pem'),'CODISSEU_MONGO_FIXTURE_WRONG_CA':str(home/'wrong-ca.pem'),'CODISSEU_MONGO_FIXTURE_REPORT':str(OUT/'local-sockets.json')}
        args=['cargo','test','-p','codisseu-db','--lib','real_mongo_tls_scram_replica_secondary_and_client_shutdown','--locked','--jobs','2','--','--ignored','--test-threads=1','--nocapture']
        with (OUT/'local-real.log').open('w') as log:
            status=subprocess.run(args,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=120).returncode
        report['test_exit']=status;report['command']=args
    finally:
        report['containers']=names; report['cleanup']=[]
        for name in reversed(names):
            logs=subprocess.run(['docker','logs',name],capture_output=True,text=True,timeout=10)
            (OUT/f'{name}.log').write_text(logs.stdout+logs.stderr)
            result=subprocess.run(['docker','rm','-f',name],capture_output=True,text=True,timeout=30)
            absent=subprocess.run(['docker','inspect',name],capture_output=True,text=True,timeout=10).returncode!=0
            report['cleanup'].append({'name':name,'absent':absent})
        shutil.rmtree(home);report['keys_and_ca_removed']=not home.exists()
        (OUT/'local-fixture.json').write_text(json.dumps(report,indent=2)+'\n')
    return status
if __name__=='__main__':raise SystemExit(main())
