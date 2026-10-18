#!/usr/bin/env python3
"""Preview or remove rebuildable MyPDF output; never clean entire data roots.

Default: preview compiler caches. --reviews also includes old app/package outputs.
Use --apply only after saving work and stopping builds. Current release, rollback,
profiles, recovery, documents, artwork and signing credentials are never selected.
"""
from __future__ import annotations

import argparse
from functools import lru_cache
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
STATE_FILES = {
    "assets.json", "workspace.json", "interface.json", "home.json",
    "identities.json", "trusted_certificates.json", "translation.json",
    "header_footer.json", "settings.json", "preferences.json",
}
PROTECTED_DIRS = {"recovery", "user-data", "mypdf-user-data", "documents", "preserved", "identity", "identities", "certificates"}
BUILDERS = {"cargo", "rustc", "swiftc", "swift-frontend", "xcodebuild", "cmake", "ninja", "make", "clang", "clang++", "cc", "c++", "ld"}
PACKAGE_NAME = re.compile(
    r"MyPDF-(?:\d+\.){1,3}\d+(?:-(?:Setup|windows-x64|store-x64|test-x64))?\.(?:dmg|msix|exe|zip)",
    re.IGNORECASE,
)


@lru_cache(maxsize=None)
def current_version(root: Path) -> str | None:
    manifest = root / "Cargo.toml"
    if not manifest.exists():
        return None
    match = re.search(r'^version\s*=\s*"([\d.]+)"', manifest.read_text(), re.MULTILINE)
    return match.group(1) if match else None


def protected(path: Path, root: Path) -> bool:
    parts = path.relative_to(root).parts
    if parts[:2] in (("dist", "release"), ("dist", "rollback")):
        return True
    # Keep the current development bundle and all copies of this version's
    # downloaded release packages, even when also consolidated in dist/release.
    if parts[:2] == ("dist", "MyPDF.app"):
        return True
    version = current_version(root)
    if version and any(p == f"release-{version}" or p.startswith(f"release-{version}-") for p in parts):
        return True
    return any(
        "profile" in p.lower() or p.lower() in PROTECTED_DIRS
        or p == ".release-lock" or p.startswith(".release-work.")
        for p in parts
    )


def safe_path(path: Path, root: Path) -> bool:
    """Only physical descendants: do not traverse even an in-repo symlink."""
    try:
        relative = path.relative_to(root)
        if not relative.parts or protected(path, root):
            return False
        current = root
        for part in relative.parts:
            current /= part
            if current.is_symlink():
                return False
        return path.exists()
    except ValueError:
        return False


def candidates(root: Path, reviews: bool = False) -> list[Path]:
    selected = []
    for target in (root / "target", root / "ipad/core/target"):
        if not safe_path(target, root):
            continue
        for child in target.iterdir():
            if child.name in {"debug", "release", "tmp", "windows-api-check", "translation-native", "translation-windows"} or re.fullmatch(
                r"(?:aarch64|x86_64)-(?:apple-(?:darwin|ios(?:-sim)?)|pc-windows-msvc)", child.name
            ):
                selected.append(child)
    selected += [root / "ipad/build"]
    # Dependency downloads in .cache/cargo remain reusable; do not touch Cargo
    # config/credentials or force network downloads on the next build.
    if reviews:
        selected.append(root / "target/ui-preview")
        selected += [root / "artifacts/translation-ct2" / n for n in ("build", "build-tools", "downloads")]
        for base in (root / "artifacts", root / "dist"):
            if not safe_path(base, root):
                continue
            for current, dirs, files in os.walk(base, followlinks=False):
                parent = Path(current)
                kept = []
                for name in dirs:
                    child = parent / name
                    if not safe_path(child, root):
                        continue
                    if child.suffix in {".app", ".xcresult"}:
                        selected.append(child)
                    else:
                        kept.append(name)
                dirs[:] = kept
                for name in files:
                    child = parent / name
                    # No generic ZIP deletion: users may keep documents/artwork
                    # archives in review folders. Only MyPDF distribution names.
                    if PACKAGE_NAME.fullmatch(name):
                        selected.append(child)
    result = []
    for path in sorted(set(selected), key=lambda p: (len(p.parts), str(p))):
        if safe_path(path, root) and not any(p in path.parents for p in result):
            result.append(path)
    return result


def state_inside(path: Path) -> Path | None:
    if not path.is_dir():
        return None
    for current, dirs, files in os.walk(path, followlinks=False):
        for name in dirs:
            if name.lower() in PROTECTED_DIRS or name.lower() in {"profile", "profiles"} or name.lower().endswith(("-profile", "_profile")):
                return Path(current) / name
        for name in files:
            if name.lower() in STATE_FILES or name.lower().endswith((".pdf", ".recovery", ".p12", ".pfx", ".pem", ".key", ".keychain-db")):
                return Path(current) / name
    return None


def assert_idle(root: Path) -> None:
    if (root / "dist/.release-lock").exists():
        raise RuntimeError("A release lock exists; finish or investigate that release before cleanup.")
    result = subprocess.run(["ps", "-axo", "pid=,comm="], text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError("Cannot inspect active builds; no files were removed. Run from a terminal with process access.")
    active = []
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) == 2 and Path(fields[1]).name in BUILDERS:
            active.append(line.strip())
    if active:
        raise RuntimeError("Stop active build tools before cleanup: " + "; ".join(active))


def tracked_files(root: Path) -> list[Path]:
    result = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], capture_output=True, check=True)
    return [root / os.fsdecode(p) for p in result.stdout.split(b"\0") if p]


def clean(root: Path, paths: list[Path], apply: bool) -> int:
    tracked = tracked_files(root)
    approved = []
    for path in paths:
        if not safe_path(path, root):
            raise RuntimeError(f"Unsafe or changed candidate: {path}")
        if any(p == path or path in p.parents for p in tracked):
            raise RuntimeError(f"Refusing to remove tracked source: {path}")
        state = state_inside(path)
        if state:
            print(f"KEEP {path.relative_to(root)} (contains protected state: {state.relative_to(root)})")
            continue
        approved.append((path, path.lstat()))
        print(f"{'REMOVE' if apply else 'WOULD REMOVE'} {path.relative_to(root)}")
    if apply:
        assert_idle(root)
        for path, before in approved:
            if not safe_path(path, root):
                raise RuntimeError(f"Candidate changed; stopped: {path}")
            after = path.lstat()
            if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                raise RuntimeError(f"Candidate replaced; stopped: {path}")
            if state_inside(path):
                raise RuntimeError(f"Protected state appeared after the preview; stopped: {path}")
            if stat.S_ISDIR(after.st_mode):
                shutil.rmtree(path)  # Does not follow symlinks within the tree.
            else:
                path.unlink()
    return len(approved)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Remove the previewed rebuildable output")
    parser.add_argument("--reviews", action="store_true", help="Also remove obsolete app bundles/packages and translation build scratch")
    args = parser.parse_args()
    try:
        count = clean(ROOT, candidates(ROOT, args.reviews), args.apply)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Cleanup stopped: {error}", file=sys.stderr)
        return 1
    print(f"{'Removed' if args.apply else 'Selected'} {count} outputs. Current release, rollback and user state are retained.")
    if not args.apply:
        print("Preview only. Add --apply to remove these outputs. Stop all builds first.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
