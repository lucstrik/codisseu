#!/usr/bin/env python3
"""Debian archive and input-integrity checks using a disposable tiny bundle."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import build_deb


class DebianPackage(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codisseu-deb-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bundle = self.root / "Fixture bundle com espaços"
        for relative, data in {
            "bin/codisseu": b'#!/bin/sh\nprintf "%s\\n" "$@"\n',
            "libexec/codisseu-bin": b"owned packaging fixture; not an application binary",
            "BUNDLE-ID": b"codisseu-packaging-fixture\n",
            "share/codisseu/runtime.json": b'{"schema_version":1,"tools":{},"providers":[]}',
            "share/codisseu/icons/256.png": b"owned packaging fixture; not a screenshot",
        }.items():
            target = self.bundle / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        (self.bundle / "bin/codisseu").chmod(0o755)
        (self.bundle / "BUILD.json").write_text(json.dumps({
            "platform": "linux-x86_64", "downloads_on_install_or_first_use": False,
            "binary_sha256": build_deb.digest(self.bundle / "libexec/codisseu-bin"),
        }))
        self.write_manifest()

    def write_manifest(self):
        rows = [build_deb.digest(path) + "  " + path.relative_to(self.bundle).as_posix()
                for path in sorted(self.bundle.rglob("*"))
                if path.is_file() and path.name != "MANIFEST.sha256"]
        (self.bundle / "MANIFEST.sha256").write_text("\n".join(rows) + "\n")

    def test_modified_missing_extra_and_external_symlink_are_rejected(self):
        target = self.bundle / "BUNDLE-ID"
        original = target.read_bytes()
        target.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "checksum"):
            build_deb.verify_bundle(self.bundle)
        target.write_bytes(original)
        extra = self.bundle / "extra"
        extra.write_bytes(b"unlisted")
        with self.assertRaisesRegex(ValueError, "unlisted"):
            build_deb.verify_bundle(self.bundle)
        extra.unlink()
        target.unlink()
        with self.assertRaises(FileNotFoundError):
            build_deb.verify_bundle(self.bundle)
        target.write_bytes(original)
        extra.symlink_to("/etc/passwd")
        with self.assertRaisesRegex(ValueError, "symlink"):
            build_deb.verify_bundle(self.bundle)

    def test_manifest_traversal_duplicate_and_control_injection_are_rejected(self):
        manifest = self.bundle / "MANIFEST.sha256"
        original = manifest.read_text()
        manifest.write_text(original + "0" * 64 + "  ../outside\n")
        with self.assertRaisesRegex(ValueError, "Unsafe"):
            build_deb.verify_bundle(self.bundle)
        manifest.write_text(original + original.splitlines()[0] + "\n")
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            build_deb.verify_bundle(self.bundle)
        manifest.write_text(original)
        with self.assertRaisesRegex(ValueError, "single line"):
            build_deb.build_package(self.bundle, self.root / "output", "0.1.0\nDepends: php", "Test <test@example.invalid>", "https://github.com/example/test")

    @unittest.skipUnless(shutil.which("dpkg-deb"), "requires dpkg-deb")
    def test_real_deb_preserves_bundle_and_contains_only_explicit_launchers(self):
        report = build_deb.build_package(self.bundle, self.root / "output", "0.1.0~alpha.1", "Test <test@example.invalid>", "https://github.com/example/test")
        package = self.root / "output" / report["package"]
        self.assertEqual(report["sha256"], build_deb.digest(package))
        extracted = self.root / "extracted"
        subprocess.run(["dpkg-deb", "--raw-extract", package, extracted], check=True, capture_output=True)
        self.assertEqual(build_deb.verify_bundle(self.bundle), build_deb.verify_bundle(extracted / "opt/codisseu"))
        control = (extracted / "DEBIAN/control").read_text()
        depends = next(row for row in control.splitlines() if row.startswith("Depends:"))
        self.assertNotIn("php", depends)
        self.assertNotIn("node", depends)
        self.assertFalse(any((extracted / "DEBIAN" / script).exists() for script in ("preinst", "postinst", "prerm", "postrm")))
        self.assertEqual((extracted / "usr/bin/codisseu").read_text(), '#!/bin/sh\nexec /opt/codisseu/bin/codisseu --cli "$@"\n')
        desktop = extracted / "usr/share/applications/io.codisseu.Codisseu.desktop"
        self.assertIn("Exec=/opt/codisseu/bin/codisseu %F", desktop.read_text())
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", desktop], check=True)
        with self.assertRaises(FileExistsError):
            build_deb.build_package(self.bundle, self.root / "output", "0.1.0~alpha.1", "Test <test@example.invalid>", "https://github.com/example/test")

    @unittest.skipUnless(shutil.which("dpkg-shlibdeps"), "requires dpkg-shlibdeps")
    def test_core_bundle_has_authoritative_empty_runtimes_and_system_libraries(self):
        notices = self.root / "notices"
        notices.mkdir()
        (notices / "LICENSE-fixture.txt").write_text("Owned test fixture; not a distribution license.\n")
        core = self.root / "core"
        metadata = build_deb.prepare_core_bundle(Path("/bin/true"), core, notices)
        self.assertEqual(metadata["package_variant"], "core")
        self.assertIn("libc6", metadata["system_dependencies"])
        self.assertFalse(metadata["bundled_language_runtimes"])
        self.assertEqual(json.loads((core / "share/codisseu/runtime.json").read_text()), {"schema_version": 1, "tools": {}, "providers": []})
        self.assertFalse((core / "lib").exists())
        self.assertFalse((core / "share/codisseu/runtime").exists())
        self.assertGreater(build_deb.verify_bundle(core)["files"], 10)


if __name__ == "__main__":
    unittest.main()
