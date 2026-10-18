#!/bin/sh
# No document content or model downloads. Requires a logged-in macOS 26+ session.
set -eu
cd "$(dirname "$0")/../.."
smoke_output=$(mktemp -d "${TMPDIR:-/tmp}/mypdf-translation-smoke.XXXXXX")
trap 'rm -rf "$smoke_output"' EXIT
xcrun swiftc -parse-as-library -O -target "$(uname -m)-apple-macosx12.0" \
  -module-cache-path "$smoke_output/module-cache" \
  native/translation/TranslationBridge.swift native/translation/Smoke.swift \
  -o "$smoke_output/translation-smoke"
"$smoke_output/translation-smoke" "$@"
