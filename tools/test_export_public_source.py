#!/usr/bin/env python3
"""Security regressions for the public export boundary, using disposable trees."""

import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import export_public_source as export


class PublicExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="codisseu-public-export-test-")
        self.base = Path(self.temp.name)
        self.root = self.base / "work"
        self.root.mkdir()
        self.put("Cargo.toml", '[workspace]\nmembers = ["apps/demo"]\n')
        self.put("Cargo.lock", "# synthetic lock\n")
        self.put("README.md", "# Public\n[Guide](docs/optional-tools.md)\n[History](docs/history/private.md)\n")
        self.put("apps/demo/Cargo.toml", '[package]\nname="demo"\nversion="0.1.0"\n')
        self.put("apps/demo/src/main.rs", 'fn main() { println!("hello"); }\n')
        self.status = self.base / "public-status.md"
        self.status.write_text("# Public status\nAlpha, limited validation.\n")

    def tearDown(self):
        self.temp.cleanup()

    def put(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def prepare(self):
        return export.prepare(self.root, self.status.read_bytes())

    def test_private_roots_and_historical_status_never_enter_snapshot(self):
        sentinel = "private-sentinel-never-publish"
        for path in (
            ".git/config", ".tools/providers.json", "target/private.txt", "artifacts/private.txt",
            "docs/status.md", "docs/history/private.md", "docs/report-20261116.md",
            "reference/private.json", "tools/reference_private.py", "crates/demo/.env",
            "fixtures/demo/.env.example", "apps/demo/session.ai.json", "crates/demo/secret.key",
            "crates/demo/artifacts/window.png", "fixtures/demo/node_modules/private.txt",
        ):
            self.put(path, sentinel)
        snapshot = self.prepare()
        self.assertFalse(any(sentinel.encode() in data for data in snapshot.payloads.values()))
        self.assertEqual(snapshot.payloads["docs/status.md"], self.status.read_bytes())
        self.assertNotIn("docs/history/private.md", snapshot.payloads)
        self.assertNotIn("[History]", snapshot.payloads["README.md"].decode())
        self.assertIn("History", snapshot.payloads["README.md"].decode())
        self.assertEqual(snapshot.sources["docs/status.md"], None)

    def test_symlink_in_included_tree_is_rejected_before_writing(self):
        outside = self.base / "outside-secret"
        outside.write_text("do-not-read")
        (self.root / "apps/demo/leak.txt").symlink_to(outside)
        with self.assertRaises(export.ExportError) as failure:
            self.prepare()
        self.assertEqual(failure.exception.kind, "symlink-refused")
        self.assertNotIn(str(self.base), str(failure.exception))
        self.assertFalse((self.base / "output").exists())

    def test_excluded_symlink_is_never_followed(self):
        (self.root / ".tools").symlink_to(self.base / "absent-private-tools")
        snapshot = self.prepare()
        self.assertFalse(any(path.startswith(".tools/") for path in snapshot.payloads))

    def test_high_confidence_token_and_key_are_blocked_without_echoing_values(self):
        token = "ghp_" + "A" * 36
        marker = "-----BEGIN " + "PRIVATE KEY-----"
        self.put("apps/demo/src/leak.rs", 'const VALUE: &str = "' + token + '";\n' + marker)
        snapshot = self.prepare()
        self.assertTrue(any(f["kind"] == "github-token" for f in snapshot.findings))
        report = json.dumps(snapshot.manifest())
        self.assertNotIn(token, report)
        self.assertNotIn(marker, report)
        output = self.base / "output"
        with self.assertRaises(export.ExportError):
            export.write_snapshot(snapshot, output)
        self.assertFalse(output.exists())

    def test_synthetic_basic_auth_is_inventoried_and_source_is_preserved(self):
        self.put("crates/demo/tests/uri.rs", 'const URI: &str = "postgres://fixture:dummy@127.0.0.1/db";\n')
        snapshot = self.prepare()
        findings = [f for f in snapshot.findings if "url-userinfo" in f["kind"]]
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["severity"], "review")
        self.assertNotIn("dummy", json.dumps(findings))
        self.assertFalse(any(f["severity"] == "block" for f in snapshot.findings))

    def test_exact_inspected_invalid_pem_sentinel_does_not_allow_real_key_body(self):
        path = "crates/codisseu-db/src/tls_roots.rs"
        begin = "-----BEGIN " + "PRIVATE KEY-----"
        end = "-----END " + "PRIVATE KEY-----"
        self.put(path, 'validate_pem(b"' + begin + '\\nsecret\\n' + end + '");')
        snapshot = self.prepare()
        findings = [f for f in snapshot.findings if "private-key" in f["kind"]]
        self.assertEqual(findings[0]["severity"], "review")
        self.put(path, begin + '\n' + 'A' * 64 + '\n' + end)
        snapshot = self.prepare()
        self.assertTrue(any(f["kind"] == "private-key" and f["severity"] == "block" for f in snapshot.findings))

    def test_snapshot_transformations_preserve_worktree_and_have_hash_provenance(self):
        company = "total" + "express"
        home = "/home/" + "lucas"
        source = self.put("crates/demo/tests/path.rs", 'let path = "' + home + '/projects/' + company + '";\nlet name = "' + 'Total' + 'Express' + '";\n')
        original = source.read_bytes()
        snapshot = self.prepare()
        path = "crates/demo/tests/path.rs"
        self.assertEqual(source.read_bytes(), original)
        self.assertNotIn(home.encode(), snapshot.payloads[path])
        self.assertNotIn(company.encode(), snapshot.payloads[path])
        self.assertIn(b"/home/usuario/projects/example_company", snapshot.payloads[path])
        entry = next(f for f in snapshot.manifest()["files"] if f["path"] == path)
        self.assertEqual(entry["source_sha256"], hashlib.sha256(original).hexdigest())
        self.assertEqual(entry["sha256"], hashlib.sha256(snapshot.payloads[path]).hexdigest())
        self.assertEqual(len(entry["transformations"]), 3)
        self.assertIn(b'ExampleGroup',snapshot.payloads[path])

    def test_doc_links_follow_only_explicit_allowlist_and_private_links_are_unlinked(self):
        self.put("docs/optional-tools.md", "[Usage](cli-usage.md)\n[Private](../reference/private.md)\n[Escape](../../outside.md)\n")
        self.put("docs/cli-usage.md", "# CLI\n[Public status](status.md)\n")
        self.put("reference/private.md", "private marker")
        snapshot = self.prepare()
        self.assertIn("docs/cli-usage.md", snapshot.payloads)
        self.assertNotIn("reference/private.md", snapshot.payloads)
        self.assertNotIn("../reference/private.md", snapshot.payloads["docs/optional-tools.md"].decode())
        selected = set(snapshot.payloads)
        for path, data in snapshot.payloads.items():
            if path.endswith(".md"):
                for target in export.link_targets(path, data.decode()):
                    self.assertTrue(target is None or target in selected, (path, target))

    def test_dry_run_does_not_create_output_and_private_status_is_refused(self):
        output = self.base / "output"
        args = ["--root", str(self.root), "--status-file", str(self.status), "--output", str(output)]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(export.main(args), 0)
        self.assertFalse(output.exists())
        private_status = self.put("docs/status.md", "private history")
        args[3] = str(private_status)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(export.main(args), 2)
        self.assertFalse(output.exists())

    def test_write_creates_manifest_refuses_existing_output_and_symlink_parents(self):
        snapshot = self.prepare()
        output = self.base / "output"
        export.write_snapshot(snapshot, output)
        manifest = json.loads((output / "PUBLIC_SOURCE_MANIFEST.json").read_text())
        self.assertFalse(manifest["git_history_included"])
        for item in manifest["files"]:
            self.assertEqual(item["sha256"], hashlib.sha256((output / item["path"]).read_bytes()).hexdigest())
        with self.assertRaises(export.ExportError):
            export.write_snapshot(snapshot, output)
        linked = self.base / "linked"
        linked.symlink_to(self.base, target_is_directory=True)
        with self.assertRaises(export.ExportError):
            export.write_snapshot(snapshot, linked / "escape")
        self.assertFalse((self.base / "escape").exists())


if __name__ == "__main__":
    unittest.main()
