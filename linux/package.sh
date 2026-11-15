#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
cargo build --locked --release --manifest-path linux/Cargo.toml

stage=$(mktemp -d "${TMPDIR:-/tmp}/mouseup-package.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
install -d "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/256x256/apps"
install -m 755 "${CARGO_TARGET_DIR:-$root/linux/target}/release/mouseup-linux" "$stage/usr/bin/mouseup"
install -m 644 assets/brand.png "$stage/usr/share/icons/hicolor/256x256/apps/mouseup.png"
cat > "$stage/DEBIAN/control" <<'EOF'
Package: mouseup
Version: 0.2.0~beta1
Section: utils
Priority: optional
Architecture: amd64
Maintainer: MouseUp contributors
Depends: libc6 (>= 2.39), libgcc-s1, libx11-6, libx11-xcb1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libxcursor1, libxi6, libxrandr2, libgl1, libegl1, libfontconfig1
Description: MouseUp experimental cursor utility for Ubuntu X11
 MouseUp is an independent open-source cursor utility. This beta is
 intended for X11 sessions on Ubuntu 24.04 and newer.
EOF
cat > "$stage/usr/share/applications/mouseup.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=MouseUp
Comment=Custom cursors for X11 applications
Exec=mouseup
Icon=mouseup
Terminal=false
Categories=Utility;
StartupWMClass=mouseup-linux
EOF
dpkg-deb --build --root-owner-group "$stage" "$root/mouseup-ubuntu-x11-0.2.0-beta1-amd64.deb"
