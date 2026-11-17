#!/usr/bin/env python3
"""Install the pinned standalone CodeLLDB distribution privately; never run scripts."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import tempfile
import urllib.request
import zipfile

VERSION = "1.12.3"
SHA256 = "1cd7f386598022b51a5b93b9ffa23e812b23f519cfe1833384ec4bef4bfd1be1"
SIZE = 55571094
URL = f"https://github.com/vadimcn/codelldb/releases/download/v{VERSION}/codelldb-linux-x64.vsix"
ROOT = Path(__file__).resolve().parents[1]
NOTICES = {
    "CodeLLDB-LICENSE.txt": (f"https://raw.githubusercontent.com/vadimcn/codelldb/v{VERSION}/LICENSE", "6eedaf734957b02e4bcc408ea0ecb55de0f6b2ab18e92339327e863fdad86aed"),
    "LLVM-LICENSE.txt": ("https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-22.1.8/LICENSE.TXT", "8d85c1057d742e597985c7d4e6320b015a9139385cff4cbae06ffc0ebe89afee"),
    "Python-LICENSE.txt": ("https://raw.githubusercontent.com/python/cpython/v3.12.7/LICENSE", "3b2f81fe21d181c499c59a256c8e1968455d6689d269aa85373bfb6af41da3bf"),
}
ENTRYPOINTS = {
    "extension/adapter/codelldb": "8d714773b3416190bad90c6f5fae46e0d3c106bc3b7778a9547a8eb96cff219e",
    "extension/lldb/lib/liblldb.so": "e8f0064f967452f551a8a17264606c63929a44b77e416190782bee63816e9d9e",
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify(destination):
    record = json.loads((destination / "codisseu-install.json").read_text())
    if record["version"] != VERSION or record["archive_sha256"] != SHA256:
        raise RuntimeError("Unexpected CodeLLDB installation")
    if not all("NOTICES/" + name in record["files"] for name in NOTICES):
        raise RuntimeError("CodeLLDB installation lacks preserved upstream license notices")
    for name, expected in ENTRYPOINTS.items():
        if record["files"].get(name) != expected:
            raise RuntimeError("Unexpected CodeLLDB native entrypoint identity")
    for name, expected in record["files"].items():
        relative = PurePosixPath(name)
        if relative.is_absolute() or ".." in relative.parts or "\\" in name:
            raise RuntimeError("Invalid CodeLLDB verification path")
        path = destination / name
        if path.is_symlink() or not path.resolve().is_relative_to(destination.resolve()) or not path.is_file() or digest(path) != expected:
            raise RuntimeError(f"CodeLLDB resource missing or modified: {name}")
    if not os.access(destination / "extension/adapter/codelldb", os.X_OK):
        raise RuntimeError("CodeLLDB native adapter is not executable")
    print(json.dumps({"version": VERSION, "verified_files": len(record["files"]), "adapter": str(destination / "extension/adapter/codelldb")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--destination", type=Path, default=ROOT / ".tools/debug/codelldb")
    args = parser.parse_args()
    destination = args.destination.absolute()
    if destination.is_symlink():
        raise RuntimeError("Refusing a symlink installation destination")
    if destination.exists():
        verify(destination)
        return
    if args.verify:
        raise RuntimeError("CodeLLDB is not installed")
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("This pinned installer currently supports Linux x86_64")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="codisseu-codelldb-", dir=destination.parent) as work:
        work = Path(work)
        archive = work / "codelldb.vsix"
        request = urllib.request.Request(URL, headers={"User-Agent": "Codisseu-tool-installer"})
        with urllib.request.urlopen(request, timeout=60) as incoming, archive.open("wb") as out:
            total = 0
            while block := incoming.read(1024 * 1024):
                total += len(block)
                if total > SIZE:
                    raise RuntimeError("CodeLLDB download exceeds pinned size")
                out.write(block)
        if archive.stat().st_size != SIZE or digest(archive) != SHA256:
            raise RuntimeError("CodeLLDB archive integrity verification failed")
        extracted = work / "payload"
        extracted.mkdir()
        with zipfile.ZipFile(archive) as bundle:
            entries = bundle.infolist()
            if len(entries) > 20000 or sum(entry.file_size for entry in entries) > 512 * 1024 * 1024:
                raise RuntimeError("CodeLLDB archive exceeds extraction limits")
            for entry in entries:
                name = PurePosixPath(entry.filename)
                mode = entry.external_attr >> 16
                if name.is_absolute() or ".." in name.parts or "\\" in entry.filename or stat.S_ISLNK(mode):
                    raise RuntimeError("Unsafe CodeLLDB archive entry")
                target = extracted.joinpath(*name.parts)
                if entry.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.open(entry) as incoming, target.open("xb") as out:
                    shutil.copyfileobj(incoming, out)
                target.chmod(0o755 if mode & 0o111 else 0o644)
        adapter = extracted / "extension/adapter/codelldb"
        if not adapter.is_file() or not (extracted / "extension/lldb/lib/liblldb.so").is_file():
            raise RuntimeError("CodeLLDB archive lacks native runtime")
        adapter.chmod(0o755)
        # The upstream VSIX omits the license files. Preserve the three pinned
        # top-level licenses explicitly alongside the complete upstream payload.
        notices = extracted / "NOTICES"
        notices.mkdir()
        for name, (url, expected) in NOTICES.items():
            with urllib.request.urlopen(url, timeout=30) as response:
                content = response.read(128 * 1024 + 1)
            if len(content) > 128 * 1024 or hashlib.sha256(content).hexdigest() != expected:
                raise RuntimeError("Upstream CodeLLDB license integrity verification failed")
            (notices / name).write_bytes(content)
        files = {str(path.relative_to(extracted)): digest(path) for path in sorted(extracted.rglob("*")) if path.is_file()}
        (extracted / "codisseu-install.json").write_text(json.dumps({"version": VERSION, "url": URL, "archive_sha256": SHA256, "files": files}, indent=2) + "\n")
        os.rename(extracted, destination)
    verify(destination)


if __name__ == "__main__":
    main()
