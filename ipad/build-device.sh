#!/bin/sh
# Build the standalone native iPad development app. No web view or desktop process.
set -eu
cd "$(dirname "$0")"
: "${MYPDF_IOS_TEAM:?Set MYPDF_IOS_TEAM to your Apple development team ID}"
command -v xcodegen >/dev/null || { echo 'Install XcodeGen first: brew install xcodegen' >&2; exit 1; }
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cargo build --locked --manifest-path core/Cargo.toml --release --target aarch64-apple-ios
xcodegen generate --spec project.yml
xcodebuild -project MyPDFiPad.xcodeproj -scheme MyPDFiPad \
  -destination 'generic/platform=iOS' -derivedDataPath build \
  DEVELOPMENT_TEAM="$MYPDF_IOS_TEAM" -allowProvisioningUpdates
printf '\nBuilt %s/build/Build/Products/Debug-iphoneos/MyPDFiPad.app\n' "$PWD"
