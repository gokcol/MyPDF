#!/usr/bin/env python3
"""Disposable selection/archival fixtures; never touch real apps or Launch Services."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('cleanup', Path(__file__).with_name('clean-macos-open-with.py'))
cleanup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleanup)


class RegistrationSelection(unittest.TestCase):
    def test_only_our_obsolete_bundle_paths_are_selected(self):
        records = [
            {'identifier': 'dev.mypdf.desktop', 'path': '/Applications/MyPDF.app'},
            {'identifier': 'dev.mypdf.desktop', 'path': '/backups/previous-installed.app'},
            {'identifier': 'dev.mypdf.desktop.review', 'path': '/builds/MyPDF Review.app'},
            {'identifier': 'dev.mypdf.save-review', 'path': '/builds/MyPDF Save Review.app'},
            {'identifier': 'org.mypdf.ipad.dev', 'path': '/Applications/MyPDF iPad.app'},
            {'identifier': 'com.parallels.winapp.mypdf', 'path': '/Applications/MyPDF Windows.app'},
            {'identifier': 'dev.mypdf.other', 'path': '/builds/a-document.pdf'},
            {'identifier': 'com.adobe.Acrobat', 'path': '/Applications/Adobe Acrobat.app'},
        ]
        self.assertEqual(cleanup.candidates(records, Path('/Applications/MyPDF.app')), [
            Path('/backups/previous-installed.app'), Path('/builds/MyPDF Review.app'), Path('/builds/MyPDF Save Review.app')
        ])

    def test_dump_parser_does_not_mistake_bindings_for_app_bundles(self):
        dump = '''header
------------------------------
bundle id:                  Review (0x42)
path:                       /builds/with spaces/MyPDF Review.app (0x1234)
name:                       MyPDF Review
identifier:                 dev.mypdf.desktop.review
versionString:              185
------------------------------
binding id:                 com.adobe.pdf (0x43)
name:                       MyPDF
identifier:                 dev.mypdf.desktop.fake-binding
path:                       /Applications/Do Not Remove.app (0x1235)
'''
        self.assertEqual(cleanup.registrations(dump), [{
            'path': '/builds/with spaces/MyPDF Review.app', 'name': 'MyPDF Review',
            'identifier': 'dev.mypdf.desktop.review', 'versionString': '185',
        }])

    def test_archive_requires_explicit_apply(self):
        with mock.patch('sys.argv', ['cleanup', '--archive']), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as error:
                cleanup.main()
        self.assertEqual(error.exception.code, 2)

    def test_running_paths_allow_spaces_and_do_not_confuse_document_arguments(self):
        self.assertEqual(cleanup.running_bundle_paths([
            '/builds/MyPDF Review.app/Contents/MacOS/MyPDF /docs/example.pdf',
            '/usr/bin/python3 /builds/Fake.app/file.py',
            '/Applications/MyPDF.app/Contents/MacOS/MyPDF',
        ]), {Path('/builds/MyPDF Review.app'), Path('/Applications/MyPDF.app')})


class ManagedArchive(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='mypdf-cleanup-test-')
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / 'repo/artifacts'
        self.root.mkdir(parents=True)
        self.app = self.make_bundle(self.root / 'MyPDF Review.app')
        self.installed = self.base / 'Applications/MyPDF.app'
        self.patches = [
            mock.patch.object(cleanup, 'process_commands', return_value=[]),
            mock.patch.object(cleanup, 'tracked_files', return_value=False),
            mock.patch.object(cleanup, 'acl', return_value=[]),
            mock.patch.object(cleanup, 'code_signature', return_value={'valid': True}),
        ]
        for patch in self.patches:
            patch.start()
        self.addCleanup(self.temporary.cleanup)
        self.addCleanup(lambda: [patch.stop() for patch in reversed(self.patches)])

    def make_bundle(self, path, identifier='dev.mypdf.desktop.review'):
        (path / 'Contents/MacOS').mkdir(parents=True)
        (path / 'Contents/Resources').mkdir()
        (path / 'Contents/_CodeSignature').mkdir()
        (path / 'Contents/Info.plist').write_bytes(plistlib.dumps({
            'CFBundleIdentifier': identifier, 'CFBundleExecutable': 'MyPDF',
            'CFBundleIconFile': 'MyPDF-calico-v2.icns',
        }))
        (path / 'Contents/MacOS/MyPDF').write_bytes(b'executable fixture\x00')
        (path / 'Contents/MacOS/MyPDF').chmod(0o755)
        (path / 'Contents/Resources/MyPDF-calico-v2.icns').write_bytes(b'icon fixture')
        (path / 'Contents/_CodeSignature/CodeResources').write_bytes(b'signature fixture')
        return path

    def eligible(self, path=None, commands=None):
        return cleanup.archive_eligibility(path or self.app, [self.root], self.installed, commands or [])

    def fake_tools(self, restore_change=None):
        def run(arguments, **kwargs):
            if arguments[0] == '/usr/bin/ditto' and '-c' in arguments:
                Path(arguments[-1]).write_bytes(b'verified archive fixture')
            elif arguments[0] == '/usr/bin/ditto' and '-x' in arguments:
                restored = Path(arguments[-1]) / self.app.name
                shutil.copytree(self.app, restored, symlinks=True)
                if restore_change:
                    restore_change(restored)
            elif arguments[0] != str(cleanup.REGISTER):
                raise AssertionError(f'Unexpected external command: {arguments}')
            return subprocess.CompletedProcess(arguments, 0, '', '')
        return mock.patch.object(cleanup.subprocess, 'run', side_effect=run)

    def test_discovery_includes_unregistered_apps_and_ignores_symlink_ancestors(self):
        second = self.make_bundle(self.root / 'nested/Another.app')
        (self.root / 'Linked.app').symlink_to(self.app, target_is_directory=True)
        self.assertEqual(cleanup.discover_bundles([self.root]), [self.app, second])
        alias = self.base / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        self.assertEqual(cleanup.discover_bundles([alias]), [])

    def test_installed_and_running_apps_are_protected(self):
        self.assertTrue(self.eligible()[0])
        with mock.patch.object(self, 'installed', self.app):
            self.assertEqual(self.eligible(), (False, 'installed application'))
        self.assertEqual(self.eligible(commands=[str(self.app / 'Contents/MacOS/MyPDF')]),
                         (False, 'running application'))

    def test_active_build_signing_or_packaging_blocks_archival(self):
        for command in ['/opt/homebrew/bin/cargo build --release',
                        '/bin/sh scripts/release-macos.sh', '/usr/bin/codesign --force /tmp/test.app',
                        '/usr/bin/hdiutil create /tmp/test.dmg']:
            with self.subTest(command=command):
                self.assertIn('active', self.eligible(commands=[command])[1])

    def test_foreign_bundle_and_unmanaged_path_are_protected(self):
        foreign = self.make_bundle(self.root / 'Other.app', 'com.adobe.Acrobat')
        windows = self.make_bundle(self.root / 'Windows.app', 'com.parallels.winapp.mypdf')
        ipad = self.make_bundle(self.root / 'iPad.app', 'org.mypdf.ipad.dev')
        outside = self.make_bundle(self.base / 'personal/MyPDF.app')
        for path in [foreign, windows, ipad, outside]:
            self.assertFalse(self.eligible(path)[0])

    def test_expired_missing_foreign_records_are_audited_without_existing_app_alert(self):
        existing = self.base / 'Other.app'
        existing.mkdir()
        missing = self.base / 'UnavailableVolume/Qt.app'
        records = [{'identifier': 'org.qt-project.Designer', 'path': str(missing)},
                   {'identifier': 'com.example.other', 'path': str(existing)},
                   {'identifier': 'dev.mypdf.desktop.review', 'path': str(self.app)}]
        removed, existing_removed = cleanup.unrelated_removals(records, [])
        self.assertEqual(removed, [('com.example.other', str(existing)),
                                   ('org.qt-project.Designer', str(missing))])
        self.assertEqual(existing_removed, [('com.example.other', str(existing))])

    def test_tracked_source_and_symlinked_app_are_protected(self):
        with mock.patch.object(cleanup, 'tracked_files', return_value=True):
            self.assertEqual(self.eligible(), (False, 'contains tracked source'))
        alias = self.root / 'Alias.app'
        alias.symlink_to(self.app, target_is_directory=True)
        self.assertIn('symlink', self.eligible(alias)[1])

    def test_personal_files_and_nested_apps_are_never_retired(self):
        personal = self.app / 'Contents/Resources/Personal.pdf'
        personal.write_bytes(b'private document')
        self.assertIn('unexpected file', self.eligible()[1])
        personal.unlink()
        (self.app / 'Contents/Resources/Helper.app').mkdir()
        self.assertIn('unexpected directory', self.eligible()[1])

    def test_hardlinks_and_external_symlinks_are_protected(self):
        binary = self.app / 'Contents/MacOS/MyPDF'
        alias = self.base / 'hardlink'
        os.link(binary, alias)
        self.assertIn('hard link', self.eligible()[1])
        alias.unlink()
        external = self.app / 'Contents/CodeResources'
        external.symlink_to(self.base / 'elsewhere')
        self.assertIn('external symlink', self.eligible()[1])

    def test_verified_archive_retires_without_deleting_original_payload(self):
        original = cleanup.tree_snapshot(self.app)
        with self.fake_tools():
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertTrue(report['archived'], report)
        self.assertFalse(self.app.exists())
        retired = Path(report['retired_directory'])
        self.assertTrue((retired / 'ArchivedContents/MacOS/MyPDF').exists())
        self.assertFalse((retired / 'Contents').exists())
        self.assertTrue(Path(report['archive']).is_file())
        manifest = json.loads(Path(report['manifest']).read_text())
        self.assertEqual(manifest['source_snapshot'], original)
        self.assertIn('ditto', manifest['restore_command'])
        self.assertEqual(len(manifest['restore_retired_steps']), 4)
        # Reversing the two documented renames restores the signed payload.
        (retired / 'ArchivedContents').rename(retired / 'Contents')
        retired.rename(self.app)
        restored = cleanup.tree_snapshot(self.app)
        self.assertEqual(cleanup.archive_comparable(restored), cleanup.archive_comparable(original))

    def test_bad_archive_content_leaves_source_and_no_retirement(self):
        before = cleanup.tree_snapshot(self.app)
        with self.fake_tools(lambda restored: (restored / 'Contents/MacOS/MyPDF').write_bytes(b'wrong')):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertIn('changed content', report['reason'])
        self.assertEqual(cleanup.tree_snapshot(self.app), before)
        self.assertFalse(Path(report['retired_directory']).exists())

    def test_bad_archive_modes_leave_source(self):
        with self.fake_tools(lambda restored: (restored / 'Contents/MacOS/MyPDF').chmod(0o600)):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertTrue(self.app.is_dir())

    def test_changed_xattrs_or_acl_leave_source(self):
        for field, source_value, restored_value in [
            ('xattrs', {'dev.mypdf.fixture': 'original'}, {'dev.mypdf.fixture': 'changed'}),
            ('acl', ['0: user:fixture allow read'], ['0: user:fixture deny read']),
        ]:
            with self.subTest(field=field):
                def metadata(path):
                    return source_value if cleanup.within(path, self.app) else restored_value
                with self.fake_tools(), mock.patch.object(cleanup, field, side_effect=metadata):
                    report = cleanup.archive_bundle(self.app, [self.root], self.installed)
                self.assertFalse(report['archived'])
                self.assertTrue(self.app.is_dir())

    def test_missing_zip_root_macl_is_recorded_and_original_attribute_retained(self):
        original_attribute = {'com.apple.macl': 'original-access-attribute'}

        def metadata(path):
            retained_root = path.parent == self.root and '.retired-' in path.name
            return original_attribute if path == self.app or retained_root else {}

        with self.fake_tools(), mock.patch.object(cleanup, 'xattrs', side_effect=metadata):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertTrue(report['archived'], report)
        self.assertEqual(report['archive_metadata_exceptions'], [{
            'path': '.', 'attribute': 'com.apple.macl', 'change': 'missing from restored ZIP',
            'preserved_in': 'retained original directory and source_snapshot',
        }])
        self.assertEqual(report['source_snapshot']['.']['xattrs'], original_attribute)
        self.assertIn('all original metadata', report['full_metadata_restore'])

    def test_changed_root_macl_child_macl_or_other_missing_attribute_is_rejected(self):
        for relative, attribute, restored_value in [
            ('.', 'com.apple.macl', 'changed'),
            ('Contents/Info.plist', 'com.apple.macl', None),
            ('.', 'dev.mypdf.fixture', None),
            ('.', 'com.apple.provenance', None),
            ('.', 'com.apple.quarantine', None),
        ]:
            with self.subTest(relative=relative, attribute=attribute):
                def metadata(path):
                    if cleanup.within(path, self.app) and str(path.relative_to(self.app)) == relative:
                        return {attribute: 'original'}
                    if not cleanup.within(path, self.app) and str(path).endswith(
                            self.app.name + ('' if relative == '.' else '/' + relative)):
                        return {} if restored_value is None else {attribute: restored_value}
                    return {}
                with self.fake_tools(), mock.patch.object(cleanup, 'xattrs', side_effect=metadata):
                    report = cleanup.archive_bundle(self.app, [self.root], self.installed)
                self.assertFalse(report['archived'])
                self.assertIn('protected metadata', report['reason'])
                self.assertTrue(self.app.is_dir())

    def test_lost_retained_root_macl_is_rejected_and_both_renames_roll_back(self):
        def metadata(path):
            return {'com.apple.macl': 'original'} if path == self.app else {}

        with self.fake_tools(), mock.patch.object(cleanup, 'xattrs', side_effect=metadata):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertIn('Retirement changed original payload or protected metadata', report['reason'])
        self.assertTrue(self.app.is_dir())
        self.assertFalse(Path(report['retired_directory']).exists())

    def test_internal_signature_symlink_round_trips(self):
        (self.app / 'Contents/CodeResources').symlink_to('_CodeSignature/CodeResources')
        self.assertTrue(self.eligible()[0])
        with self.fake_tools():
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertTrue(report['archived'], report)
        retained = Path(report['retired_directory']) / 'ArchivedContents/CodeResources'
        self.assertEqual(os.readlink(retained), '_CodeSignature/CodeResources')

    def test_lost_valid_signature_leaves_source(self):
        with self.fake_tools(), mock.patch.object(cleanup, 'code_signature',
                                                 side_effect=[{'valid': True}, {'valid': False}]):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertIn('valid code signature', report['reason'])
        self.assertTrue(self.app.is_dir())

    def test_original_changed_during_archive_is_preserved(self):
        def change_original(_):
            (self.app / 'Contents/MacOS/MyPDF').write_bytes(b'changed original')
        with self.fake_tools(change_original):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertIn('original bundle changed', report['reason'])
        self.assertEqual((self.app / 'Contents/MacOS/MyPDF').read_bytes(), b'changed original')

    def test_retirement_rename_failure_rolls_back_first_rename(self):
        original = cleanup.tree_snapshot(self.app)
        real_rename = os.rename

        def fail_contents(source, destination):
            if Path(destination).name == 'ArchivedContents':
                raise OSError('fixture rename failure')
            return real_rename(source, destination)

        with self.fake_tools(), mock.patch.object(cleanup.os, 'rename', side_effect=fail_contents):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertIn('fixture rename failure', report['reason'])
        self.assertEqual(cleanup.tree_snapshot(self.app), original)
        self.assertTrue(Path(report['archive']).exists())

    def test_post_retirement_manifest_failure_rolls_back_both_renames(self):
        original = cleanup.tree_snapshot(self.app)
        real_write = Path.write_text
        writes = 0

        def fail_once(path, *args, **kwargs):
            nonlocal writes
            writes += 1
            if writes == 2:
                raise OSError('fixture final manifest failure')
            return real_write(path, *args, **kwargs)

        with self.fake_tools(), mock.patch.object(Path, 'write_text', fail_once):
            report = cleanup.archive_bundle(self.app, [self.root], self.installed)
        self.assertFalse(report['archived'])
        self.assertEqual(cleanup.tree_snapshot(self.app), original)
        self.assertFalse(Path(report['retired_directory']).exists())


if __name__ == '__main__':
    unittest.main()
