# Building MouseUp

Windows x64, Git, Rust stable, the Microsoft C++ build tools and the Windows SDK are required. The selected cursor files are included in the repository; no YoloMouse installer is needed.

Open **x64 Native Tools Command Prompt for Visual Studio**, run `powershell`, and then run:

```powershell
git clone https://github.com/Telrik/MouseUp.git
cd MouseUp
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
cargo test --workspace
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

The result is `dist-single\mouseup.exe`. Windows loads architecture-specific hook components from files, so the EXE extracts its embedded components and cursor assets into `%APPDATA%\MouseUp\bundles` at launch. Cursor settings are stored in `%APPDATA%\MouseUp`.

The optional extraction script can verify or extract the selected 16 shapes and their colors from a compatible YoloMouse MSI into `assets/yolomouse/Cursors`. The installer is excluded from version control. The included third-party cursor files and resulting EXE contain artwork whose redistribution rights have not been established.

MouseUp replaces the normal Windows cursor and intercepts cursor calls in supported applications. It can also show an overlay when the Windows cursor is hidden. A game that draws its own cursor may still draw it underneath; exclusive fullscreen, anti-cheat, different privileges, and some applications may prevent replacement. MouseUp does not bypass anti-cheat. See [investigation notes](investigation.md) for tested behavior and current limitations.

MouseUp source is licensed under [MIT](../LICENSE); the original YoloMouse cursor artwork is not covered by this license. Dependencies have their own licenses; [third-party notices](../THIRD_PARTY_NOTICES.txt) include MinHook's license.
