# What Happens?

> GENERATED FILE: do not edit by hand. Every scenario is a file in `scenarios/`; add one with `python issues/issues.py wh add`.

> This is a repo way of asking what might happen if a certain behavior, action or event in this software is done or happens. What other expected or unexpected behaviors might be triggered. It does not answer what the triggered behaviors are; it lists the questions for what can trigger them. This lets users or devs reproduce and test on their own side, without a known reference behavior, so they can shape and report unexpected behaviors.

**How to use:** pick an `untested` scenario, try it on your devices, then report what you saw. If something unexpected happened, report it with `python issues/issues.py report` and link it (`wh set ID issue --issue ID`).

**16 scenarios**: 16 untested, 0 tested, 0 issue

## pairing

- <a id="wh-56106f835d88"></a>**WH-002** `untested`: When A forgets B while B is offline and A later pairs with B again, what happens on each device?
  - note: B still holds the old trust record. Compare with B initiating the connection instead.
  - issues: [45257a7](items/45257a7ff1be-trust-revocation-is-local-only.md)
- <a id="wh-e8775bf2dd84"></a>**WH-003** `untested`: When A and B both press Pair on each other at the same time, what happens?
- <a id="wh-8656491cd35e"></a>**WH-004** `untested`: When a pairing completes on Android while the Scan list is still refreshing, what do the paired and available lists show?
  - note: A device can briefly be in both lists. Watch for crashes or a duplicated row.
  - issues: [c17420e](items/c17420eb5067-android-crashes-after-pairing-completes.md)
- <a id="wh-69c59cea58db"></a>**WH-005** `untested`: When the app is killed or backgrounded while the pairing code is on screen, what state is each device in after relaunch?

## connect

- <a id="wh-4f925ee9dc48"></a>**WH-001** `untested`: When device A and device B try to connect to each other at the same moment, what happens?
  - note: Both dial; arbitration decides whose challenge wins (control code 4008 CONNECT_IN_PROGRESS).
  - issues: [7f716d6](items/7f716d6727c7-auto-connect-overlap-between-two-trusted-devices.md)
- <a id="wh-d485d2ee30e0"></a>**WH-013** `untested`: When more devices try to connect than the connection limit allows, what happens to the extra ones and to the existing ones?

## transfer

- <a id="wh-3d0f0e622d35"></a>**WH-006** `untested`: When A disconnects while B has a download in progress and A reconnects, what happens to the download?
  - note: Download should be marked interrupted, then resume on reconnect.
  - issues: [a6f28d4](items/a6f28d450f5c-reconnect-from-the-disconnecting-device-can.md)
- <a id="wh-dcf30e3795af"></a>**WH-007** `untested`: When A forgets B in the middle of a file transfer, what happens to the transfer on both sides?
- <a id="wh-9c0f85c3bad8"></a>**WH-008** `untested`: When a file offer expires while its download is already running, what happens?
- <a id="wh-c45d0b9c5625"></a>**WH-009** `untested`: When a download stalls and a new clipboard item arrives meanwhile, is the clipboard item delivered and does the download recover?

## discovery

- <a id="wh-b144b8359984"></a>**WH-012** `untested`: When the default WebSocket port is already in use on one device, can the other device still discover and connect to it?
  - note: Related to the effective WebSocket port fallback.

## protocol

- <a id="wh-1c1e7b279b20"></a>**WH-010** `untested`: When a peer announces a proto version range this build does not support, what do both devices show?
  - note: Use the emulator: --proto-versions 2-2. Expect control code 4009 and a message on the peer.
- <a id="wh-62d000fc47dd"></a>**WH-011** `untested`: When a legacy build (no hello) pairs and connects with this build, which features are available?

## platform

- <a id="wh-90af11f210dc"></a>**WH-014** `untested`: When a connected device switches network (Wi-Fi to hotspot or the reverse), how long until each side shows the right state?
- <a id="wh-3871ad9f6195"></a>**WH-015** `untested`: When two devices share the same hostname, can the user tell them apart in the lists?
- <a id="wh-48b109ba37d1"></a>**WH-016** `untested`: When the desktop app is quit from the tray menu while a device is connected or a transfer is running, what does the terminal show and what do the peers see?
  - note: Issue 0003 says the 0002 fix may be incomplete. Also try quitting from the taskbar and during pairing.
  - issues: [6c2cef3](items/6c2cef33b661-tray-quit-prints-a-webview2-teardown-error.md), [36a8ea7](items/36a8ea76e895-issue-0002-may-not-be-fixed-completely.md)
