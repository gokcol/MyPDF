#!/bin/sh
# Build MyPDF for Apple Silicon, sign it with a Developer ID, notarize and
# staple it, and wrap it in a signed, notarized disk image.
#
#   MYPDF_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   MYPDF_NOTARY_PROFILE=MyPDF sh scripts/release-macos.sh
#
# The notary profile is saved once with
#   xcrun notarytool store-credentials MyPDF --apple-id … --team-id …
# Without MYPDF_NOTARY_PROFILE the app and image are signed but not notarized.
# The identity may also be the certificate's SHA-1 fingerprint, from
#   security find-identity -v -p codesigning
# which avoids accented names that do not match as typed.
set -eu
cd "$(dirname "$0")/.."
identity=${MYPDF_SIGN_IDENTITY:?Set MYPDF_SIGN_IDENTITY to a Developer ID Application identity}
profile=${MYPDF_NOTARY_PROFILE:-}
version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" packaging/Info.plist)
mkdir -p dist
# Serialize promotion so concurrent release runs cannot swap each other's backups.
lock=dist/.release-lock
mkdir "$lock" 2>/dev/null || { echo "Another release is running ($lock exists)" >&2; exit 1; }
work=
promoting=false
had_release=false
cleanup() {
    status=$?
    trap - 0 HUP INT TERM
    restore_failed=false
    if [ "$promoting" = true ] && [ -d "$work/release" ]; then
        # The candidate was not published. Restore both existing generations.
        if [ "$had_release" = true ] && [ ! -e dist/release ] && [ -e dist/rollback ]; then
            mv dist/rollback dist/release || restore_failed=true
        fi
        if [ -e "$work/previous-rollback" ]; then
            if [ ! -e dist/rollback ]; then
                mv "$work/previous-rollback" dist/rollback || restore_failed=true
            else
                restore_failed=true
            fi
        fi
    fi
    if [ "$restore_failed" = true ]; then
        echo "Release promotion failed; preserved recovery files in $work and lock $lock" >&2
        exit 1
    elif [ -n "$work" ]; then
        rm -rf "$work"
    fi
    rmdir "$lock"
    exit "$status"
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
work=$(mktemp -d dist/.release-work.XXXXXX)
out=$work/release
app=$out/MyPDF.app
dmg=$out/MyPDF-$version.dmg

# Submit a file to Apple's notary service and fail unless it is accepted.
notarize() {
    result=$(xcrun notarytool submit "$1" --keychain-profile "$profile" --wait --output-format json)
    echo "$result"
    case $result in
    *'"status":"Accepted"'*) ;;
    *)
        id=$(printf '%s' "$result" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
        [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile "$profile" || true
        echo "Notarization of $1 was not accepted." >&2
        exit 1
        ;;
    esac
}

# A rollback can also carry an installation's settings, stamps and recovery.
# Archive just that data, deduplicated by content, before retiring its app.
preserve_rollback_data() {
    python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile

source = Path("dist/rollback")
names = ("user-data", "mypdf-user-data", "recovery", "user-data-sha256.json")
if source.is_symlink():
    raise RuntimeError("Refusing to archive data through a rollback symlink")

def inventory(root):
    entries = []
    def visit(path):
        mode = path.lstat().st_mode
        relative = path.relative_to(root).as_posix()
        if stat.S_ISDIR(mode):
            entries.append([relative, "directory"])
            for child in sorted(path.iterdir()):
                visit(child)
        elif stat.S_ISREG(mode):
            digest = hashlib.sha256()
            with path.open("rb") as handle:
                for block in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(block)
            entries.append([relative, "file", digest.hexdigest()])
        else:
            raise RuntimeError(f"Refusing to retire rollback data with a symlink or special file: {path}")
    for name in names:
        path = root / name
        if path.exists() or path.is_symlink():
            visit(path)
    return entries

entries = inventory(source)
if not entries:
    raise SystemExit(0)
manifest = json.dumps({"format": 1, "entries": entries}, sort_keys=True, indent=2) + "\n"
digest = hashlib.sha256(manifest.encode()).hexdigest()
preserved = Path("artifacts/preserved")
for parent in (preserved.parent, preserved):
    if parent.is_symlink():
        raise RuntimeError(f"Preservation directory must not be a symlink: {parent}")
preserved.mkdir(parents=True, exist_ok=True)
archive = preserved / f"rollback-data-{digest}"

def sync_tree(root):
    for path in root.rglob("*"):
        if path.is_symlink():
            raise RuntimeError(f"Refusing to follow an archive symlink: {path}")
        if path.is_file():
            with path.open("rb") as handle:
                os.fsync(handle.fileno())
    directories = [p for p in root.rglob("*") if p.is_dir()] + [root]
    for directory in directories:
        descriptor = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

if archive.exists() or archive.is_symlink():
    if archive.is_symlink() or (archive / "manifest.json").is_symlink() or inventory(archive) != entries or (archive / "manifest.json").read_text() != manifest:
        raise RuntimeError(f"Existing rollback data archive failed verification: {archive}")
    sync_tree(archive)
else:
    temporary = Path(tempfile.mkdtemp(prefix=".rollback-data-", dir=preserved))
    try:
        for name in names:
            path = source / name
            if path.is_symlink():
                raise RuntimeError(f"Rollback data became a symlink: {path}")
            if path.is_dir():
                shutil.copytree(path, temporary / name, symlinks=True)
            elif path.exists():
                shutil.copy2(path, temporary / name, follow_symlinks=False)
        if inventory(temporary) != entries or inventory(source) != entries:
            raise RuntimeError("Rollback data changed or its archive copy failed verification")
        (temporary / "manifest.json").write_text(manifest)
        sync_tree(temporary)
        temporary.rename(archive)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
for directory in (preserved, preserved.parent, Path(".")):
    descriptor = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
print(f"Preserved rollback user data: {archive}")
PY
}

# Compile Apple Silicon once and package directly into the unpublished release.
sh scripts/bundle-macos.sh --target aarch64-apple-darwin --output "$app"
codesign --force --options runtime --timestamp --sign "$identity" "$app"
codesign --verify --strict --verbose=2 "$app"

if [ -n "$profile" ]; then
    zip=$out/MyPDF-notarize.zip
    ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip"
    rm "$zip"
    xcrun stapler staple "$app"
    spctl --assess --type execute --verbose=2 "$app"
fi

stage=$out/stage
mkdir -p "$stage"
ditto "$app" "$stage/MyPDF.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "MyPDF $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
codesign --force --timestamp --sign "$identity" "$dmg"
if [ -n "$profile" ]; then
    notarize "$dmg"
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi
(cd "$out" && shasum -a 256 "$(basename "$dmg")" >"$(basename "$dmg").sha256")
# Archive failures stop before either existing release is renamed or removed.
if [ -e dist/release ] && [ -e dist/rollback ]; then
    preserve_rollback_data
fi
# Only a fully prepared release may replace the current release. Renames stay
# within dist; the trap restores the old release if publication cannot finish.
promoting=true
if [ -e dist/release ]; then
    had_release=true
    if [ -e dist/rollback ]; then
        mv dist/rollback "$work/previous-rollback"
    fi
    mv dist/release dist/rollback
fi
mv "$out" dist/release
echo "Built $PWD/dist/release/MyPDF-$version.dmg (previous release: dist/rollback)"
