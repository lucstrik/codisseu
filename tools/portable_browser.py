"""Prepare the pinned, native CEF process without touching user profiles or networking."""
import hashlib, json, os, re, shutil, subprocess, tomllib
from pathlib import Path


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def command(arguments, timeout=120):
    return subprocess.run([str(a) for a in arguments], check=True, capture_output=True, text=True, timeout=timeout).stdout


def rust_notices(repository, destination):
    """Include the helper's independent Cargo closure, including build dependencies."""
    crate = repository / "apps/codisseu-browser-engine"
    metadata = json.loads(command(["cargo", "metadata", "--manifest-path", crate / "Cargo.toml", "--format-version", "1", "--locked", "--offline", "--filter-platform", "x86_64-unknown-linux-gnu"]))
    nodes = {node["id"]:node for node in metadata["resolve"]["nodes"]}
    pending = [metadata["resolve"]["root"]]; reachable = set()
    while pending:
        package = pending.pop()
        if package in reachable: continue
        reachable.add(package)
        for dependency in nodes[package]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]): pending.append(dependency["pkg"])
    checksums = {(p["name"],p["version"]):p.get("checksum") for p in tomllib.loads((crate / "Cargo.lock").read_text())["package"]}
    records = []
    for package in sorted(metadata["packages"], key=lambda p:(p["name"],p["version"])):
        if package["id"] not in reachable: continue
        record = {key:package.get(key) for key in ["name", "version", "license", "repository", "source"]}
        record["crate_sha256"] = checksums.get((package["name"],package["version"]))
        if package["source"] is None:
            record["first_party"] = True; records.append(record); continue
        root = Path(package["manifest_path"]).parent.resolve()
        candidates = {p for pattern in ["LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "COPYRIGHT*"] for p in root.glob(pattern) if p.is_file()}
        if package.get("license_file"):
            declared = (root / package["license_file"]).resolve()
            if declared.is_relative_to(root) and declared.is_file(): candidates.add(declared)
        target = destination / "cargo" / (package["name"] + "-" + package["version"])
        target.mkdir(parents=True)
        preserved = []
        for original in sorted(candidates):
            output = target / original.name
            shutil.copy2(original, output)
            preserved.append({"file":str(output.relative_to(destination)), "sha256":digest(output)})
        if not preserved and str(package.get("repository", "")).rstrip("/") in ["https://github.com/tauri-apps/cef-rs", "https://github.com/tauri-apps/cef-rs.git"]:
            # Workspace-level licenses are absent from these published crates.
            # Exact upstream tag originals were explicitly audited/prepared.
            for original in sorted((repository / "tools/browser_notices").glob("cef-rs-LICENSE-*.txt")):
                output = target / original.name; shutil.copy2(original, output)
                preserved.append({"file":str(output.relative_to(destination)), "sha256":digest(output), "origin":"Exact cef-rs release tag; provenance.json preserved"})
        if not preserved: raise RuntimeError("Prepare missing helper dependency notices explicitly: " + package["name"] + " " + package["version"])
        record["notices"] = preserved; records.append(record)
    shutil.copy2(crate / "Cargo.lock", destination / "helper-Cargo.lock")
    shutil.copy2(crate / "Cargo.toml", destination / "helper-Cargo.toml")
    result = {"scope":"Linux x86_64 helper independent Cargo dependency closure; includes build tools, excludes dev dependencies", "lock_sha256":digest(crate / "Cargo.lock"), "packages":records, "complete":True}
    (destination / "helper-cargo-notices.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def refresh_engine(repository, bundle):
    """Update only an audited helper delta in an unpublished, verified tree."""
    install = repository / ".tools/browser"
    engine = json.loads((install / "engine-install.json").read_text())
    build = json.loads((bundle / "BUILD.json").read_text())
    previous = build["browser"]["engine"]
    if previous["cargo_lock_sha256"] != engine["cargo_lock_sha256"]:
        raise RuntimeError("Helper dependencies changed; rebuild the complete bundle")
    for relative, expected in engine["source_closure"].items():
        if digest(repository / relative) != expected: raise RuntimeError("Stale helper source receipt")
    sdk = bundle / "share/codisseu/notices/browser/codisseu-install.json"
    if sdk.read_bytes() != (install / "codisseu-install.json").read_bytes():
        raise RuntimeError("SDK changed; rebuild the complete bundle")
    raw = bundle / "share/codisseu/runtime/browser/bin/codisseu-browser-engine.bin"
    current = install / "bin/codisseu-browser-engine"
    def needed(path):
        return sorted(re.findall(r"Shared library: \[([^\]]+)\]", command(["readelf", "-d", path])))
    if needed(raw) != needed(current): raise RuntimeError("Helper native ABI closure changed")
    temporary = raw.with_name(".codisseu-browser-engine.new")
    shutil.copy2(current, temporary); os.replace(temporary, raw)
    audit = bundle / "share/codisseu/notices/browser"
    shutil.copy2(install / "engine-install.json", audit / "engine-install.json")
    source = json.loads((audit / "sources.json").read_text()); source["engine"] = engine
    (audit / "sources.json").write_text(json.dumps(source, indent=2) + "\n")
    build["browser"]["engine"] = engine
    (bundle / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n")
    rows = []
    for path in sorted(bundle.rglob("*")):
        if path.is_file() and path != bundle / "MANIFEST.sha256": rows.append(digest(path) + "  " + path.relative_to(bundle).as_posix())
    (bundle / "MANIFEST.sha256").write_text("\n".join(rows) + "\n")
    return {"previous_helper_sha256":previous["engine_sha256"], "helper_sha256":engine["engine_sha256"], "main_unchanged":build["binary_sha256"], "sdk_unchanged":True, "dependencies_unchanged":True, "manifest_files":len(rows)}


def prepare(repository, resources, bundle, notices, native_copy):
    source = repository / ".tools/browser"
    command(["python3", repository / "tools/install_browser_runtime.py", "--verify"])
    engine = json.loads((source / "engine-install.json").read_text())
    if engine["profile"] != "release" or digest(source / "bin/codisseu-browser-engine") != engine["engine_sha256"]:
        raise RuntimeError("Prepare and validate the final release CEF helper explicitly")
    if digest(repository / "apps/codisseu-browser-engine/Cargo.lock") != engine["cargo_lock_sha256"]:
        raise RuntimeError("CEF helper Cargo closure changed since the audited release")
    if not engine.get("source_closure") or not engine.get("source_unchanged_during_build"):
        raise RuntimeError("Rebuild the final helper with a verified source closure")
    for relative, expected in engine["source_closure"].items():
        path = (repository / relative).resolve(strict=True)
        if not path.is_relative_to(repository.resolve()) or digest(path) != expected:
            raise RuntimeError("CEF helper source changed after its final build: " + relative)
    destination = resources / "runtime/browser"
    (destination / "bin").mkdir(parents=True)
    shutil.copytree(source / "cef", destination / "cef", symlinks=True)
    for path in (destination / "cef").rglob("*"):
        if path.is_symlink() and (os.path.isabs(os.readlink(path)) or not path.resolve().is_relative_to((destination / "cef").resolve())):
            raise RuntimeError("CEF runtime contains an external link")
    raw = destination / "bin/codisseu-browser-engine.bin"
    shutil.copy2(source / "bin/codisseu-browser-engine", raw)
    # libcef stays private to the separate process. Its 1.5GB image is never
    # duplicated into the IDE's global native directory or linked by the GUI.
    dependencies = {}
    for binary in [source / "bin/codisseu-browser-engine", *sorted((source / "cef").glob("*.so*"))]:
        with binary.open("rb") as stream: elf = stream.read(4) == b"\x7fELF"
        if not elf: continue
        for line in command(["ldd", binary]).splitlines():
            match = re.search(r"=> (/\S+) \(", line) or re.match(r"\s*(/\S+) \(", line)
            if not match: continue
            link = Path(match[1]); dependency = link.resolve(strict=True)
            if dependency.is_relative_to((source / "cef").resolve()): continue
            previous = dependencies.get(link.name)
            if previous and digest(previous) != digest(dependency): raise RuntimeError("Conflicting CEF native ABI")
            dependencies[link.name] = dependency
    # NSS loads its crypto/root modules with dlopen; they do not appear in the
    # libcef ldd graph. Preserve the modules from the same installed package.
    if "libnss3.so" in dependencies:
        for filename in command(["dpkg-query", "-L", "libnss3"]).splitlines():
            path = Path(filename)
            if path.name.endswith(".so") and path.is_file():
                dependencies[path.name] = path.resolve(strict=True)
    # winit/glutin resolve these client/display dispatch libraries with dlopen.
    # Preserve their SONAME aliases, not only versioned backing filenames.
    for name in ["libXcursor.so.1", "libXrandr.so.2", "libXi.so.6", "libXfixes.so.3", "libXinerama.so.1", "libX11-xcb.so.1", "libxkbcommon-x11.so.0", "libxkbcommon.so.0", "libEGL.so.1", "libGLX.so.0", "libGL.so.1", "libOpenGL.so.0", "libwayland-client.so.0", "libwayland-cursor.so.0", "libwayland-egl.so.1"]:
        path = Path("/usr/lib/x86_64-linux-gnu") / name
        if not path.is_file(): raise RuntimeError("Prepare required native desktop library: " + name)
        dependencies[name] = path.resolve(strict=True)
    for name, original in sorted(dependencies.items()): native_copy(original, bundle / "lib" / name)
    fonts = destination / "fonts"; fonts.mkdir()
    font_records = []
    for name, license_name in [("NotoSans-Regular.ttf", "NotoSans-copyright.txt"), ("DejaVuSansMono.ttf", "DejaVuSansMono-copyright.txt"), ("NotoColorEmoji.ttf", "NotoColorEmoji-copyright.txt"), ("NotoSansCJK-Regular.ttc", "NotoSansCJK-OFL.txt"), ("NotoSansSymbols2-Regular.ttf", "NotoSansSymbols2-OFL.txt")]:
        original = repository / "reference/public/fonts" / name
        shutil.copy2(original, fonts / name)
        font_records.append({"file":name, "sha256":digest(original), "notice":license_name})
    fontconfig = destination / "fontconfig"; fontconfig.mkdir()
    (fontconfig / "fonts.conf").write_text('''<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <dir prefix="relative">../fonts</dir>
  <cachedir prefix="xdg">fontconfig</cachedir>
</fontconfig>
''')
    loader = next((bundle / "lib").glob("ld-linux-*")).name
    wrapper = destination / "bin/codisseu-browser-engine"
    wrapper.write_text('''#!/bin/sh
set -eu
codisseu_browser=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
codisseu_resources=$(CDPATH= cd -- "$codisseu_browser/../.." && pwd)
codisseu_bundle=$(CDPATH= cd -- "$codisseu_resources/../.." && pwd)
export CODISSEU_BROWSER_EXECUTABLE="$codisseu_browser/bin/codisseu-browser-engine"
export FONTCONFIG_FILE="$codisseu_browser/fontconfig/fonts.conf"
export FONTCONFIG_PATH="$codisseu_browser/fontconfig"
''' + f'exec "$codisseu_bundle/lib/{loader}" --library-path "$codisseu_browser/cef:$codisseu_bundle/lib" "$codisseu_browser/bin/codisseu-browser-engine.bin" "$@"\n')
    wrapper.chmod(0o755)
    audit = notices / "browser"; audit.mkdir()
    (audit / "fonts").mkdir()
    for font in font_records:
        source_license = repository / "reference/public/fonts" / font["notice"]
        shutil.copy2(source_license, audit / "fonts" / font["notice"])
        font["notice_sha256"] = digest(source_license)
    shutil.copytree(repository / "tools/browser_notices", audit / "cef-rs")
    for name in ["LICENSE.txt", "CREDITS.html", "README.txt"]: shutil.copy2(source / "cef" / name, audit / ("CEF-" + name))
    for name in ["engine-install.json", "codisseu-install.json"]: shutil.copy2(source / name, audit / name)
    shutil.copy2(repository / "tools/browser-engine.lock.json", audit / "browser-engine.lock.json")
    shutil.copy2(source / "cef/devtools-assets/embedding.json", audit / "devtools-embedding.json")
    shutil.copy2(source / "cef/devtools-assets/embedding.diff", audit / "devtools-embedding.diff")
    shutil.copy2(repository / "apps/codisseu-browser-engine/assets/bootstrap.js", audit / "own-devtools-bootstrap.js")
    cargo = rust_notices(repository, audit)
    pins = json.loads((repository / "tools/browser-engine.lock.json").read_text())
    info = {"cef":pins["cef_version"], "chromium":pins["chromium_version"], "protocol":2, "archive":pins, "engine":engine, "runtime_file_count":len(json.loads((source / "codisseu-install.json").read_text())["files"]), "devtools":json.loads((source / "cef/devtools-assets/manifest.json").read_text()), "embedding":json.loads((source / "cef/devtools-assets/embedding.json").read_text()), "fonts":font_records, "helper_cargo_packages":len(cargo["packages"]), "sandbox":"Chromium renderer user/PID namespaces and seccomp required; no --no-sandbox fallback. Native browser/network brokers retain upstream privileges.", "loopback":"Only the explicit DevTools frontend origin may receive LOOPBACK_NETWORK permission, restricted by exact nonce endpoint CSP and authenticated gateway.", "subprocesses":"Private wrapper set as browser_subprocess_path and OnBeforeChildProcessLaunch program; wrapper re-enters private glibc loader for every role.", "cef_linked_into_ide":False, "downloads_required":False, "native_dependencies":{name:digest(path) for name,path in sorted(dependencies.items())}}
    (audit / "sources.json").write_text(json.dumps(info, indent=2) + "\n")
    roles = {"browser-engine":"runtime/browser/bin/codisseu-browser-engine", "browser-runtime":"runtime/browser/cef/libcef.so"}
    return roles, info
