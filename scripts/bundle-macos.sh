#!/bin/sh
# Build a native bundle, or pass --target aarch64-apple-darwin for release.
# --output selects the destination bundle; --debug keeps development builds.
# --review (also --no-document-handlers) excludes Finder document handlers.
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "This script packages a macOS app. Build the mypdf binary directly on Windows." >&2
    exit 1
fi
profile=release
target=${CARGO_BUILD_TARGET:-}
output=dist/MyPDF.app
document_handlers=true
while [ $# -gt 0 ]; do
    case "$1" in
        --debug) profile=debug; shift ;;
        --review|--no-document-handlers) document_handlers=false; shift ;;
        --target|--output)
            [ $# -ge 2 ] || { echo "$1 requires a value" >&2; exit 1; }
            case "$1" in --target) target=$2 ;; --output) output=$2 ;; esac
            shift 2
            ;;
        *) echo "Usage: $0 [--debug] [--review|--no-document-handlers] [--target <macOS triple>] [--output <path.app>]" >&2; exit 1 ;;
    esac
done
case "$target" in
    ''|aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) echo "Unsupported macOS target: $target" >&2; exit 1 ;;
esac
case "$output" in *.app) ;; *) echo "Bundle output must end in .app" >&2; exit 1 ;; esac
# Review recipes often rename an optimized bundle without passing --debug.
# Exclude those names too, so opening a test app cannot add another PDF handler.
output_name=$(basename "$output" | LC_ALL=C tr '[:upper:]' '[:lower:]')
case "$profile:$output_name" in debug:*|*:*review.app) document_handlers=false ;; esac
host=$(rustc -vV | sed -n 's/^host: //p')
[ -n "$host" ] || { echo "Could not determine the Rust host target" >&2; exit 1; }
target_dir=${CARGO_TARGET_DIR:-target}
set -- build --locked
[ "$profile" = debug ] || set -- "$@" --release
[ -z "$target" ] || set -- "$@" --target "$target"
binary_dir=$target_dir/${target:+$target/}$profile
if [ -z "$target" ] || [ "$target" = "$host" ]; then
    # Build the app and runnable icon helper together on the release host.
    cargo "$@" --bin mypdf --example app_icon
    icon_tool=$binary_dir/examples/app_icon
else
    cargo "$@" --bin mypdf
    # Cross-compiled executables cannot run on an Intel packaging host.
    set -- build --locked --target "$host" --example app_icon
    [ "$profile" = debug ] || set -- "$@" --release
    cargo "$@"
    icon_tool=$target_dir/$host/$profile/examples/app_icon
fi
# Build into a fresh bundle so Finder does not inherit an old directory/icon
# cache timestamp when the packaged app replaces an installed version.
mkdir -p "$(dirname "$output")"
output=$(cd "$(dirname "$output")" && pwd)/$(basename "$output")
work=$(mktemp -d "$(dirname "$output")/.mypdf-bundle.XXXXXX")
cleanup() {
    status=$?
    trap - 0 HUP INT TERM
    if { [ -e "$work/previous.app" ] || [ -L "$work/previous.app" ]; } &&
        [ ! -e "$output" ] && [ ! -L "$output" ]; then
        if ! mv "$work/previous.app" "$output"; then
            echo "Could not restore the previous bundle; it remains in $work/previous.app" >&2
            exit 1
        fi
    fi
    rm -rf "$work"
    exit "$status"
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
app=$work/MyPDF.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary_dir/mypdf" "$app/Contents/MacOS/MyPDF"
cp packaging/Info.plist "$app/Contents/Info.plist"
if [ "$document_handlers" = false ]; then
    # Change only the fresh bundle, before codesign. Signed release bundles and
    # their rollback copies retain their original metadata and signatures.
    python3 - "$app/Contents/Info.plist" <<'PY'
from pathlib import Path
import plistlib
import sys

path = Path(sys.argv[1])
info = plistlib.loads(path.read_bytes())
info.pop("CFBundleDocumentTypes", None)
path.write_bytes(plistlib.dumps(info, sort_keys=False))
PY
fi
mkdir -p "$work/MyPDF.iconset"
"$icon_tool" "$work/MyPDF.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/MyPDF.png" --out "$work/MyPDF.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$work/MyPDF.png" --out "$work/MyPDF.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/MyPDF.iconset" -o "$app/Contents/Resources/MyPDF-calico-v2.icns"
touch "$app"
codesign --force --sign - "$app"
if [ -e "$output" ] || [ -L "$output" ]; then
    mv "$output" "$work/previous.app"
fi
mv "$app" "$output"
echo "Built $output"
