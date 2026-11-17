#!/usr/bin/env python3
"""Install only pinned, hash-verified standalone adapters into .tools/debug.

No VS Code host, global changes, package lifecycle scripts, or automatic updates.
Run --verify for a read-only installation check. Python standard library only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "tools/debug-adapters.json"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_entry(root, item):
    target = root / item["directory"]
    for path, expected in [(item["entrypoint"], item["entrypoint_sha256"]),
                           (item["license_path"], item["license_sha256"])]:
        found = target / path
        if not found.is_file() or digest(found) != expected:
            raise RuntimeError(f"Missing or modified adapter file: {found}")


def safe_member(name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name:
        raise RuntimeError(f"Unsafe archive member: {name}")


def install(root, item):
    target = root / item["directory"]
    if target.exists():
        verify_entry(root, item)
        return
    root.mkdir(parents=True, exist_ok=True)
    archive = root / item["archive"]
    if not archive.exists():
        with tempfile.NamedTemporaryFile(dir=root, delete=False) as output:
            temporary = Path(output.name)
            try:
                request = urllib.request.Request(item["url"], headers={"User-Agent": "Codisseu-debug-installer/1"})
                with urllib.request.urlopen(request, timeout=60) as response:
                    limit = item["archive_bytes"] + 1
                    while limit:
                        chunk = response.read(min(limit, 65536))
                        if not chunk:
                            break
                        output.write(chunk)
                        limit -= len(chunk)
                output.flush()
                if temporary.stat().st_size != item["archive_bytes"] or digest(temporary) != item["archive_sha256"]:
                    raise RuntimeError("Adapter download failed size/SHA-256 validation")
                os.replace(temporary, archive)
            finally:
                temporary.unlink(missing_ok=True)
    if archive.stat().st_size != item["archive_bytes"] or digest(archive) != item["archive_sha256"]:
        raise RuntimeError(f"Archive SHA-256 mismatch: {archive}")
    with tempfile.TemporaryDirectory(dir=root, prefix="extract-") as temporary:
        stage = Path(temporary)
        if archive.name.endswith(".tar.gz"):
            with tarfile.open(archive) as source:
                for member in source.getmembers():
                    safe_member(member.name)
                    if member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
                        raise RuntimeError(f"Unsupported archive member: {member.name}")
                source.extractall(stage, filter="data")
        else:
            with zipfile.ZipFile(archive) as source:
                for member in source.infolist():
                    safe_member(member.filename)
                    if stat.S_ISLNK(member.external_attr >> 16):
                        raise RuntimeError("Symlink in adapter archive")
                source.extractall(stage)
        # Entry paths are relative to the selected adapter's extraction root.
        adapter_root = stage
        for path, expected in [(item["entrypoint"], item["entrypoint_sha256"]),
                               (item["license_path"], item["license_sha256"])]:
            if digest(adapter_root / path) != expected:
                raise RuntimeError("Extracted adapter did not match the pinned entry point")
        os.replace(stage, target)
    verify_entry(root, item)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="verify without writing or downloading")
    parser.add_argument("--destination", type=Path, default=ROOT / ".tools/debug")
    args = parser.parse_args()
    data = json.loads(MANIFEST.read_text())
    for item in data["adapters"]:
        if args.verify:
            verify_entry(args.destination, item)
        else:
            install(args.destination, item)
        print(f"{item['id']} {item['version']}: verified")


if __name__ == "__main__":
    main()
