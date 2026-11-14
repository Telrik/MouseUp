# Building MouseUp

Windows x64, Rust stable, the Microsoft C++ build tools, the Windows SDK, and a compatible YoloMouse MSI are required. The installer supplies the original cursor artwork; it is not included in this repository.

```powershell
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
powershell -ExecutionPolicy Bypass -File scripts\extract-cursors.ps1 -MsiPath C:\path\to\YoloMouse.msi
cargo test --workspace
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

The result is `dist-single\mouseup.exe`. Windows loads architecture-specific hook components from files, so the EXE extracts its embedded components and cursor assets into `%APPDATA%\MouseUp\bundles` at launch. Cursor settings are stored in `%APPDATA%\MouseUp`.

The extraction script copies the selected 16 shapes and their colors into `assets/yolomouse/Cursors`. Those third-party cursor files and the installer are excluded from version control. The resulting EXE contains that artwork, whose redistribution rights have not been established.

MouseUp replaces the normal Windows cursor and intercepts cursor calls in supported applications. It can also show an overlay when the Windows cursor is hidden. A game that draws its own cursor may still draw it underneath; exclusive fullscreen, anti-cheat, different privileges, and some applications may prevent replacement. MouseUp does not bypass anti-cheat. See [investigation notes](investigation.md) for tested behavior and current limitations.

MouseUp source is licensed under [MIT](../LICENSE); the original YoloMouse cursor artwork is not covered by this license. Dependencies have their own licenses; [third-party notices](../THIRD_PARTY_NOTICES.txt) include MinHook's license.
