---
id: 6c2cef33b6616a6cc55b8f9dc64ab6a37626f51e
legacy: 0002
title: Tray Quit prints a WebView2 teardown error
status: fixed
comment: IPC shutdown now runs at Tauri ExitRequested before WebView teardown.
---

**Observation:** When the desktop app is opened they're two or three ways to stop it or shut-it-down. the close button on the reactUI is only used to hide the app to a tray. to close/quit/shutdown the app one has to either close it through taskbar-close or through the tray menus where they can click `Quit`. the bug is shown in that second option(the tray `Quit`). it closes/shutdown the app but something is failing so the terminals show an error.
```
    [0909/155249.920:ERROR:ui\gfx\win\window_impl.cc:172] Failed to unregister class Chrome_WidgetWin_0. Error = 1412
```
Fixed in the application lifecycle by shutting down the IPC client on Tauri `ExitRequested`, before WebView teardown. If the Chromium/WebView2 diagnostic still appears on a particular Windows runtime after this change, it should be treated as an upstream WebView2 teardown diagnostic rather than a reason to move IPC cleanup later.

**Note:** This bug is not solved fixed yet it still shows — remove this note when fixed completely
the terminal still shows this
```
[0910/013145.061:ERROR:ui\gfx\win\window_impl.cc:172] Failed to unregister class Chrome_WidgetWin_0. Error = 1412
```
*If it is something to be handled/fixed on code it has to be fixed right a way.*
