import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("release_notes", Path(__file__).with_name("release-notes.py"))
notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notes)


class ReleaseNotesTests(unittest.TestCase):
    def test_shared_notes_and_unreleased_separation(self):
        data = {"unreleased": ["Not published"], "releases": [{"version": "2.4.2", "changes": ["Saved text stays readable."]}]}
        github, store = notes.render(data, "2.4.2")
        self.assertIn("Saved text stays readable.", github)
        self.assertIn("Saved text stays readable.", store)
        self.assertNotIn("Not published", github + store)
        with self.assertRaises(ValueError):
            notes.render(data, "2.4.3")

    def test_bundled_release_entries_are_exportable(self):
        data = json.loads((notes.ROOT / "changelog.json").read_text())
        for release in data["releases"]:
            github, store = notes.render(data, release["version"])
            self.assertTrue(github)
            self.assertLessEqual(len(store), 1500)


if __name__ == "__main__":
    unittest.main()
