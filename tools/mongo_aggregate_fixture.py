#!/usr/bin/env python3
"""Exercise literal Mongo reads on an owned tmpfs/loopback container only."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def run(*args, **kw):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True,
                          text=True, timeout=kw.pop('timeout', 90), **kw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    token = uuid.uuid4().hex[:12]
    cid = None
    report = {'scope': 'owned tmpfs/loopback only; cached image; no personal profiles or QA'}
    status = 1
    try:
        image = 'mongo:6.0'
        report['image_id'] = run('docker', 'image', 'inspect', '--format', '{{.Id}}', image).stdout.strip()
        cid = run('docker', 'run', '--detach', '--rm', '--pull', 'never',
                  '--name', f'codisseu-aggregate-{token}', '--label', f'org.codisseu.aggregate={token}',
                  '--memory', '1g', '--cpus', '2', '--publish', '127.0.0.1::27017',
                  '--tmpfs', '/data/db:rw,size=512m', image, '--bind_ip_all', '--quiet').stdout.strip()
        report['container'] = cid
        end = time.monotonic() + 90
        while subprocess.run(['docker', 'exec', cid, 'mongosh', '--quiet', '--eval', 'db.runCommand({ping:1}).ok'],
                             capture_output=True, timeout=10).returncode:
            assert time.monotonic() < end, 'Mongo startup timeout'
            time.sleep(.3)
        address = run('docker', 'port', cid, '27017/tcp').stdout.strip()
        assert address.startswith('127.0.0.1:')
        report['address'] = address
        report['version'] = run('docker', 'exec', cid, 'mongod', '--version').stdout
        query = (ROOT / 'fixtures/databases/mongo/aggregate-cte.js').read_text()
        ids = [int(n) for n in re.findall(r'\d+', query.split('$in: [', 1)[1].split(']', 1)[0])]
        assert len(ids) == len(set(ids)) == 56
        seed = '''
const ids = IDS;
const docs = [];
for (const [index, id] of ids.entries()) {
    for (let rev=0; rev<3; ++rev) {
        const latest = rev===2;
        docs.push({_id:index*3+rev+1, remetenteId:42,
            processor:{cte:{fatCteId:id, awb:`fixture-${id}-${latest?'latest':'old'}`,
                dataAutorizacao:ISODate('2025-01-02T00:00:00Z')}, encomenda:{status:latest?'latest':'old'}},
            dataCriacao:ISODate(rev===0?'2025-01-01T00:00:00Z':'2025-01-02T00:00:00Z'),
            dataAutorizacao:ISODate('2025-01-02T00:00:00Z'), documentoStatus:latest?'authorized':'old'});
    }
}
docs.push({_id:10001,remetenteId:999,processor:{cte:{fatCteId:ids[0]}},dataCriacao:ISODate('2030-01-01T00:00:00Z')});
docs.push({_id:10002,remetenteId:42,processor:{cte:{fatCteId:-1}},dataCriacao:ISODate('2030-01-01T00:00:00Z')});
db.getSiblingDB('codisseu_aggregate_fixture').getCollection('sample_documents').insertMany(docs);
db.getSiblingDB('codisseu_fixture').users.insertMany([{_id:1,label:'first'},{_id:2,label:'second'}]);
'''.replace('IDS', json.dumps(ids))
        run('docker', 'exec', cid, 'mongosh', '--quiet', '--eval', seed)
        expected = json.loads(run('docker', 'exec', cid, 'mongosh', '--quiet', '--eval',
                                 'JSON.stringify(' + query.strip().removesuffix(';') + '.toArray())').stdout)
        assert [d['fatCteId'] for d in expected] == sorted(ids)
        for d in expected:
            assert d['quantidadeDocumentos'] == 3
            assert d['awb'] == f"fixture-{d['fatCteId']}-latest"
            assert d['statusEncomenda'] == 'latest'
        (out / 'independent-result.json').write_text(json.dumps(expected, indent=2) + '\n')
        env = dict(os.environ, CODISSEU_QUERY_MONGO_PORT=address.rsplit(':', 1)[1],
                   CODISSEU_QUERY_EXEC_CAPTURES=str(out / 'ui'))
        with (out / 'ui-real.log').open('w') as log:
            result = subprocess.run(['cargo', 'test', '-p', 'codisseu-ui', '--lib',
                                     'query_execution_tests::mongo_shell_aggregate_real', '--locked',
                                     '--jobs', '2', '--', '--ignored', '--test-threads=1'], cwd=ROOT,
                                    env=env, stdout=log, stderr=subprocess.STDOUT, timeout=300)
        report['test_exit'] = result.returncode
        counts = json.loads(run('docker', 'exec', cid, 'mongosh', '--quiet', '--eval',
                                "JSON.stringify([db.getSiblingDB('codisseu_aggregate_fixture').getCollection('sample_documents').countDocuments({}),db.getSiblingDB('codisseu_fixture').users.countDocuments({})])").stdout)
        assert counts == [170, 2], counts
        report.update(independent_groups=len(expected), independent_counts_after=counts)
        assert result.returncode == 0, 'UI failed; see ui-real.log'
        status = 0
    finally:
        if cid:
            labels = json.loads(run('docker', 'inspect', '--format', '{{json .Config.Labels}}', cid).stdout)
            assert labels.get('org.codisseu.aggregate') == token
            run('docker', 'rm', '--force', cid)
            report['cleanup_absent'] = subprocess.run(['docker', 'inspect', cid], capture_output=True).returncode != 0
        report['exit'] = status
        (out / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    return status


if __name__ == '__main__':
    raise SystemExit(main())
