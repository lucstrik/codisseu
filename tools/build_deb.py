#!/usr/bin/env python3
"""Package a verified offline Linux bundle; never download or install tools.

The existing bundle is read-only input. No maintainer script is used: installing
or removing this package cannot run Composer/npm or touch personal profiles.
"""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def digest(path, algorithm="sha256"):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def verify_bundle(bundle):
    """Reject missing, modified, unlisted or escaping installation resources."""
    bundle = Path(bundle).resolve(strict=True)
    if not bundle.is_dir() or bundle.is_symlink():
        raise ValueError("Bundle must be a directory")
    manifest = bundle / "MANIFEST.sha256"
    records = {}
    for row in manifest.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", row)
        if not match:
            raise ValueError("Invalid SHA256 manifest row")
        expected, relative = match.groups()
        path = PurePosixPath(relative)
        if path.is_absolute() or any(part in (".", "..") for part in path.parts) or "\\" in relative or str(path) != relative:
            raise ValueError("Unsafe bundle manifest path")
        if relative == "MANIFEST.sha256" or relative in records:
            raise ValueError("Duplicate or self-referential manifest path")
        target = bundle / relative
        if not target.resolve(strict=True).is_relative_to(bundle) or not target.is_file():
            raise ValueError("Bundle resource escapes its installation")
        if digest(target) != expected:
            raise ValueError("Bundle resource checksum mismatch: " + relative)
        records[relative] = expected
    actual = set()
    for path in bundle.rglob("*"):
        if path.is_symlink():
            if os.path.isabs(os.readlink(path)) or not path.resolve(strict=True).is_relative_to(bundle):
                raise ValueError("Bundle contains an external symlink")
        if path.is_file() and path != manifest:
            actual.add(path.relative_to(bundle).as_posix())
        elif not path.is_dir() and not path.is_file():
            raise ValueError("Bundle contains a special file")
    if actual != set(records):
        raise ValueError("Bundle contains unlisted or missing resources")
    required = ["bin/codisseu", "libexec/codisseu-bin", "BUILD.json", "BUNDLE-ID", "share/codisseu/runtime.json", "share/codisseu/icons/256.png"]
    if not all(relative in records for relative in required):
        raise ValueError("Bundle lacks required installation resources")
    if not os.access(bundle / "bin/codisseu", os.X_OK):
        raise ValueError("Bundle launcher is not executable")
    build = json.loads((bundle / "BUILD.json").read_text())
    if build.get("platform") != "linux-x86_64" or build.get("downloads_on_install_or_first_use") is not False:
        raise ValueError("Bundle must be offline Linux x86_64")
    if build.get("binary_sha256") != records["libexec/codisseu-bin"]:
        raise ValueError("Bundle binary metadata mismatch")
    return {"files": len(records), "manifest_sha256": digest(manifest), "binary_sha256": build["binary_sha256"]}


def native_dependencies(binary):
    """Ask dpkg to derive the ABI floor from the actual release executable."""
    with tempfile.TemporaryDirectory(prefix="codisseu-shlibdeps-") as temporary:
        root = Path(temporary)
        (root / "debian").mkdir()
        (root / "debian/control").write_text(
            "Source: codisseu\nSection: devel\nPriority: optional\n"
            "Maintainer: Codisseu <18267941+lucstrik@users.noreply.github.com>\n\n"
            "Package: codisseu\nArchitecture: amd64\nDescription: Codisseu\n"
        )
        result = subprocess.run(["dpkg-shlibdeps", "-O", "-e" + str(Path(binary).resolve(strict=True))], cwd=root, check=True, capture_output=True, text=True)
    dependencies = next(row.split("=", 1)[1] for row in result.stdout.splitlines() if row.startswith("shlibs:Depends="))
    # winit/wgpu load these display/graphics dispatch libraries dynamically,
    # so they are deliberately additional to ELF NEEDED/symbol dependencies.
    desktop = "libegl1, libgl1, libopengl0, libvulkan1, libx11-6, libx11-xcb1, libxcursor1, libxrandr2, libxi6, libxfixes3, libxinerama1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, ca-certificates"
    return dependencies + ", " + desktop


