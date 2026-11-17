#!/usr/bin/env python3
"""Capture the real Storm M2 window: SQLite results, LSP problems and restored session."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--providers', type=Path, default=ROOT / '.tools/providers.json')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/codisseu')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    providers = args.providers.resolve()
    assert binary.is_file() and providers.is_file(), 'Compile Storm e instale os providers primeiro'
    project = out / 'workspace'
    shutil.copytree(ROOT / 'fixtures/fullstack', project)
    php, ts, sql = [project / name for name in ('src/index.php', 'src/app.ts', 'query.sql')]
    php.write_text("<?php\n// Fixture: erro de sintaxe intencional para diagnóstico real.\necho '🌧'; $broken = ;\n")
    ts.write_text("// Fixture: tipo incompatível intencional para diagnóstico real.\nexport const emoji = '🌧';\nexport const count: number = 'wrong';\n")
    (project / 'tsconfig.json').write_text('{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true},"include":["src/*.ts"]}\n')
    db = project / 'workspace.sqlite'
    with sqlite3.connect(db) as connection:
        connection.execute('CREATE TABLE developers(id INTEGER PRIMARY KEY, name TEXT NOT NULL, language TEXT, notes TEXT, payload BLOB)')
        connection.executemany('INSERT INTO developers VALUES (?,?,?,?,?)', [
            (1, 'Ana', 'PHP', 'Olá, ação', b'\x00\x01'),
            (2, 'Lucas', 'TypeScript', None, b'\x53\x51\x4c'),
            (9007199254740993, 'Storm', 'SQL', 'inteiro i64 exato', b'')])
    sql.write_text('-- SQLite real · mesma sessão de PHP e TypeScript\nSELECT id, name, language, notes, payload\nFROM developers\nORDER BY id;\n')
    files = [php, ts, sql]
    before = {str(p.relative_to(project)): sha(p) for p in files + [db]}
    session = out / 'session.json'
    shared = ['--database', str(db), '--providers', str(providers), '--start-providers', '--execute-sql', '--wait-nonempty-diagnostics']
    runs = [
        ('results', ['--no-restore', '--workspace', str(project), *sum((['--open', str(p)] for p in files), []), *shared]),
        ('problems', [*shared, '--problems']),
        ('restored', ['--wait-nonempty-diagnostics']),
    ]
    observed = []
    for name, extra in runs:
        capture = out / f'{name}.png'
        command = [str(binary), '--session', str(session), *extra, '--capture', str(capture), '--close-after-capture']
        started = time.monotonic()
        with (out / f'{name}.log').open('w') as log:
            result = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            try:
                executable = Path(f'/proc/{result.pid}/exe')
                deadline = time.monotonic() + 5
                while executable.resolve() != binary.resolve():
                    assert result.poll() is None and time.monotonic() < deadline, 'Own process did not exec the expected application'
                    time.sleep(.01)
                binary_sha256 = sha(executable)
                result.wait(timeout=60)
            finally:
                if result.poll() is None:
                    result.terminate()
                    try: result.wait(timeout=5)
                    except subprocess.TimeoutExpired: result.kill(); result.wait()
        assert result.returncode == 0, f'{name}: código {result.returncode}; veja log'
        png = capture.read_bytes()
        assert png[:8] == b'\x89PNG\r\n\x1a\n'
        metadata = json.loads(capture.with_suffix('.json').read_text())
        state = metadata['observed']
        assert [d['path'] for d in state['documents']] == [str(p) for p in files]
        persisted = json.loads(session.read_text())
        assert persisted['schema_version'] == 2
        assert persisted['database_path'] == str(db)
        assert len(persisted['provider_configs']) == 2
        if name != 'restored':
            assert state['sqlite_connected'] and state['sqlite_result_rows'] == 3
            assert state['sqlite_schema'] == ['developers']
            assert len(state['providers']) == 2 and all(p['state'] == 'Ready' for p in state['providers'])
            assert {d['language'] for d in state['diagnostics']} == {'Php', 'TypeScript'}
            assert state['analyzed_versions'][str(php)] == 1 and state['analyzed_versions'][str(ts)] == 1
            assert state['panel'] == name
        else:
            assert not state['sqlite_connected'], 'restauração não conecta SQLite sem ação explícita'
            assert len(state['providers']) == 2 and all(p['state'] == 'Ready' for p in state['providers'])
            assert state['provider_startup']['trusted'] and not state['provider_startup']['needs_trust']
            assert {d['language'] for d in state['diagnostics']} == {'Php', 'TypeScript'}
        observed.append({'name': name, 'binary_sha256': binary_sha256, 'command': command, 'exit_code': result.returncode, 'wall_seconds': round(time.monotonic()-started, 3), 'png_sha256': sha(capture), 'size_px': struct.unpack('>II', png[16:24]), 'metadata': metadata})
        print(f'{name}: janela real e estado verificados', flush=True)
    assert before == {str(p.relative_to(project)): sha(p) for p in files + [db]}, 'leitura não altera fontes ou base'
    (out / 'observed-runs.json').write_text(json.dumps(observed, ensure_ascii=False, indent=2)+'\n')
    assert len({run['binary_sha256'] for run in observed}) == 1, 'Binário mudou entre as execuções; repetir com build estável'
    manifest = {'kind': 'M2 native framebuffer; actual SQLite/LSP; no compositor input injection', 'binary_sha256': observed[0]['binary_sha256'], 'binary_hash_source': '/proc/<own-pid>/exe at each launch', 'cargo_lock_sha256': sha(ROOT / 'Cargo.lock'), 'providers_sha256': sha(providers), 'fixture_sha256': before, 'runs': observed}
    (out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print(f'3 execuções nativas verificadas: {out}')


if __name__ == '__main__':
    run()
