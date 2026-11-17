#!/usr/bin/env python3
"""Run native SQL integration against isolated, disposable loopback containers.

Uses already-installed mysql:8.4 and postgres:17-alpine images, never production
profiles, mounted host data, privileged Docker, or automatic image downloads.
Server data uses bounded tmpfs to isolate transaction tests from host disk pressure;
this suite does not establish storage durability or production query performance.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "artifacts" / "database-metadata-fixtures"
PASSWORD = "codisseu_fixture"


def command(args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs)


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    token = uuid.uuid4().hex[:10]
    created = []
    manifest = {"servers": [], "result": "in_progress", "sources": {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in [ROOT / "crates/codisseu-db/src/remote_sql.rs", ROOT / "crates/codisseu-db/tests/remote_metadata.rs"]
    }}
    try:
        ports = {}
        for engine, image, target_port, envs, health in [
            ("PG", "postgres:17-alpine", 5432,
             ["POSTGRES_DB=codisseu", f"POSTGRES_PASSWORD={PASSWORD}"],
             ["pg_isready", "-h", "127.0.0.1", "-U", "postgres"]),
            ("MY", "mysql:8.4", 3306,
             ["MYSQL_DATABASE=codisseu", f"MYSQL_ROOT_PASSWORD={PASSWORD}"],
             ["mysqladmin", "--protocol=tcp", "-h127.0.0.1", "-uroot", f"-p{PASSWORD}", "ping"]),
        ]:
            name = f"codisseu-metadata-{engine.lower()}-{token}"
            args = ["docker", "run", "--detach", "--rm", "--pull", "never", "--name", name,
                    "--memory", "1g", "--cpus", "2", "--publish", f"127.0.0.1::{target_port}"]
            data_dir = "/var/lib/postgresql/data" if engine == "PG" else "/var/lib/mysql"
            args.extend(["--tmpfs", f"{data_dir}:rw,size=512m"])
            for env in envs:
                args.extend(["--env", env])
            container = command(args + [image]).stdout.strip()
            created.append(name)
            image_id = command(["docker", "inspect", "--format", "{{.Image}}", name]).stdout.strip()
            deadline = time.monotonic() + 90
            while True:
                health_result = subprocess.run(["docker", "exec", name] + health,
                                              capture_output=True, timeout=5)
                if health_result.returncode == 0:
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"{engine} fixture did not become ready")
                time.sleep(0.3)
            address = command(["docker", "port", name, f"{target_port}/tcp"]).stdout.strip()
            assert address.startswith("127.0.0.1:"), address
            ports[f"CODISSEU_SQL_{engine}_PORT"] = address.split(":")[-1]
            manifest["servers"].append({"engine": engine, "image": image, "image_id": image_id,
                                        "container": container, "address": address,
                                        "data_storage": "bounded tmpfs (512 MiB); no durability claim"})
        with (OUTPUT / "tests.log").open("w") as log:
            result = subprocess.run(["cargo", "test", "-p", "codisseu-db", "--test", "remote_metadata",
                                     "--locked", "--jobs", "2", "--", "--ignored", "--nocapture"],
                                    cwd=ROOT, env={**os.environ, **ports}, stdout=log,
                                    stderr=subprocess.STDOUT)
        manifest["exit_code"] = result.returncode
        manifest["result"] = "passed" if result.returncode == 0 else "failed"
        print((OUTPUT / "tests.log").read_text())
        return result.returncode
    finally:
        removed, cleanup_errors = [], []
        for name in reversed(created):
            try:
                cleanup = subprocess.run(["docker", "rm", "--force", name], capture_output=True, timeout=30)
                if cleanup.returncode == 0:
                    removed.append(name)
                else:
                    cleanup_errors.append(name)
            except subprocess.TimeoutExpired:
                cleanup_errors.append(name)
        manifest["containers_removed"] = removed
        manifest["cleanup_errors"] = cleanup_errors
        (OUTPUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        if cleanup_errors:
            raise RuntimeError(f"Fixture cleanup failed: {cleanup_errors}")


if __name__ == "__main__":
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,default=OUTPUT)
    OUTPUT=parser.parse_args().output.resolve()
    raise SystemExit(main())
