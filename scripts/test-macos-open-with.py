#!/usr/bin/env python3
"""Check cleanup selection preserves the installed app and unrelated handlers."""
import importlib.util
from pathlib import Path
import unittest

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


if __name__ == '__main__':
    unittest.main()
