#!/bin/sh
# Compile the same native inference bridge on Mac for local regression testing.
# Normal Mac builds continue to use Apple's on-device framework.
set -eu
cd "$(dirname "$0")/.."
build_dir=${MYPDF_CT2_BUILD_DIR:-target/translation-native}
output_dir=${MYPDF_CT2_OUTPUT_DIR:-dist/translation/native}
cmake_cmd=${MYPDF_CMAKE:-cmake}
"$cmake_cmd" -S native/translation-ct2 -B "$build_dir" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PWD/$output_dir"
"$cmake_cmd" --build "$build_dir" --target mypdf_ct2 --parallel 4
"$cmake_cmd" --install "$build_dir" --component MyPDFTranslation
printf 'Native translation runtime: %s\n' "$PWD/$output_dir"
