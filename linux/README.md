# MouseUp for Ubuntu X11 — beta

This experimental build targets Ubuntu 24.04 x86-64 in an **X11 session**. Wayland is not supported. It embeds the same 16 cursor shapes and 12 colors as the Windows version, supports per-application hotkeys, and can use a StatusNotifier tray when the desktop provides one. Without a tray, closing the window exits the application.

The X11 cursor is hidden while a click-through overlay displays the selected image. The original cursor is restored on exit or pause. Resize cursors are left unchanged where X11 identifies them. Some games draw their own cursor inside their image; it can remain visible beneath the overlay. Full-screen, compositors, multi-monitor setups, anti-cheat software and other applications may behave differently. This is an early beta and compatibility is not guaranteed.

Download `mouseup-ubuntu-x11-0.2.0-beta1-amd64.deb` from the [Ubuntu beta release](https://github.com/Telrik/MouseUp/releases/tag/ubuntu-x11-v0.2.0-beta.1), then install it using Ubuntu's package installer or:

```sh
sudo apt install ./mouseup-ubuntu-x11-0.2.0-beta1-amd64.deb
```

Start **MouseUp** from the applications menu. Use `Ctrl+Alt+1` / `Ctrl+Alt+2` to change the current application's cursor, `Ctrl+Alt+D` to restore it, and `Ctrl+Alt+Y` to pause or resume.

## Build

On Ubuntu 24.04 x86-64, install Rust stable, Git, the C compiler, `pkg-config`, and the X11/OpenGL development libraries. Then:

```sh
git clone https://github.com/Telrik/MouseUp.git
cd MouseUp
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxfixes-dev libxkbcommon-dev libxkbcommon-x11-dev libgl1-mesa-dev
sh linux/package.sh
```

The resulting Debian package is written to the repository root. The selected third-party cursor artwork is embedded in the executable; see [`assets/yolomouse/NOTICE.txt`](../assets/yolomouse/NOTICE.txt).
