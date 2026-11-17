#!/usr/bin/env python3
"""Inspect a real .deb; optionally install it in a disposable Linux container.

The optional container step explicitly downloads OS dependencies with apt, then
disconnects its network before testing. It never installs anything on the host.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import uuid

import build_deb


def run(arguments, timeout=180, **kwargs):
    return subprocess.run([str(item) for item in arguments], check=True, capture_output=True, text=True, timeout=timeout, **kwargs)


def inspect(package, output):
    package = Path(package).resolve(strict=True)
    report = {"package": package.name, "sha256": build_deb.digest(package), "bytes": package.stat().st_size}
    expected = package.with_suffix(".deb.sha256").read_text().strip().split("  ")
    if expected != [report["sha256"], package.name]:
        raise ValueError("Release checksum does not match the package")
    extracted = output / "extracted"
    run(["dpkg-deb", "--raw-extract", package, extracted])
    report["bundle"] = build_deb.verify_bundle(extracted / "opt/codisseu")
    report["control"] = (extracted / "DEBIAN/control").read_text()
    if any((extracted / "DEBIAN" / name).exists() for name in ["preinst", "postinst", "prerm", "postrm"]):
        raise ValueError("Unexpected Debian maintainer script")
    for path in extracted.rglob("*"):
        if path.is_file() and path.stat().st_mode & 0o6000:
            raise ValueError("Unexpected privileged executable")
    launcher = extracted / "opt/codisseu/bin/codisseu"
    with tempfile.TemporaryDirectory(prefix="codisseu-deb-profile-") as temporary:
        home = Path(temporary)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "XDG_CONFIG_HOME": str(home / "config"), "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"), "XDG_CACHE_HOME": str(home / "cache"), "LANG": "C.UTF-8"}
        report["version"] = run([launcher, "--version"], env=env).stdout.strip()
        help_text = run([launcher, "--help"], env=env).stdout
        if "Codisseu" not in help_text:
            raise ValueError("Unexpected CLI help")
        report["runtime_info"] = json.loads(run([launcher, "--runtime-info"], env=env).stdout)
        source = home / "fixture.php"
        source.write_text("<?php\n// arquivo de teste do pacote\necho 'olá';\n")
        report["check_file"] = run([launcher, "--check-file", source], env=env).stdout.strip()
        report["profile_removed"] = True
    report["maintainer_scripts"] = False
    report["privileged_files"] = False
    return report


def container_install(package, image, output):
    # Inspect ensures that this verifier never pulls an unspecified base image.
    identity = json.loads(run(["docker", "image", "inspect", image]).stdout)[0]["Id"]
    name = "codisseu-release-check-" + uuid.uuid4().hex[:12]
    container = run(["docker", "create", "--pull", "never", "--name", name,
                     "--label", "io.codisseu.fixture=debian-release", "--memory", "1536m", "--cpus", "2",
                     "--mount", f"type=bind,src={package},dst=/tmp/codisseu.deb,readonly",
                     "--entrypoint", "/bin/sleep", identity, "infinity"]).stdout.strip()
    report = {"base_image_id": identity, "normal_host_profile_used": False, "container": container}
    try:
        run(["docker", "start", container])
        install = run(["docker", "exec", "-e", "DEBIAN_FRONTEND=noninteractive", container, "/bin/sh", "-ec",
                       "apt-get update && apt-get install -y --no-install-recommends /tmp/codisseu.deb"], timeout=600)
        (output / "container-apt.log").write_text(install.stdout + install.stderr)
        run(["docker", "network", "disconnect", "bridge", container])
        networks = json.loads(run(["docker", "inspect", container]).stdout)[0]["NetworkSettings"]["Networks"]
        if networks:
            raise ValueError("Test container is still attached to a network")
        script = r'''
for tool in php node npm composer rustc cargo python3 git docker; do
  if command -v "$tool" >/dev/null 2>&1; then echo "Unexpected optional runtime: $tool"; exit 1; fi
done
cat /etc/os-release
dpkg-query -W -f='${Package} ${Version}\n' codisseu
codisseu --version
codisseu --help >/tmp/codisseu-help.txt
codisseu --runtime-info
printf "<?php\n// fixture offline\necho 'ola';\n" >/tmp/codisseu-fixture.php
codisseu --check-file /tmp/codisseu-fixture.php
test -s /tmp/codisseu-help.txt
'''
        checked = run(["docker", "exec", container, "/usr/bin/env", "-i", "HOME=/tmp/codisseu-profile", "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "/bin/sh", "-ec", script])
        (output / "container-offline.log").write_text(checked.stdout + checked.stderr)
        report.update({"installation_exit": 0, "offline_cli_exit": 0, "network_attachments_during_checks": networks, "optional_runtimes_absent": True})
    finally:
        run(["docker", "rm", "--force", container])
        report["container_removed"] = True
        (output / "container.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--container-image", help="Already installed Debian-compatible base image; no image is pulled")
    args = parser.parse_args()
    output = args.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    report = {"result": "failed"}
    try:
        report.update(inspect(args.package, output))
        if args.container_image:
            report["container_checks"] = container_install(args.package.resolve(strict=True), args.container_image, output)
        report["result"] = "passed"
    finally:
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
