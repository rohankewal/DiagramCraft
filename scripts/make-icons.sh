#!/bin/sh
# Regenerates every icon file from the one drawing in `dc-app`'s icon module.
#
# Run this after changing that module, or the icon shipped in `assets/` and
# built into the binary will no longer match what the code draws. A test
# checks for exactly that drift.
#
#     scripts/make-icons.sh
#
# `iconutil` is macOS-only, so the `.icns` step is skipped elsewhere. The
# PNGs are produced on any platform.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
assets="$root/assets"
dc="$root/target/release/diagramcraft"

if [ ! -x "$dc" ]; then
    echo "no release binary at $dc" >&2
    echo "build one first:  cargo build --release" >&2
    exit 1
fi

mkdir -p "$assets"

# The master, used in the README, and the sizes a window icon wants.
"$dc" icon "$assets/icon.png" 1024
for size in 512 256 128 64 32 16; do
    "$dc" icon "$assets/icon-$size.png" "$size"
done

if ! command -v iconutil >/dev/null 2>&1; then
    echo "iconutil not found; skipping the .icns"
    exit 0
fi

# iconutil wants this exact naming: each size paired with its retina partner.
set=$(mktemp -d)/DiagramCraft.iconset
mkdir -p "$set"
for pair in 16:16x16 32:16x16@2x 32:32x32 64:32x32@2x \
            128:128x128 256:128x128@2x 256:256x256 512:256x256@2x \
            512:512x512 1024:512x512@2x; do
    px=${pair%%:*}
    name=${pair##*:}
    "$dc" icon "$set/icon_$name.png" "$px" >/dev/null
done

iconutil -c icns "$set" -o "$assets/DiagramCraft.icns"
rm -rf "$set"

echo "wrote $assets/DiagramCraft.icns"
