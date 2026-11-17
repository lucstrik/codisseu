#!/usr/bin/env python3
"""Compare PHAR, extracted runtime and legacy-compatible launcher in private fixtures."""
import json
import os
import pathlib
import signal
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent


def probe(entry):
    with tempfile.TemporaryDirectory(prefix='codisseu-provider-storage-') as tmp:
        base = pathlib.Path(tmp)
        cache, workspace, temp = (base / name for name in ['cache', 'workspace', 'temp'])
        for path in [cache, workspace, temp]:
            path.mkdir()
        (workspace / 'test.php').write_text('<?php class Example {}')
        env = {'HOME': str(base), 'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
               'XDEBUG_MODE': 'off', 'XDG_CACHE_HOME': str(cache), 'TMPDIR': str(temp)}
        for _ in range(4):
            child = subprocess.Popen(
                ['/usr/bin/php', '-d', 'xdebug.mode=off', '-d', f'sys_temp_dir={temp}',
                 str(entry), 'language-server'], cwd=workspace, env=env,
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL, start_new_session=True)
            try:
                request = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize',
                           'params': {'processId': os.getpid(), 'rootUri': workspace.as_uri(),
                                      'capabilities': {}}}
                body = json.dumps(request).encode()
                child.stdin.write(f'Content-Length: {len(body)}\r\n\r\n'.encode() + body)
                child.stdin.flush()
                # This deliberately injects process failure, not a readiness assertion.
                time.sleep(.7)
            finally:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=5)
                child.stdin.close()
        files = list(temp.iterdir())
        return {'forced_stops': 4, 'temporary_count': len(files),
                'temporary_bytes': sum(path.stat().st_size for path in files)}


if __name__ == '__main__':
    entries = {'phar': ROOT / '.tools/downloads/phpactor-2026.10.07.0.phar',
               'unpacked': ROOT / '.tools/phpactor-2026.10.07.0/bin/phpactor',
               'compatibility': ROOT / '.tools/phpactor.phar'}
    results = {name: probe(path) for name, path in entries.items()}
    for name, path in entries.items():
        version = subprocess.run(['/usr/bin/php', '-d', 'xdebug.mode=off', str(path), '--version'],
                                 check=True, capture_output=True, text=True, timeout=10)
        results[name]['version'] = version.stdout.strip()
    print(json.dumps(results, indent=2))
    if any(results[name]['temporary_bytes'] != 0 for name in ['unpacked', 'compatibility']):
        raise SystemExit('Extracted provider/compatibility launcher left nonempty temporary files')
