#!/usr/bin/env python3
"""Owned runtime/compatibility installation lifecycle tests, without processes."""
import hashlib
import json
import pathlib
import tempfile
import unittest
from unittest import mock
from providers import phpactor_runtime as runtime

ROOT = pathlib.Path(__file__).resolve().parent.parent


class RuntimeInstall(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='codisseu-runtime-test-')
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.archive = self.root / 'fixture.phar'
        self.archive.write_bytes(b'owned archive fixture')
        self.digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.destination = self.root / 'installed'

    def test_rejects_wrong_archive_before_creating_runtime(self):
        with self.assertRaisesRegex(ValueError, 'checksum'):
            runtime.unpack_phpactor(self.archive, self.destination, '/unused/php')
        self.assertFalse(self.destination.exists())

    def test_preserves_foreign_existing_directory(self):
        self.destination.mkdir()
        sentinel = self.destination / 'user.txt'
        sentinel.write_text('preserve')
        with mock.patch.object(runtime, 'PHPACTOR_SHA256', self.digest):
            with self.assertRaisesRegex(ValueError, 'not managed'):
                runtime.unpack_phpactor(self.archive, self.destination, '/unused/php')
        self.assertEqual(sentinel.read_text(), 'preserve')

    def test_failed_unpack_removes_only_its_staging(self):
        sentinel = self.root / 'unrelated'
        sentinel.write_text('preserve')
        with mock.patch.object(runtime, 'PHPACTOR_SHA256', self.digest):
            with mock.patch.object(runtime.subprocess, 'run', side_effect=OSError('failure')):
                with self.assertRaises(OSError):
                    runtime.unpack_phpactor(self.archive, self.destination, '/unused/php')
        self.assertFalse(self.destination.exists())
        self.assertEqual(sentinel.read_text(), 'preserve')
        self.assertEqual(list(self.root.glob('.phpactor-install-*')), [])

    def test_manifest_and_repeated_install_preserve_runtime(self):
        def extract(args, **kwargs):
            stage = pathlib.Path(args[-1])
            for name in ['bin/phpactor', 'vendor/autoload.php', 'LICENSE', '.box/notice']:
                path = stage / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('original ' + name)
        with mock.patch.object(runtime, 'PHPACTOR_SHA256', self.digest):
            with mock.patch.object(runtime.subprocess, 'run', side_effect=extract) as process:
                entry = runtime.unpack_phpactor(self.archive, self.destination, '/unused/php')
                record = json.loads((self.destination / 'codisseu-runtime.json').read_text())
                self.assertIn('.box/notice', record['files'])
                before = entry.stat().st_mtime_ns
                self.assertEqual(runtime.unpack_phpactor(self.archive, self.destination,
                                                        '/unused/php'), entry)
                self.assertEqual(entry.stat().st_mtime_ns, before)
                self.assertEqual(process.call_count, 1)


class CompatibilityInstall(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='codisseu-compat-test-')
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.entry = self.root / f'phpactor-{runtime.PHPACTOR_VERSION}/bin/phpactor'
        self.entry.parent.mkdir(parents=True)
        self.entry.write_text('<?php // owned runtime fixture')
        self.marker = self.entry.parent.parent / 'codisseu-runtime.json'
        self.archive = b'owned known archive bytes'
        self.digest = hashlib.sha256(self.archive).hexdigest()
        self.marker.write_text(json.dumps({'source_sha256': self.digest}))
        self.legacy = self.root / 'phpactor.phar'
        self.patch_digest = mock.patch.object(runtime, 'PHPACTOR_SHA256', self.digest)
        self.patch_digest.start()
        self.addCleanup(self.patch_digest.stop)

    def test_migrates_only_known_archive_atomically_and_is_idempotent(self):
        self.legacy.write_bytes(self.archive)
        runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.read_bytes(), runtime.launcher_bytes())
        before = self.legacy.stat().st_mtime_ns
        runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.stat().st_mtime_ns, before)
        self.assertEqual(list(self.root.glob('.phpactor-compat-*')), [])

    def test_creates_absent_compatibility_path_without_touching_runtime(self):
        before = self.entry.read_bytes()
        runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.read_bytes(), runtime.launcher_bytes())
        self.assertEqual(self.entry.read_bytes(), before)

    def test_preserves_foreign_file_and_false_marker(self):
        for content in [b'foreign data', runtime.launcher_bytes() + b'// foreign addition']:
            self.legacy.write_bytes(content)
            with self.assertRaises(ValueError):
                runtime.install_compatibility_launcher(self.legacy, self.entry)
            self.assertEqual(self.legacy.read_bytes(), content)

    def test_preserves_symlink_and_its_target(self):
        foreign = self.root / 'foreign'
        foreign.write_bytes(self.archive)
        self.legacy.symlink_to(foreign)
        with self.assertRaises(ValueError):
            runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertTrue(self.legacy.is_symlink())
        self.assertEqual(foreign.read_bytes(), self.archive)

    def test_rolls_back_concurrent_foreign_replacement_without_deleting_it(self):
        self.legacy.write_bytes(self.archive)
        exchange = runtime._exchange
        raced = False
        def replace_before_exchange(left, right):
            nonlocal raced
            if not raced:
                raced = True
                self.legacy.write_bytes(b'foreign raced data')
            exchange(left, right)
        with mock.patch.object(runtime, '_exchange', side_effect=replace_before_exchange):
            with self.assertRaises(ValueError):
                runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.read_bytes(), b'foreign raced data')
        self.assertEqual(list(self.root.glob('.phpactor-compat-*')), [])

    def test_rolls_back_concurrent_symlink_without_following_it(self):
        self.legacy.write_bytes(self.archive)
        foreign = self.root / 'foreign'
        foreign.write_bytes(b'outside fixture')
        exchange = runtime._exchange
        raced = False
        def link_before_exchange(left, right):
            nonlocal raced
            if not raced:
                raced = True
                self.legacy.unlink()
                self.legacy.symlink_to(foreign)
            exchange(left, right)
        with mock.patch.object(runtime, '_exchange', side_effect=link_before_exchange):
            with self.assertRaises(ValueError):
                runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertTrue(self.legacy.is_symlink())
        self.assertEqual(foreign.read_bytes(), b'outside fixture')
        self.assertEqual(list(self.root.glob('.phpactor-compat-*')), [])

    def test_preserves_both_items_if_another_writer_prevents_rollback(self):
        self.legacy.write_bytes(self.archive)
        exchange = runtime._exchange
        def replace_after_exchange(left, right):
            exchange(left, right)
            replacement = self.root / 'third-writer'
            replacement.write_bytes(b'new concurrent file')
            replacement.replace(self.legacy)
        with mock.patch.object(runtime, '_exchange', side_effect=replace_after_exchange):
            with self.assertRaisesRegex(ValueError, 'preserved at'):
                runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.read_bytes(), b'new concurrent file')
        retained = list(self.root.glob('.phpactor-compat-*'))
        self.assertEqual(len(retained), 1)
        self.assertEqual(retained[0].read_bytes(), self.archive)

    def test_unavailable_atomic_exchange_preserves_archive_and_cleans_own_staging(self):
        self.legacy.write_bytes(self.archive)
        with mock.patch.object(runtime, '_exchange', side_effect=OSError('unsupported')):
            with self.assertRaises(OSError):
                runtime.install_compatibility_launcher(self.legacy, self.entry)
        self.assertEqual(self.legacy.read_bytes(), self.archive)
        self.assertEqual(list(self.root.glob('.phpactor-compat-*')), [])


if __name__ == '__main__':
    unittest.main()
