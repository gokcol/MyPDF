#!/bin/sh
# Pinned official Sparkle distribution. Cached locally, never vendored in Git.
set -eu
cd "$(dirname "$0")/.."
version=2.10.1
digest=1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27
archive=.cache/Sparkle-$version.tar.xz
root=.cache/sparkle-$version
if [ ! -f "$root/.verified-$digest" ]; then
    mkdir -p .cache
    if [ ! -f "$archive" ]; then
        curl --fail --location --proto '=https' --tlsv1.2 \
            "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" -o "$archive.part"
        mv "$archive.part" "$archive"
    fi
    actual=$(shasum -a 256 "$archive" | cut -d ' ' -f 1)
    [ "$actual" = "$digest" ] || { echo 'Sparkle checksum mismatch' >&2; exit 1; }
    mkdir -p "$root"
    tar -xf "$archive" -C "$root"
    touch "$root/.verified-$digest"
fi
printf '%s\n' "$PWD/$root"
