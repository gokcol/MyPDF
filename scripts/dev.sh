#!/bin/sh
# Run MyPDF from source for development.
#
#   sh scripts/dev.sh [options] [file.pdf ...]
#
#   --release         Optimized build: smoother rendering, slower first compile
#   --fresh           Start with an empty, throwaway profile
#   --real-profile    Use your normal MyPDF data (recents, library, recovery)
#   --preview[=A,B]   Render headless UI screenshots instead of opening a window:
#                     every screen, or only the named ones (--preview=Home,Confirmation)
#   -h, --help        Show this help
#
# By default, development runs keep recents, saved artwork and recovery copies
# in target/dev-profile, separate from the installed app's data.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)

release=
data_dir="$root/target/dev-profile"
preview=
while [ $# -gt 0 ]; do
    case "$1" in
        --release) release=--release ;;
        --fresh) data_dir=$(mktemp -d "${TMPDIR:-/tmp}/mypdf-dev.XXXXXX") ;;
        --real-profile) data_dir= ;;
        --preview) preview=all ;;
        --preview=*) preview=${1#--preview=} ;;
        -h | --help)
            sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "Unknown option: $1 (see --help)" >&2
            exit 2
            ;;
        *) break ;;
    esac
    shift
done

# --manifest-path keeps PDF paths relative to where you ran the script.
if [ -n "$preview" ]; then
    out="$root/target/ui-preview"
    if [ "$preview" != all ]; then
        export MYPDF_PREVIEW_ONLY="$preview"
    fi
    MYPDF_PREVIEW_DIR="$out" cargo test --manifest-path "$root/Cargo.toml" --locked \
        --bin mypdf native_visual_preview -- --ignored
    echo "Screenshots: $out"
    exit 0
fi

if [ -n "$data_dir" ]; then
    mkdir -p "$data_dir"
    export MYPDF_DATA_DIR="$data_dir"
    echo "Profile: $data_dir"
fi
# $release is intentionally unquoted so an empty value adds no argument.
exec cargo run --manifest-path "$root/Cargo.toml" --locked $release --bin mypdf -- "$@"