def prepare_core_bundle(binary, destination, notices):
    """Prepare the application using system libraries and optional runtimes."""
    binary = Path(binary).resolve(strict=True)
    notices = Path(notices).resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK) or not notices.is_dir() or not any(notices.iterdir()):
        raise ValueError("Core packaging needs an executable and prepared redistribution notices")
    with binary.open("rb") as stream:
        header = stream.read(20)
    if header[:4] != b"\x7fELF" or header[4] != 2 or header[18:20] != b"\x3e\x00":
        raise ValueError("Core executable must be Linux x86_64 ELF")
    destination = Path(destination)
    (destination / "bin").mkdir(parents=True)
    (destination / "libexec").mkdir()
    resources = destination / "share/codisseu"
    (resources / "icons").mkdir(parents=True)
    shutil.copy2(binary, destination / "libexec/codisseu-bin")
    launcher = destination / "bin/codisseu"
    launcher.write_text('''#!/bin/sh
set -eu
codisseu_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export CODISSEU_RESOURCE_DIR="$codisseu_root/share/codisseu"
exec "$codisseu_root/libexec/codisseu-bin" "$@"
''')
    launcher.chmod(0o755)
    (resources / "runtime.json").write_text(json.dumps({"schema_version": 1, "tools": {}, "providers": []}, indent=2) + "\n")
    provenance = json.loads((ROOT / "assets/icons/provenance.json").read_text())
    records = {item["path"]: item["sha256"] for item in provenance["files"]}
    for size in provenance["sizes"]:
        relative = f"assets/icons/io.codisseu.Codisseu-{size}.png"
        source = ROOT / relative
        if digest(source) != records[relative]:
            raise ValueError("Application icon provenance mismatch")
        shutil.copy2(source, resources / f"icons/{size}.png")
    shutil.copy2(ROOT / "assets/icons/provenance.json", resources / "icons/provenance.json")
    shutil.copytree(notices, resources / "notices", symlinks=True)
    build = {
        "platform": "linux-x86_64", "package_variant": "core", "binary_sha256": digest(binary),
        "downloads_on_install_or_first_use": False, "bundled_language_runtimes": False,
        "system_dependencies": native_dependencies(binary),
        "runtime_configuration": "Optional external tools are configured explicitly; no language runtime is installed or replaced by this package.",
    }
    (destination / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n")
    (destination / "BUNDLE-ID").write_text("codisseu-core-0.1.0-linux-x86_64-" + build["binary_sha256"][:12] + "\n")
    rows = [digest(path) + "  " + path.relative_to(destination).as_posix() for path in sorted(destination.rglob("*")) if path.is_file()]
    (destination / "MANIFEST.sha256").write_text("\n".join(rows) + "\n")
    return build


def build_package(bundle, output, version, maintainer, homepage):
    bundle = Path(bundle).resolve(strict=True)
    output = Path(output).absolute()
    if any("\n" in value or "\r" in value for value in (version, maintainer, homepage)):
        raise ValueError("Debian control values must fit a single line")
    subprocess.run(["dpkg", "--validate-version", version], check=True, capture_output=True, text=True)
    if not re.fullmatch(r"[^<>\n]+ <[^<>\s]+@[^<>\s]+>", maintainer):
        raise ValueError("Maintainer must include a contact address")
    if not homepage.startswith("https://github.com/"):
        raise ValueError("Homepage must be the project's HTTPS GitHub URL")
    report = verify_bundle(bundle)
    build = json.loads((bundle / "BUILD.json").read_text())
    dependencies = build.get("system_dependencies", "libc6 (>= 2.34), libstdc++6 (>= 12), libgcc-s1, ca-certificates")
    if not isinstance(dependencies, str) or "\n" in dependencies or "\r" in dependencies:
        raise ValueError("Invalid package system dependencies")
    core = build.get("package_variant") == "core"
    output.mkdir(parents=True, exist_ok=True)
    # GitHub sanitizes '~' in release asset names. Keep Debian version ordering
    # in control while choosing a stable downloadable external filename.
    filename_version = version.replace("~", "-")
    package = output / f"codisseu_{filename_version}_amd64.deb"
    if package.exists() or package.with_suffix(".deb.sha256").exists():
        raise FileExistsError("Output exists; prior release artifacts preserved")
    with tempfile.TemporaryDirectory(prefix="codisseu-deb-", dir=output) as temporary:
        root = Path(temporary)
        (root / "opt").mkdir()
        # Hard links avoid another multi-gigabyte runtime copy. We never chmod
        # or modify the input tree, and dpkg-deb only reads the staged content.
        def stage_file(source, destination):
            try:
                os.link(source, destination)
            except OSError as error:
                if error.errno != errno.EXDEV:
                    raise
                shutil.copy2(source, destination)
            return destination
        shutil.copytree(bundle, root / "opt/codisseu", symlinks=True, copy_function=stage_file)
        binaries = root / "usr/bin"
        binaries.mkdir(parents=True)
        launcher = binaries / "codisseu"
        launcher.write_text('#!/bin/sh\nexec /opt/codisseu/bin/codisseu --cli "$@"\n')
        launcher.chmod(0o755)
        applications = root / "usr/share/applications"
        applications.mkdir(parents=True)
        (applications / "io.codisseu.Codisseu.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Codisseu\n"
            "Comment=PHP, JavaScript, TypeScript e bancos no mesmo lugar\n"
            "Exec=/opt/codisseu/bin/codisseu %F\nIcon=io.codisseu.Codisseu\n"
            "Terminal=false\nCategories=Development;IDE;\nStartupWMClass=io.codisseu.Codisseu\n"
            "Keywords=PHP;JavaScript;TypeScript;SQL;Rust;Python;\n"
        )
        for icon in sorted((bundle / "share/codisseu/icons").glob("*.png")):
            if not icon.stem.isdecimal():
                continue
            destination = root / f"usr/share/icons/hicolor/{icon.stem}x{icon.stem}/apps"
            destination.mkdir(parents=True)
            stage_file(icon, destination / "io.codisseu.Codisseu.png")
        docs = root / "usr/share/doc/codisseu"
        docs.mkdir(parents=True)
        (docs / "copyright").write_text(
            "Codisseu\n" + homepage + "\n\n"
            "The application and its third-party runtimes have separate licenses.\n"
            "Complete package notices and version/source records are installed at\n"
            "/opt/codisseu/share/codisseu/notices and /opt/codisseu/BUILD.json.\n"
            "Corresponding-source material, where required, accompanies the release.\n"
        )
        control = root / "DEBIAN"
        control.mkdir()
        installed_kib = (sum(path.stat().st_size for path in root.rglob("*") if path.is_file()) + 1023) // 1024
        (control / "control").write_text(
            f"Package: codisseu\nVersion: {version}\nArchitecture: amd64\n"
            f"Maintainer: {maintainer}\nSection: devel\nPriority: optional\n"
            f"Installed-Size: {installed_kib}\nHomepage: {homepage}\n"
            f"Depends: {dependencies}\n"
            "Suggests: git, docker.io\n"
            "Description: IDE nativa para PHP, JavaScript, TypeScript e bancos de dados\n"
            " Editor, terminal, Git, ferramentas de banco e linguagens no mesmo app.\n"
            + (" Ferramentas de linguagem sao opcionais e configuradas explicitamente.\n" if core else " Inclui motores privados de analise PHP/Node/Rust/Python para uso offline.\n") +
            " Versao alpha para Linux x86_64; requer uma sessao grafica e driver do SO.\n"
        )
        rows = []
        for path in sorted(root.rglob("*")):
            if path.is_file() and not path.is_relative_to(control) and not path.is_symlink():
                rows.append(digest(path, "md5") + "  " + path.relative_to(root).as_posix())
        (control / "md5sums").write_text("\n".join(rows) + "\n")
        pending = output / ("." + package.name + ".partial")
        try:
            subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "-z1", "--build", root, pending], check=True)
            pending.replace(package)
        finally:
            pending.unlink(missing_ok=True)
    sha = digest(package)
    package.with_suffix(".deb.sha256").write_text(sha + "  " + package.name + "\n")
    report.update({"package": package.name, "version": version, "architecture": "amd64", "sha256": sha, "bytes": package.stat().st_size, "maintainer_scripts": False, "downloads_on_install": False, "variant": "core" if core else "full", "depends": dependencies})
    package.with_suffix(".deb.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--bundle", type=Path)
    source.add_argument("--core-binary", type=Path)
    parser.add_argument("--notices", type=Path, help="Prepared application/Cargo/asset notices for --core-binary")
    parser.add_argument("--output", type=Path, default=Path("dist"))
    parser.add_argument("--version", default="0.1.0~alpha.3")
    parser.add_argument("--maintainer", required=True)
    parser.add_argument("--homepage", default="https://github.com/lucstrik/codisseu")
    args = parser.parse_args()
    if args.core_binary:
        if args.notices is None:
            parser.error("--core-binary requires --notices")
        with tempfile.TemporaryDirectory(prefix="codisseu-core-bundle-") as temporary:
            bundle = Path(temporary) / "bundle"
            prepare_core_bundle(args.core_binary, bundle, args.notices)
            report = build_package(bundle, args.output, args.version, args.maintainer, args.homepage)
    else:
        report = build_package(args.bundle, args.output, args.version, args.maintainer, args.homepage)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
