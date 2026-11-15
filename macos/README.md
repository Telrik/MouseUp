# MouseUp for macOS — experimental beta

This is an early, untested macOS build for Apple silicon and Intel. It provides the 16 cursor shapes, 12 colors, per-application choices, keyboard shortcuts and a menu-bar icon. It uses a click-through overlay and does **not** suppress the macOS cursor: the system pointer and game-drawn cursors can remain visible underneath. Overlay placement, high-DPI and fullscreen compatibility have not been verified on a Mac.

Download `MouseUp-macOS-beta.zip` from the [macOS beta release](https://github.com/Telrik/MouseUp/releases/tag/macos-v0.2.0-beta.1), extract it and move `MouseUp.app` to Applications. This beta is ad-hoc signed, not signed with an Apple Developer ID or notarized; Gatekeeper may prevent it from opening. Do not disable system security settings to run it.

Shortcuts use Control + Option + `1` / `2` to change the active application's cursor, Control + Option + `D` to restore it, and Control + Option + `Y` to pause or resume. The menu-bar icon provides Open and Exit. Some shortcuts may be reserved by macOS or other apps.

## Build

Install Xcode command-line tools and Rust stable on macOS, then:

```sh
git clone https://github.com/Telrik/MouseUp.git
cd MouseUp
sh macos/package.sh
```

The universal application archive is written to `macos/dist`. The embedded cursor artwork originates from YoloMouse; its redistribution rights have not been established. See [`assets/yolomouse/NOTICE.txt`](../assets/yolomouse/NOTICE.txt).
