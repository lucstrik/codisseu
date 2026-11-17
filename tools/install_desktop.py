#!/usr/bin/env python3
"""Install Codisseu's owned launcher/icon for the current Linux account only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
APP_ID = 'io.codisseu.Codisseu'
MARKER = '# Managed by Codisseu desktop integration\n'

def install(binary, data, bin_dir):
    binary = binary.expanduser().resolve(strict=True)
    if not os.access(binary, os.X_OK):
        raise RuntimeError('Build the Codisseu executable first')
    icon_root = data / 'icons/hicolor'
    targets = {}
    previous_by_target = {}
    provenance = json.loads((ROOT / 'assets/icons/provenance.json').read_text())
    sizes = provenance.get('sizes', [128, 256, 512])
    previous_icons = {}
    for entry in provenance.get('previous_assets', []):
        previous_icons.setdefault(Path(entry['path']).name, set()).add(entry['sha256'])
    for size in sizes:
        source = ROOT / f'assets/icons/{APP_ID}-{size}.png'
        target = icon_root / f'{size}x{size}/apps/{APP_ID}.png'
        targets[target] = source.read_bytes()
        previous_by_target[target] = previous_icons.get(source.name, set())
    # Plasma may retain a named QIcon after replacing its hicolor PNGs. An
    # immutable, content-addressed URL gives each approved brand new cache keys.
    icon_bytes = (ROOT / f'assets/icons/{APP_ID}-256.png').read_bytes()
    icon_hash = hashlib.sha256(icon_bytes).hexdigest()
    desktop_icon = (data / 'icons/codisseu' / f'{APP_ID}-{icon_hash}.png').absolute()
    targets[desktop_icon] = icon_bytes
    def quote(value):
        return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%') + '"'
    launcher = bin_dir / 'codisseu'
    shell_path = "'" + str(binary).replace("'", "'\\''") + "'"
    targets[launcher] = ('#!/bin/sh\n' + MARKER + 'exec ' + shell_path + ' --cli "$@"\n').encode()
    desktop = data / f'applications/{APP_ID}.desktop'
    targets[desktop] = (MARKER + '[Desktop Entry]\nType=Application\nVersion=1.0\nName=Codisseu\nComment=IDE nativa para PHP, JavaScript, TypeScript e bancos de dados\nExec=' + quote(binary) + '\nIcon=' + str(desktop_icon) + '\nTerminal=false\nCategories=Development;IDE;\nStartupNotify=true\nStartupWMClass=' + APP_ID + '\n').encode()
    # Refuse ownership conflicts before modifying any destination.
    for path, content in targets.items():
        if path.is_symlink():
            raise RuntimeError(f'Existing unmanaged link preserved: {path}')
        if path.exists() and path.read_bytes() != content:
            known_icon = path.suffix == '.png' and hashlib.sha256(path.read_bytes()).hexdigest() in previous_by_target.get(path, set())
            if not known_icon and (path.suffix == '.png' or b'# Managed by Codisseu' not in path.read_bytes()[:200]):
                raise RuntimeError(f'Existing unmanaged file preserved: {path}')
    path_helper = ROOT / 'tools/install_command_path.sh'
    subprocess.run(['sh', str(path_helper), '--check', str(bin_dir)], check=True, capture_output=True, text=True)
    records = []
    for path, content in targets.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.codisseu-', delete=False) as temporary:
            temporary.write(content)
            stage = Path(temporary.name)
        try:
            stage.chmod(0o755 if path == launcher else 0o644)
            os.replace(stage, path)
        finally:
            stage.unlink(missing_ok=True)
        records.append({'path': str(path), 'sha256': hashlib.sha256(content).hexdigest()})
    if shutil.which('desktop-file-validate'):
        subprocess.run(['desktop-file-validate', str(desktop)], check=True)
    if shutil.which('update-desktop-database'):
        subprocess.run(['update-desktop-database', str(desktop.parent)], check=True)
    if shutil.which('gtk-update-icon-cache'):
        subprocess.run(['gtk-update-icon-cache', '-f', '-t', str(icon_root)], check=True)
    path_setup = subprocess.run(['sh', str(path_helper), str(bin_dir)], check=True, capture_output=True, text=True).stdout.strip()
    return {'name': 'Codisseu', 'app_id': APP_ID, 'binary': str(binary), 'desktop_icon': str(desktop_icon), 'files': records, 'command_path': path_setup}

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/codisseu')
    parser.add_argument('--data-home', type=Path, default=Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share'))))
    parser.add_argument('--bin-dir', type=Path, default=Path.home() / '.local/bin')
    parser.add_argument('--record', type=Path, required=True)
    args = parser.parse_args()
    result = install(args.binary, args.data_home, args.bin_dir)
    args.record.parent.mkdir(parents=True, exist_ok=True)
    args.record.write_text(json.dumps(result, indent=2) + '\n')
    print(args.record)
