#!/usr/bin/env python3
"""Resize the supplied Codisseu master losslessly in identity, using ImageMagick.

No redraw/crop/background replacement. --check never writes assets.
"""
import argparse
import hashlib
import json
import struct
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / 'assets/icons'
MANIFEST = ICONS / 'provenance.json'
SIZES = [16, 24, 32, 48, 64, 96, 128, 256, 512, 1024]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dimensions(path):
    header = path.read_bytes()[:33]
    assert header[:8] == b'\x89PNG\r\n\x1a\n', path
    return list(struct.unpack('>II', header[16:24]))


def render():
    old = json.loads(MANIFEST.read_text())
    history = old.get('previous_assets', [])[:]
    # Keep only known earlier Codisseu icon bytes as safe upgrade candidates.
    for entry in old.get('files', []):
        if entry['path'].startswith('assets/icons/') and entry not in history:
            history.append(entry)
    source = ROOT / 'codisseu.png'
    assert dimensions(source)[0] == dimensions(source)[1], 'Master must be square; never stretch it'
    for size in SIZES:
        subprocess.run(['magick', str(source), '-filter', 'Lanczos', '-resize',
                        f'{size}x{size}', '-strip',
                        f'PNG32:{ICONS / f"io.codisseu.Codisseu-{size}.png"}'], check=True)
    paths = [source, *[ICONS / f'io.codisseu.Codisseu-{size}.png' for size in SIZES]]
    manifest = {
        'source': 'PNG supplied by user on2026-10-30 at /home/usuario/Downloads/codisseu.png',
        'creator': 'User-supplied artwork; creator not independently verified',
        'authorization': 'User explicitly requested replacing all application branding with this image',
        'scope': 'Application/window/desktop/dock/portable branding; language and action icons retain their meanings',
        'transformations': 'Master bytes unchanged; proportional square Lanczos resizes, no crop/redraw/color/background changes',
        'sizes': SIZES,
        'master_dimensions': dimensions(source),
        'renderer': subprocess.check_output(['magick', '-version'], text=True).splitlines()[0],
        'files': [{'path': str(path.relative_to(ROOT)), 'sha256': digest(path),
                   'bytes': path.stat().st_size, 'dimensions': dimensions(path)} for path in paths],
        'previous_assets': history,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')


def check():
    manifest = json.loads(MANIFEST.read_text())
    assert manifest['sizes'] == SIZES
    assert len(manifest['files']) == len(SIZES) + 1
    for entry in manifest['files']:
        path = ROOT / entry['path']
        assert digest(path) == entry['sha256'], path
        assert dimensions(path) == entry['dimensions'], path
    for size in SIZES:
        path = ICONS / f'io.codisseu.Codisseu-{size}.png'
        assert dimensions(path) == [size, size], path
        assert path.read_bytes()[25] == 6, 'Derived PNG must retain RGBA'
    print(f'Codisseu master and {len(SIZES)} proportional RGBA icon sizes verified')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--render', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.render:
        render()
    check()
