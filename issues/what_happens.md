# What Happens?

> GENERATED FILE: do not edit by hand. Edit `what_happens.toml` or run `python issues/issues.py wh add`, then `python issues/issues.py wh render`.

> This is a repo way of asking what might happen if a certain behavior, action or event in this software is done or happens. What other expected or unexpected behaviors might be triggered. It does not answer what the triggered behaviors are; it lists the questions for what can trigger them. This lets users or devs reproduce and test on their own side, without a known reference behavior, so they can shape and report unexpected behaviors.

**How to use:** pick an `untested` scenario, try it on your devices, then report what you saw. If something unexpected happened, report it with `python issues/issues.py report` and link it here (`wh set WH-00N issue --issue NNNN`).

**16 scenarios**: 16 untested, 0 tested, 0 issue

## pairing

- **WH-002** `untested`: When A forgets B while B is offline and A later pairs with B again, what happens on each device?
  - note: B still holds the old trust record. Compare with B initiating the connection instead.
  - issues: [0004](issue.md#0004)
- **WH-003** `untested`: When A and B both press Pair on each other at the same time, what happens?
- **WH-004** `untested`: When a pairing completes on Android while the Scan list is still refreshing, what do the paired and available lists show?
  - note: A device can briefly be in both lists. Watch for crashes or a duplicated row.
  - issues: [0006](issue.md#0006)
- **WH-005** `untested`: When the app is killed or backgrounded while the pairing code is on screen, what state is each device in after relaunch?

## connect

- **WH-001** `untested`: When device A and device B try to connect to each other at the same moment, what happens?
  - note: Both dial; arbitration decides whose challenge wins (control code 4008 CONNECT_IN_PROGRESS).
  - issues: [0001](issue.md#0001)
- **WH-013** `untested`: When more devices try to connect than the connection limit allows, what happens to the extra ones and to the existing ones?

## transfer

- **WH-006** `untested`: When A disconnects while B has a download in progress and A reconnects, what happens to the download?
  - note: Download should be marked interrupted, then resume on reconnect.
  - issues: [0005](issue.md#0005)
- **WH-007** `untested`: When A forgets B in the middle of a file transfer, what happens to the transfer on both sides?
- **WH-008** `untested`: When a file offer expires while its download is already running, what happens?
- **WH-009** `untested`: When a download stalls and a new clipboard item arrives meanwhile, is the clipboard item delivered and does the download recover?

## discovery

- **WH-012** `untested`: When the default WebSocket port is already in use on one device, can the other device still discover and connect to it?
  - note: Related to the effective WebSocket port fallback.

## protocol

- **WH-010** `untested`: When a peer announces a proto version range this build does not support, what do both devices show?
  - note: Use the emulator: --proto-versions 2-2. Expect control code 4009 and a message on the peer.
- **WH-011** `untested`: When a legacy build (no hello) pairs and connects with this build, which features are available?

## platform

- **WH-014** `untested`: When a connected device switches network (Wi-Fi to hotspot or the reverse), how long until each side shows the right state?
- **WH-015** `untested`: When two devices share the same hostname, can the user tell them apart in the lists?
- **WH-016** `untested`: When the desktop app is quit from the tray menu while a device is connected or a transfer is running, what does the terminal show and what do the peers see?
  - note: Issue 0003 says the 0002 fix may be incomplete. Also try quitting from the taskbar and during pairing.
  - issues: [0002](issue.md#0002), [0003](issue.md#0002)

## Issue coverage

| issue | status | scenarios |
|-------|--------|-----------|
| [0001](issue.md#0001) | fixed | WH-001 |
| [0002](issue.md#0002) | fixed | WH-016 |
| [0003](issue.md#0002) (see 0002) | fixed | WH-016 |
| [0004](issue.md#0004) | report(weak) | WH-002 |
| [0005](issue.md#0005) | fixed | WH-006 |
| [0006](issue.md#0006) | fixed | WH-004 |
