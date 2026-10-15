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
out=dist/release
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

# The bundle layout, icon and Info.plist come from the development script.
sh scripts/bundle-macos.sh
rm -rf "$out"
mkdir -p "$out"
ditto dist/MyPDF.app "$app"
# Apple Silicon only, whichever Mac builds it.
cargo build --locked --release --bin mypdf --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/mypdf "$app/Contents/MacOS/MyPDF"
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
echo "Built $PWD/$dmg"
