#!/usr/bin/env python3
"""Collect the Linux application's locked Cargo and embedded-asset notices.

No network, personal profiles, runtimes, or proprietary reference captures.
The manifest records the selected dependency closure, not a legal certification.
"""
import argparse
import fnmatch
import hashlib
import json
import re
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PATTERNS = ("LICENSE*", "LICENCE*", "COPYING*", "COPYRIGHT*", "NOTICE*",
            "OFL*", "UFL*", "AUTHORS*", "*license*.txt")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def collect(destination):
    destination = destination.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
         "--filter-platform", "x86_64-unknown-linux-gnu"], cwd=ROOT,
        check=True, capture_output=True, text=True, timeout=120).stdout)
    packages = {p["id"]: p for p in metadata["packages"]}
    # cargo metadata resolves the whole workspace and also lists disabled optional
    # edges. cargo tree selects the application's actual normal/build features.
    tree = subprocess.run(
        ["cargo", "tree", "-p", "codisseu", "--locked", "--offline", "--target",
         "x86_64-unknown-linux-gnu", "--edges", "normal,build", "--prefix", "none",
         "--format", "{p}"], cwd=ROOT, check=True, capture_output=True,
        text=True, timeout=120).stdout
    selected = set(re.findall(r"^(\S+) v(\S+)", tree, re.MULTILINE))
    reachable = {p["id"] for p in metadata["packages"]
                 if (p["name"], p["version"]) in selected}
    checksums = {(p["name"], p["version"]): p.get("checksum")
                 for p in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]}
    records, missing = [], []
    for identifier in sorted(reachable, key=lambda i: (packages[i]["name"], packages[i]["version"])):
        package = packages[identifier]
        first_party = package["source"] is None and package["name"].startswith("codisseu")
        record = {k: package.get(k) for k in ("name", "version", "license", "repository", "source")}
        record.update(first_party=first_party,
                      crate_sha256=checksums.get((package["name"], package["version"])))
        records.append(record)
        if first_party:
            continue
        source = Path(package["manifest_path"]).parent.resolve()
        candidates = [p for p in source.rglob("*") if p.is_file() and not p.is_symlink()
                      and any(fnmatch.fnmatch(p.name, pattern) for pattern in PATTERNS)]
        if package["name"] == "epaint_default_fonts":
            candidates.extend(p for p in (source / "fonts").glob("*.txt") if p.is_file())
        if package.get("license_file"):
            declared = (source / package["license_file"]).resolve()
            if declared.is_relative_to(source) and declared.is_file():
                candidates.append(declared)
        # Exact package notices recovered in earlier audits and release tags.
        if not candidates:
            label = package["name"] + "-" + package["version"]
            for base in (ROOT / "reference/public").rglob(label):
                if base.is_dir():
                    candidates.extend(p for p in base.rglob("*")
                                      if p.is_file() and not p.is_symlink())
            if package["name"] == "tree-sitter-typescript":
                candidates.append(ROOT / "reference/public/tree-sitter/tree-sitter-typescript-MIT-LICENSE.txt")
        record["notices"] = []
        for path in sorted(set(candidates)):
            relative = path.relative_to(source) if path.is_relative_to(source) else Path(path.parent.name) / path.name
            output = destination / "cargo" / (package["name"] + "-" + package["version"]) / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, output)
            record["notices"].append({"file": output.relative_to(destination).as_posix(), "sha256": digest(output)})
        if not record["notices"]:
            missing.append(package["name"] + " " + package["version"])
    # These directories contain public upstream notices/provenance. Font binary
    # files are already embedded in the executable, so do not duplicate them.
    for name in ("fonts", "ide-icons", "vscode-codicons", "visual-v3", "visual-v4",
                 "intellij-oss", "icons", "tree-sitter", "unicode-segmentation"):
        source = ROOT / "reference/public" / name
        if source.is_dir():
            shutil.copytree(source, destination / "assets" / name,
                            ignore=shutil.ignore_patterns("*.ttf", "*.ttc", "*.png", "*.svg"))
    for name in ("asset-sources.json", "sources.json"):
        shutil.copy2(ROOT / "reference/public" / name, destination / "assets" / name)
    shutil.copy2(ROOT / "reference/public/release-notices/sources.json", destination / "recovered-notice-sources.json")
    shutil.copytree(ROOT / "tools/portable_python_notices", destination / "syntax/python")
    shutil.copytree(ROOT / "tools/portable_rust_notices", destination / "syntax/rust")
    shutil.copy2(ROOT / "crates/codisseu-db/certificates/README.md", destination / "AWS-certificates.txt")
    shutil.copy2(ROOT / "crates/codisseu-db/certificates/source.json", destination / "AWS-certificates-source.json")
    shutil.copytree(ROOT / "vendor/egui-winit", destination / "vendor/egui-winit",
                    ignore=shutil.ignore_patterns("src", "Cargo.lock", "target"))
    if (ROOT / "LICENSE").is_file():
        shutil.copy2(ROOT / "LICENSE", destination / "Codisseu-LICENSE.txt")
    manifest = {"schema_version": 1, "target": "x86_64-unknown-linux-gnu",
                "scope": "Application normal/build dependency closure; excludes optional external tools and private runtime bundle",
                "cargo_lock_sha256": digest(ROOT / "Cargo.lock"), "packages": records,
                "missing_notices": missing, "complete_notice_collection": not missing}
    (destination / "cargo-notices.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"destination": str(destination), "packages": len(records), "missing": missing}))
    return 1 if missing else 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    raise SystemExit(collect(parser.parse_args().output))
