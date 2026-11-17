#!/usr/bin/env python3
"""Replace only the installed Codisseu executable, preserving desktop and profiles."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[1]
source = ROOT / 'target/delivery-release/release/codisseu'
target = Path.home() / '.local/share/codisseu/bin/codisseu'
record = ROOT / 'artifacts/new-window-20261106/desktop-update.json'
staged = None
result = {'source': str(source), 'target': str(target), 'installed': False}
try:
    assert source.is_file() and os.access(source, os.X_OK), 'Build release first'
    assert target.is_file() and not target.is_symlink(), 'Expected existing desktop installation'
    expected = hashlib.file_digest(source.open('rb'), 'sha256').hexdigest()
    result['source_sha256'] = expected
    # Stage before touching the running executable. Atomic replacement allows
    # existing windows to remain open until the user chooses to restart them.
    fd, name = tempfile.mkstemp(prefix='.codisseu-update-', dir=target.parent)
    os.close(fd)
    staged = Path(name)
    shutil.copyfile(source, staged)
    staged.chmod(0o755)
    assert hashlib.file_digest(staged.open('rb'), 'sha256').hexdigest() == expected
    old_hash = hashlib.file_digest(target.open('rb'), 'sha256').hexdigest()
    backup = record.parent / ('installed-before-' + old_hash[:12])
    if not backup.exists():
        shutil.copy2(target, backup)
    os.replace(staged, target)
    result.update(installed=True, previous_sha256=old_hash, backup=str(backup))
    assert hashlib.file_digest(target.open('rb'), 'sha256').hexdigest() == expected
except Exception as error:
    result['error'] = str(error)
finally:
    if staged is not None:
        staged.unlink(missing_ok=True)
    record.parent.mkdir(parents=True, exist_ok=True)
    record.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
raise SystemExit(0 if result['installed'] else 1)
