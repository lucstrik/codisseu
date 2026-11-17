"""Unpack the pinned Phpactor distribution once, without executing its PHAR stub."""
import hashlib
import json
import os
import pathlib
import re
import shutil
import stat
import subprocess
import tempfile

PHPACTOR_VERSION = '2026.10.07.0'
PHPACTOR_SHA256 = 'b28bf6166e016d539fc2f2aec18ef918b12590e436a6f1ccbdf30b5bbdbdb16b'


def launcher_bytes(version=PHPACTOR_VERSION, digest=PHPACTOR_SHA256):
    return (f'<?php\n// Codisseu managed Phpactor compatibility launcher v1.\n'
            f'// runtime-version: {version}\n// source-sha256: {digest}\n'
            f"require __DIR__ . '/phpactor-{version}/bin/phpactor';\n").encode('ascii')


def _managed_launcher(data):
    match = re.fullmatch(
        rb'<\?php\n// Codisseu managed Phpactor compatibility launcher v1\.\n'
        rb'// runtime-version: ([0-9.]+)\n// source-sha256: ([a-f0-9]{64})\n.*',
        data, re.DOTALL)
    return bool(match and data == launcher_bytes(match[1].decode(), match[2].decode()))


def _identity(info):
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns


def _managed_snapshot(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise ValueError(f'Compatibility path is not a single regular managed file: {path}')
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as source:
        if _identity(os.fstat(source.fileno())) != _identity(info):
            raise ValueError('Compatibility path changed during inspection')
        data = source.read(16 * 1024 * 1024 + 1)
        if _identity(os.fstat(source.fileno())) != _identity(info):
            raise ValueError('Compatibility file changed during inspection')
    if _identity(path.lstat()) != _identity(info):
        raise ValueError('Compatibility path changed during inspection')
    if hashlib.sha256(data).hexdigest() != PHPACTOR_SHA256 and not _managed_launcher(data):
        raise ValueError(f'Refusing to replace an unmanaged compatibility file: {path}')
    return info, data


def _exchange(left, right):
    """Swap names atomically, retaining both inodes for post-publication checking."""
    import ctypes
    library = ctypes.CDLL(None, use_errno=True)
    try:
        exchange = library.renameat2
    except AttributeError as error:
        raise OSError('Atomic compatibility migration requires Linux renameat2') from error
    exchange.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
                         ctypes.c_char_p, ctypes.c_uint]
    exchange.restype = ctypes.c_int
    if exchange(-100, os.fsencode(left), -100, os.fsencode(right), 2):
        raise OSError(ctypes.get_errno(), 'Atomic compatibility exchange failed')


def install_compatibility_launcher(path, entry):
    """Migrate only the pinned archive or an exact authored launcher.

    Exchange preserves the old inode until it is verified again. A concurrent
    foreign file/symlink is restored, never unlinked. An unexpected third writer
    leaves its predecessor at the recovery path instead of deleting its data.
    """
    path, entry = pathlib.Path(path), pathlib.Path(entry)
    expected_entry = path.parent / f'phpactor-{PHPACTOR_VERSION}/bin/phpactor'
    if entry != expected_entry or entry.is_symlink() or not entry.is_file():
        raise ValueError('Compatibility launcher requires the managed extracted runtime')
    marker = entry.parent.parent / 'codisseu-runtime.json'
    if json.loads(marker.read_text()).get('source_sha256') != PHPACTOR_SHA256:
        raise ValueError('Compatibility runtime source digest mismatch')
    content = launcher_bytes()
    try:
        previous, data = _managed_snapshot(path)
        if data == content:
            return path
    except FileNotFoundError:
        previous = None
    handle, temporary_name = tempfile.mkstemp(prefix='.phpactor-compat-', dir=path.parent)
    temporary = pathlib.Path(temporary_name)
    removable = set()
    try:
        with os.fdopen(handle, 'wb') as output:
            output.write(content)
            output.flush()
            os.fchmod(output.fileno(), (previous.st_mode & 0o777) if previous else 0o644)
            os.fsync(output.fileno())
            own = output.fileno()
            removable.add((os.fstat(own).st_dev, os.fstat(own).st_ino))
        if previous is None:
            # Atomic create-if-absent. Never replace a target appearing meanwhile.
            os.link(temporary, path, follow_symlinks=False)
        else:
            _exchange(temporary, path)
            try:
                displaced, _ = _managed_snapshot(temporary)
                if _identity(displaced) != _identity(previous):
                    raise ValueError('Compatibility file was replaced during migration')
                current = path.lstat()
                if (current.st_dev, current.st_ino) not in removable:
                    raise ValueError('Compatibility destination changed during migration')
                removable.add((displaced.st_dev, displaced.st_ino))
            except (OSError, ValueError):
                current = path.lstat()
                if (current.st_dev, current.st_ino) in removable:
                    _exchange(temporary, path)
                else:
                    raise ValueError(f'Concurrent writer; previous item preserved at {temporary}')
                raise
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        try:
            current = temporary.lstat()
            if (current.st_dev, current.st_ino) in removable:
                temporary.unlink()
        except FileNotFoundError:
            pass
    return path


def unpack_phpactor(phar, destination, php):
    """Install to a new directory, preserving every upstream file and notice.

    The immutable destination is never replaced while a provider may use it.
    A failed installation removes only its private staging directory.
    """
    phar, destination = pathlib.Path(phar).resolve(), pathlib.Path(destination)
    digest = hashlib.sha256(phar.read_bytes()).hexdigest()
    if digest != PHPACTOR_SHA256:
        raise ValueError('Phpactor archive checksum mismatch')
    marker = destination / 'codisseu-runtime.json'
    if destination.exists():
        if (marker.is_file() and json.loads(marker.read_text()).get('source_sha256') == digest
                and (destination / 'bin/phpactor').is_file()
                and (destination / 'vendor/autoload.php').is_file()):
            return destination / 'bin/phpactor'
        raise ValueError(f'Existing Phpactor runtime is not managed: {destination}')
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = pathlib.Path(tempfile.mkdtemp(prefix='.phpactor-install-', dir=destination.parent))
    try:
        # Read the trusted archive through Phar, never require/execute its stub.
        script = ('$archive = new Phar($argv[1]); '
                  '$archive->extractTo($argv[2]);')
        subprocess.run([str(php), '-d', 'xdebug.mode=off', '-r', script,
                        str(phar), str(staging)], check=True, timeout=60,
                       stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for required in ['bin/phpactor', 'vendor/autoload.php', 'LICENSE']:
            if not (staging / required).is_file():
                raise ValueError(f'Incomplete Phpactor runtime: {required}')
        files = {str(path.relative_to(staging)): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(staging.rglob('*')) if path.is_file()}
        record = {'schema': 1, 'version': PHPACTOR_VERSION,
                  'source_sha256': digest, 'format': 'unpacked-phar', 'files': files}
        (staging / 'codisseu-runtime.json').write_text(json.dumps(record, indent=2) + '\n')
        staging.rename(destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return destination / 'bin/phpactor'
