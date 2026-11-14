# Cursor investigation

MouseUp is an independent implementation. Its source code is MIT licensed; the original YoloMouse cursor artwork and binaries are not included in the repository. Local builds use 16 original cursor shapes extracted from a supplied MSI.

## Original behavior studied

REA 6.9.0 and Ghidra 12.2 were used to inspect the locally supplied YoloMouse 1.25 installer. The 32-bit injector records thread-specific `WH_GETMESSAGE` hooks and enumerates windows for a process. The original hook DLL intercepts several Windows cursor APIs and tracks cursor state per thread. The main application can replace system cursors with `SetSystemCursor` and restore the system scheme with `SPI_SETCURSORS`. Static analysis alone does not establish every execution path or complete behavioral equivalence.

## MouseUp implementation

MouseUp uses architecture-matched helpers and a window-thread hook. It independently intercepts `SetCursor` and `GetCursor`, keeping the application's requested value separate from the displayed cursor. When the hidden-cursor option is enabled, a null `SetCursor` request may still display the replacement. For the Windows normal arrow, a helper replaces the system cursor and retains a copy for restoration. Resize cursor types are left alone.

The application restores the normal arrow when it exits or its controller pipe closes; a second copy lets the controller recover if the system-cursor helper fails. The window-hook helper requests detour removal before unhooking. The DLL stays pinned until the target process exits so an interrupted cleanup cannot leave a jump to unloaded code; without the shared session settings, detours forward calls to Windows.

An experiment using `WH_GETMESSAGE` alone passed direct cursor calls but failed to replace the cursor set by the Windows default window procedure. MouseUp therefore retains its existing message hook. Original `ShowCursor`, window-class cursor and clipping behavior are not fully reproduced.

## Validation and limits

Cursor loading and previews were tested for all 16 original shapes and their color variants. Controlled x86 and x64 processes tested API interception, logical cursor state, repeated attachment, restoration, and helper termination. System-cursor tests compared rendered pixels after normal exit, helper failure, and controller termination. The user confirmed stable typing in Codex, and a previous version was exercised in Factorio menus.

Game-drawn cursors may remain visible underneath the overlay; exclusive fullscreen, anti-cheat or privilege boundaries may prevent replacement. Not all games or window types have been tested. A Windows session can reject new hook DLL paths with error 126; a restart may clear the condition, but its cause is not established for every machine.
