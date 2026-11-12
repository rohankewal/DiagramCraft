#!/bin/sh
# Wraps the release binary in a macOS application bundle.
#
# The bare binary runs perfectly well and sets its own dock icon, so this is
# only needed to get a double-clickable DiagramCraft.app with the icon shown
# in Finder. Run it from anywhere; paths are resolved against the repository.
#
#     scripts/bundle-macos.sh [OUTPUT_DIR]
#
# The bundle is unsigned. macOS will refuse to open it on another machine
# until it is signed and notarised, or until the user clears the quarantine
# attribute themselves.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out=${1:-"$root/target"}
app="$out/DiagramCraft.app"
binary="$root/target/release/diagramcraft"
icon="$root/assets/DiagramCraft.icns"

if [ ! -x "$binary" ]; then
    echo "no release binary at $binary" >&2
    echo "build one first:  cargo build --release" >&2
    exit 1
fi

if [ ! -f "$icon" ]; then
    echo "no icon at $icon" >&2
    echo "regenerate one:  scripts/make-icons.sh" >&2
    exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/DiagramCraft"
cp "$icon" "$app/Contents/Resources/DiagramCraft.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>                  <string>DiagramCraft</string>
    <key>CFBundleDisplayName</key>           <string>DiagramCraft</string>
    <key>CFBundleIdentifier</key>            <string>co.diagramcraft.DiagramCraft</string>
    <key>CFBundleVersion</key>               <string>$version</string>
    <key>CFBundleShortVersionString</key>    <string>$version</string>
    <key>CFBundleExecutable</key>            <string>DiagramCraft</string>
    <key>CFBundleIconFile</key>              <string>DiagramCraft</string>
    <key>CFBundlePackageType</key>           <string>APPL</string>
    <key>LSMinimumSystemVersion</key>        <string>10.15</string>
    <key>NSHighResolutionCapable</key>       <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>      <string>Lucid Standard Import</string>
            <key>CFBundleTypeRole</key>      <string>Editor</string>
            <key>CFBundleTypeExtensions</key><array><string>lucid</string></array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key>      <string>Visio Drawing</string>
            <key>CFBundleTypeRole</key>      <string>Viewer</string>
            <key>CFBundleTypeExtensions</key><array><string>vsdx</string><string>vsdm</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Finder caches bundle icons aggressively; touching the bundle asks it to
# look again rather than keep showing the generic one.
touch "$app"

echo "wrote $app"
