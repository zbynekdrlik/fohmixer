---
paths:
  - "crates/fohmixer-hub/src/plugwin/**"
  - "crates/fohmixer-hub/src/plugwin.rs"
  - "crates/fohmixer-hub/src/router/plugwin*.rs"
  - "crates/fohmixer-ui/src/components/eq*.rs"
  - "tools/plugwin-probe/**"
---

# A plug-in's editor window on the surface (#71, D17, F28)

What the PC tests of 2026-10-09 established (details and evidence on #71). Read this before touching the Pro-Q 4 screen.

## Never in a running Live first

- **Anything that sends input to a plug-in window is proven against the plug-in in a separate host process before it runs in a running Live.** A posted drag into Pro-Q 4.02 inside the band instance was followed by `is_editor_open = false`, and Live crashed: `FabFilter Pro-Q 4.vst3`, `0xc0000409`, Exception Data 7 (`abort`). The lead is FabFilter's own fix of "a crash when closing the interface with a text editor open". Two quick presses on a value field read as a double-click and open that text field. The owner postponed the FabFilter update (#71), so the hub must never close an editor while a text field may be open.
- **The isolated host:** Carla 2.5.10 win64, its bridge alone, on the dummy engine. No audio driver is loaded; check that the process has no `asio`, `audioses`, `mmdevapi` or `wdmaud` module.
  - It lives in the PC's probe folder.
  - Command: `$env:CARLA_BRIDGE_DUMMY = '1'` (exactly `1`), then `Start-Process <…>\Carla\carla-bridge-native.exe -ArgumentList @('vst3', '"<…>\FabFilter\FabFilter Pro-Q 4.vst3"', '""') -PassThru`.
  - Use `Start-Process`, which detaches. `Process.Start` with handles inherited keeps the MCP shell waiting until it times out.
  - The label is `""`; `"(none)"` fails ("Failed to get plugin description").
  - The editor is a top-level `Pro-Q 4 (GUI)` of class `JUCE_…`.
  - Closing that window ends the bridge with `0xC0000005` whatever the plug-in's state, so this host cannot reproduce the close crash.
- **Every injection checks the target first:** the point's window (`WindowFromPoint` → `GetAncestor(GA_ROOT)` → owner PID) must belong to the host meant, never to Live by accident.

## What Live gives

- **Opening and closing:** `PluginDevice.is_editor_open` (get / set / observe, Live 12.4.3+) opens and closes a plug-in's editor. It reads on the PC's 12.4.6 through the FohMixer script's generic `get_prop` / `set_prop`, for a device on a track and one inside a rack chain. Two editors can be open at once in one instance.
- **Identity:** `class_display_name` is the plug-in's product name (`Pro-Q 4`). It holds even when the device is renamed; `name` is the device's name.
- **The window:** a top-level `Vst3PlugWindow` titled `<device>/<track>`. The title holds a track name, so never log or print it. Pro-Q's own UI is one child, `FF_UIWindow`, at (8, 31), 1349×809 at 100 %. Find the new window by comparing the window list before and after the open; Live does not make it the foreground window.
- **Parameters:** Live exposes only one Pro-Q 4 parameter (`Device On`) unless the owner configures them in Live, so the EQ is driven through the window, not the parameters.

## Capture

- `PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT)` gives the whole editor even behind other windows, in about 17 ms per 1365×848 frame. It renders on the window's thread, which is likely Live's, so it is the fallback only.
- **Windows Graphics Capture** (the `windows-capture` crate, MIT) reads the DWM surface instead.
  - On the PC's Windows 10 19044 its yellow border cannot be switched off (needs 20348).
  - A fully off-screen or minimised window gives no frames.
- JPEG q75 of a frame: about 5 ms and 62 KB (System.Drawing), so about 2 MB/s at 30 fps.

## Input

- **Posted mouse messages** to `FF_UIWindow` give hover (`WM_MOUSEMOVE`), clicks (`WM_LBUTTONDOWN` / `UP`) and the wheel (`WM_MOUSEWHEEL`, screen coordinates). They give **no drag**, because FabFilter reads the real cursor and button state.
- **A real drag is injected touch** (the owner's decision on #71):
  - Use `InitializeTouchInjection` / `InjectTouchInput`, or the synthetic-pointer API: DOWN `INRANGE|INCONTACT|DOWN`, UPDATE `INRANGE|INCONTACT|UPDATE` every ~16 ms (re-send the last point at least every 100 ms while the finger rests), and UP `UP` at exactly the last point.
  - **Never set `POINTER_FLAG_PRIMARY`**: every call then fails with `ERROR_INVALID_PARAMETER` (87).
  - `POINTER_TOUCH_INFO` is 144 bytes, `POINTER_INFO` 96.
  - The editor must be the window at that screen point.
  - The system cursor follows the contact; put it back with `SetCursorPos` after the gesture.
  - A real-mouse drag (`SendInput` / `mouse_event`) works too.
- **On Pro-Q 4:**
  - A single tap on the curve creates nothing; a double-click creates a band.
  - A touch drag that starts on a band moves it.
  - **A double tap on a band opens its frequency text field**: the close-crash state.
