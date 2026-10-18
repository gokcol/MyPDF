#!/usr/bin/env python3
"""Safety checks for cleanup using disposable fixtures, never the checkout."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import subprocess

spec = importlib.util.spec_from_file_location("clean", Path(__file__).with_name("clean-build-output.py"))
clean = importlib.util.module_from_spec(spec)
spec.loader.exec_module(clean)


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.file("Cargo.toml").write_text('[package]\nversion = "1.2.0"\n')

    def file(self, name):
        p = self.root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text("keep or discard fixture")
        return p

    def run_cleanup(self, reviews=True, apply=True):
        with patch.object(clean, "tracked_files", return_value=[]), patch.object(clean, "assert_idle"):
            return clean.clean(self.root, clean.candidates(self.root, reviews), apply)

    def test_removes_compiler_and_old_packages_but_keeps_all_user_data(self):
        remove = [self.file(p) for p in [
            "target/debug/deps/test.o", "target/debug/incremental/form_profile-hash/data.o",
            "target/release/mypdf", "ipad/core/target/aarch64-apple-ios/release/libmypdf.a",
            "ipad/build/module-cache", "artifacts/old/MyPDF.app/Contents/MacOS/MyPDF",
            "artifacts/old/MyPDF-1.1.0.dmg", "artifacts/old/MyPDF-1.1.0-windows-x64.zip",
        ]]
        keep = [self.file(p) for p in [
            "target/dev-profile/assets.json", "target/dev-profile/recovery/1.recovery",
            "artifacts/old/user-data/assets.json", "artifacts/old/user-data/recovery/1.recovery",
            "artifacts/old/native-profile/MyPDF-1.1.0.zip", "artifacts/old/documents/MyPDF-1.1.0.zip",
            "artifacts/MyPDF-1.1-Store-listing-en-US.zip", "artifacts/proof.pdf",
            "artifacts/personal-id.p12", "artifacts/stamp.svg", "artifacts/stamp.png",
            "dist/release/MyPDF.app/Contents/MacOS/MyPDF", "dist/release/MyPDF-1.2.0.dmg",
            "dist/MyPDF.app/Contents/MacOS/MyPDF",
            "artifacts/release-1.2.0/windows/MyPDF-1.2.0-Setup.exe",
            "artifacts/release-1.2.0-build126/msix/MyPDF-1.12.0.0-store-x64.msix",
            "dist/rollback/MyPDF.app/Contents/MacOS/MyPDF", "dist/rollback/user-data/assets.json",
            "dist/.release-work.pending/MyPDF.app/Contents/MacOS/MyPDF",
            ".cache/cargo/credentials.toml", ".cache/cargo/registry/src/dependency.rs",
        ]]
        self.run_cleanup()
        self.assertTrue(all(not p.exists() for p in remove))
        self.assertTrue(all(p.exists() for p in keep))

    def test_dry_run_and_review_opt_in(self):
        build = self.file("target/debug/file.o")
        app = self.file("artifacts/old/MyPDF.app/Contents/file")
        self.run_cleanup(apply=False)
        self.assertTrue(build.exists() and app.exists())
        self.run_cleanup(reviews=False)
        self.assertFalse(build.exists())
        self.assertTrue(app.exists())

    def test_unexpected_recovery_or_identity_inside_build_output_blocks_it(self):
        data = self.file("target/debug/isolated/recovery/unsaved.recovery")
        certificate = self.file("ipad/build/personal.P12")
        obj = self.file("target/debug/compiled.o")
        self.run_cleanup()
        self.assertTrue(data.exists() and certificate.exists() and obj.exists())

    def test_unexpected_documents_and_pem_are_preserved(self):
        pdf = self.file("target/debug/notes.PDF")
        key = self.file("target/release/identity/private.pem")
        self.run_cleanup()
        self.assertTrue(pdf.exists() and key.exists())

    def test_named_profile_inside_compiler_output_is_preserved(self):
        note = self.file("target/debug/custom-profile/note.txt")
        self.run_cleanup()
        self.assertTrue(note.exists())

    def test_symlink_root_ancestor_and_nested_links_never_followed(self):
        outside = self.root / "outside"
        outside.mkdir()
        precious = self.file("outside/MyPDF.app/Contents/file")
        (self.root / "artifacts").symlink_to(outside, target_is_directory=True)
        (self.root / "target").mkdir()
        (self.root / "target/debug").symlink_to(outside, target_is_directory=True)
        self.file("target/release/binary")
        (self.root / "target/release/link").symlink_to(outside, target_is_directory=True)
        self.run_cleanup()
        self.assertTrue(precious.exists())
        self.assertTrue((self.root / "target/debug").is_symlink())
        self.assertFalse((self.root / "target/release").exists())

    def test_source_file_prevents_deletion(self):
        source = self.file("target/debug/accidentally-tracked.rs")
        with patch.object(clean, "tracked_files", return_value=[source]), self.assertRaises(RuntimeError):
            clean.clean(self.root, clean.candidates(self.root), True)
        self.assertTrue(source.exists())

    def test_apply_stops_if_process_check_fails_or_build_is_active(self):
        for result in [subprocess.CompletedProcess([], 1, "", "denied"), subprocess.CompletedProcess([], 0, "123 /opt/bin/cargo\n", "")]:
            with patch.object(clean.subprocess, "run", return_value=result), self.assertRaises(RuntimeError):
                clean.assert_idle(self.root)
        (self.root / "dist/.release-lock").mkdir(parents=True)
        with self.assertRaises(RuntimeError):
            clean.assert_idle(self.root)

    def test_replaced_candidate_is_not_deleted(self):
        self.file("target/debug/compiled.o")
        def change(_root):
            (self.root / "target/debug").rename(self.root / "retained")
            self.file("target/debug/unsaved.txt")
        with patch.object(clean, "tracked_files", return_value=[]), patch.object(clean, "assert_idle", side_effect=change), self.assertRaises(RuntimeError):
            clean.clean(self.root, clean.candidates(self.root), True)
        self.assertTrue((self.root / "target/debug/unsaved.txt").exists())

    def test_new_recovery_after_preview_is_not_deleted(self):
        self.file("target/debug/compiled.o")
        def write_recovery(_root):
            self.file("target/debug/unsaved.recovery")
        with patch.object(clean, "tracked_files", return_value=[]), patch.object(clean, "assert_idle", side_effect=write_recovery), self.assertRaises(RuntimeError):
            clean.clean(self.root, clean.candidates(self.root), True)
        self.assertTrue((self.root / "target/debug/unsaved.recovery").exists())


if __name__ == "__main__":
    unittest.main()
