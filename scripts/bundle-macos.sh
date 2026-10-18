#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "This script packages a macOS app. Build the mypdf binary directly on Windows." >&2
    exit 1
fi
profile=release
if [ "${1:-}" = --debug ]; then
    profile=debug
    cargo build --locked --bin mypdf --example app_icon
else
    cargo build --locked --release --bin mypdf --example app_icon
fi
# Build into a fresh bundle so Finder does not inherit an old directory/icon
# cache timestamp when the packaged app replaces an installed version.
app=dist/MyPDF-staging.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/mypdf" "$app/Contents/MacOS/.MyPDF.new"
mv "$app/Contents/MacOS/.MyPDF.new" "$app/Contents/MacOS/MyPDF"
cp packaging/Info.plist "$app/Contents/Info.plist"
mkdir -p dist/MyPDF.iconset
"target/$profile/examples/app_icon" dist/MyPDF.png
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" dist/MyPDF.png --out "dist/MyPDF.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" dist/MyPDF.png --out "dist/MyPDF.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns dist/MyPDF.iconset -o "$app/Contents/Resources/MyPDF-calico-v2.icns"
touch "$app"
codesign --force --sign - "$app"
rm -rf dist/MyPDF.app
mv "$app" dist/MyPDF.app
app=dist/MyPDF.app
echo "Built $PWD/$app"
