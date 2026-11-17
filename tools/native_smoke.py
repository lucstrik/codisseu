#!/usr/bin/env python3
"""Run the real native window twice on an isolated fixture; no OS input injection."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    options = parser.parse_args()
    out = (options.output or ROOT / "artifacts" / time.strftime("native-%Y%m%d-%H%M%S")).resolve()
    binary = ROOT / "target/debug/codisseu"
    if not binary.is_file():
        parser.error("Compile primeiro: cargo build --locked -p storm")
    out.mkdir(parents=True, exist_ok=False)
    fixture = out / "workspace"
    shutil.copytree(ROOT / "fixtures/fullstack", fixture)
    session = out / "session.json"
    files = [fixture / "src/index.php", fixture / "src/app.ts"]
    before = {str(p.relative_to(fixture)): hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture.rglob("*") if p.is_file()}
    results = []
    runs = [
        ("native", ["--no-restore", "--workspace", str(fixture), "--open", str(files[0]), "--open", str(files[1])]),
        ("restored", []),
    ]
    for name, extra in runs:
        screenshot = out / f"{name}.png"
        command = [str(binary), "--session", str(session), *extra, "--capture", str(screenshot), "--close-after-capture"]
        started = time.monotonic()
        with (out / f"{name}.log").open("w") as log:
            completed = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=60, check=False)
        assert completed.returncode == 0, f"{name}: código {completed.returncode}; veja log"
        data = screenshot.read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n", "captura PNG ausente/inválida"
        size = struct.unpack(">II", data[16:24])
        assert min(size) >= 500, f"área cliente inesperada: {size}"
        state = json.loads(session.read_text())
        assert state["tabs"] == [str(p) for p in files]
        assert state["active"] == str(files[1]) and state["workspace"] == str(fixture)
        results.append({"scenario": name, "command": command, "exit_code": completed.returncode, "wall_seconds": round(time.monotonic()-started, 3), "screenshot": screenshot.name, "size_px": size, "capture_metadata": json.loads(screenshot.with_suffix(".json").read_text()), "session_assertions": "passed"})
    after = {str(p.relative_to(fixture)): hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture.rglob("*") if p.is_file()}
    assert before == after, "abrir/restaurar não deve modificar arquivos"
    manifest = {"schema_version": 1, "kind": "native window lifecycle; real display and framebuffer; no OS keyboard/mouse simulation", "tests": results, "fixture_sha256": before, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()}
    (out / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"2 execuções nativas aprovadas; capturas e manifesto: {out}")


if __name__ == "__main__":
    run()
