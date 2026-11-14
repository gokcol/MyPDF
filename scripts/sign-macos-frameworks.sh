#!/bin/sh
# Sign nested Sparkle code inside-out with the same identity as MyPDF.
set -eu
app=$1
identity=$2
framework=$app/Contents/Frameworks/Sparkle.framework
[ -d "$framework" ] || exit 0
sign() {
    if [ "$identity" = - ]; then
        codesign --force --preserve-metadata=entitlements --sign - "$1"
    else
        codesign --force --options runtime --preserve-metadata=entitlements --timestamp --sign "$identity" "$1"
    fi
}
sign "$framework/Versions/B/XPCServices/Downloader.xpc"
sign "$framework/Versions/B/XPCServices/Installer.xpc"
sign "$framework/Versions/B/Autoupdate"
sign "$framework/Versions/B/Updater.app"
sign "$framework"
