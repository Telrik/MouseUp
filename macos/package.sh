#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release --manifest-path macos/Cargo.toml --target aarch64-apple-darwin
cargo build --locked --release --manifest-path macos/Cargo.toml --target x86_64-apple-darwin

mkdir -p macos/dist
stage=$(mktemp -d "${TMPDIR:-/tmp}/mouseup-macos.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
app="$stage/MouseUp.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$stage/MouseUp.iconset"
target="${CARGO_TARGET_DIR:-$root/macos/target}"
lipo -create "$target/aarch64-apple-darwin/release/mouseup-macos" "$target/x86_64-apple-darwin/release/mouseup-macos" -output "$app/Contents/MacOS/MouseUp"
chmod 755 "$app/Contents/MacOS/MouseUp"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/brand.png --out "$stage/MouseUp.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/brand.png --out "$stage/MouseUp.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$stage/MouseUp.iconset" -o "$app/Contents/Resources/MouseUp.icns"
cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>MouseUp</string>
  <key>CFBundleDisplayName</key><string>MouseUp</string>
  <key>CFBundleIdentifier</key><string>com.github.telrik.mouseup</string>
  <key>CFBundleExecutable</key><string>MouseUp</string>
  <key>CFBundleIconFile</key><string>MouseUp.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.2.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
codesign --force --deep --sign - --timestamp=none "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" "$root/macos/dist/MouseUp-macOS-beta.zip"
printf 'Built %s\n' "$root/macos/dist/MouseUp-macOS-beta.zip"
