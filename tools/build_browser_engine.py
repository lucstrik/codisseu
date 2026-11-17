#!/usr/bin/env python3
"""Build the isolated, pinned Rust/CEF process and install it privately."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debug", action="store_true")
    parser.add_argument("--online", action="store_true", help="Allow explicit dependency preparation")
    args = parser.parse_args()
    base = ROOT / ".tools/browser"
    subprocess.run([sys.executable, str(ROOT / "tools/install_browser_runtime.py"), "--verify"], check=True)
    crate = ROOT / "apps/codisseu-browser-engine"
    closure = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", crate / "Cargo.toml", crate / "Cargo.lock", crate / "build.rs", *(crate / "src").rglob("*.rs"), *(crate / "assets").rglob("*.js")]
    for name in ["codisseu-browser", "codisseu-runtime"]:
        module = ROOT / "crates" / name
        closure.extend([module / "Cargo.toml", *(module / "src").rglob("*.rs")])
        if (module / "build.rs").is_file(): closure.append(module / "build.rs")
    closure_before = {str(path.relative_to(ROOT)): digest(path) for path in sorted(closure)}
    command = ["cargo", "build", "--manifest-path", str(crate / "Cargo.toml"), "--locked", "--jobs", "2"]
    if not args.online:
        command.append("--offline")
    if not args.debug:
        command.append("--release")
    environment = dict(os.environ, CEF_PATH=str(base / "sdk-flat"), CARGO_TARGET_DIR=str(crate / "target"))
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    closure_after = {str(path.relative_to(ROOT)): digest(path) for path in sorted(closure)}
    if closure_before != closure_after:
        raise RuntimeError("Helper source closure changed during build; existing installed helper preserved")
    source = crate / "target" / ("debug" if args.debug else "release") / "codisseu-browser-engine"
    destination = base / "bin/codisseu-browser-engine"
    if destination.is_symlink():
        raise RuntimeError("Refusing a symlink engine destination")
    temporary = destination.with_name(".codisseu-browser-engine.new")
    if temporary.exists():
        raise RuntimeError("Previous engine staging file preserved")
    shutil.copy2(source, temporary)
    os.replace(temporary, destination)
    notices = base / "notices"
    notices.mkdir(exist_ok=True)
    for path in (ROOT / "tools/browser_notices").iterdir():
        shutil.copy2(path, notices / path.name)
    compiler = subprocess.run(["rustc", "-Vv"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    record = {
        "schema_version": 1,
        "engine_sha256": digest(destination),
        "engine_bytes": destination.stat().st_size,
        "cargo_lock_sha256": digest(crate / "Cargo.lock"),
        "cargo_toml_sha256": digest(crate / "Cargo.toml"),
        "pins_sha256": digest(ROOT / "tools/browser-engine.lock.json"),
        "profile": "debug" if args.debug else "release",
        "cef_dynamic_link": "$ORIGIN/../cef",
        "sandbox": "Required; no unsandboxed fallback",
        "source_closure": closure_after,
        "source_unchanged_during_build": True,
        "rustc_vV": compiler,
        "sources": {str(p.relative_to(ROOT)): digest(p) for p in sorted([crate / "build.rs", *(crate / "src").rglob("*.rs"), *(crate / "assets").rglob("*.js")])},
        "notices": {p.name: digest(p) for p in sorted(notices.iterdir()) if p.is_file()},
    }
    record_path = base / "engine-install.json"
    record_temporary = base / ".engine-install.json.new"
    if record_temporary.exists():
        raise RuntimeError("Previous engine receipt staging file preserved")
    record_temporary.write_text(json.dumps(record, indent=2) + "\n")
    os.replace(record_temporary, record_path)
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
