#!/usr/bin/env python3
"""Explicit local provider installation. No npm/composer project hooks are executed."""
import base64
import hashlib
import json
import pathlib
import shutil
import tarfile
import urllib.request
from providers.phpactor_runtime import PHPACTOR_VERSION, unpack_phpactor, install_compatibility_launcher

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEST = ROOT / '.tools'
DOWNLOADS = DEST / 'downloads'
DOWNLOADS.mkdir(parents=True, exist_ok=True)


def fetch(url, name, digest, algorithm='sha256'):
    path = DOWNLOADS / name
    if not path.exists():
        with urllib.request.urlopen(url, timeout=60) as response, path.open('wb') as out:
            shutil.copyfileobj(response, out)
    actual = hashlib.new(algorithm, path.read_bytes()).hexdigest()
    if actual != digest:
        raise RuntimeError(f'Checksum mismatch for {name}: {actual}')
    return path


node_name = 'node-v24.22.0-linux-x64.tar.xz'
node = fetch('https://nodejs.org/dist/v24.22.0/' + node_name, node_name,
             'cc2f601cee82d9a5a233302fe307abc600727035fd7d2c3a3571755989e95008')
with tarfile.open(node) as archive:
    archive.extractall(DEST, filter='data')
phar = fetch('https://github.com/phpactor/phpactor/releases/download/2026.10.07.0/phpactor.phar',
             'phpactor-2026.10.07.0.phar', 'b28bf6166e016d539fc2f2aec18ef918b12590e436a6f1ccbdf30b5bbdbdb16b')
php = shutil.which('php') or '/usr/bin/php'
phpactor = unpack_phpactor(phar, DEST / f'phpactor-{PHPACTOR_VERSION}', php)
install_compatibility_launcher(DEST / 'phpactor.phar', phpactor)
for package in json.loads((ROOT / 'tools/providers/npm-sources.json').read_text()):
    algorithm, encoded = package['integrity'].split('-', 1)
    archive_path = fetch(package['url'], f"{package['name']}-{package['version']}.tgz",
                         base64.b64decode(encoded).hex(), algorithm)
    target = DEST / 'node_modules' / package['name']
    target.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive_path) as archive:
        for member in archive.getmembers():
            if not member.name.startswith('package/'):
                raise RuntimeError('Unexpected npm archive path')
            member.name = member.name[len('package/'):]
            if member.name:
                archive.extract(member, target, filter='data')
configs = [
    {'language': 'Php', 'program': php,
     'args': ['-d', 'xdebug.mode=off', str(phpactor), '--config-extra={"language_server_worse_reflection.workspace_index.update_interval":0,"worse_reflection.enable_cache":false,"indexer.exclude_patterns":["/vendor/**/Tests/**/*","/vendor/**/tests/**/*","/vendor/composer/**/*","/vendor/rector/rector/stubs-rector","/.tools/**/*","/target/**/*","/artifacts/**/*","/node_modules/**/*","/storage/framework/**/*","/bootstrap/cache/**/*"]}', 'language-server']},
    {'language': 'TypeScript', 'program': str(DEST / 'node-v24.22.0-linux-x64/bin/node'),
     'args': [str(DEST / 'node_modules/typescript-language-server/lib/cli.mjs'), '--stdio']},
]
(DEST / 'providers.json').write_text(json.dumps(configs, indent=2) + '\n')
manifest = {'node': '24.22.0', 'phpactor': PHPACTOR_VERSION, 'phpactor_format': 'unpacked-phar',
            'phpactor_legacy_entrypoint': 'phpactor.phar: managed plain-PHP compatibility launcher',
            'typescript-language-server': '6.0.2', 'typescript': '6.0.3',
            'files': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in DOWNLOADS.iterdir() if p.is_file()}}
(DEST / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print('Local providers installed:', DEST / 'providers.json')
