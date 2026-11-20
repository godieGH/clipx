# Issues

> GENERATED FILE: do not edit by hand. Every issue is a file in `items/`; change one, then run `python issues/issues.py render`. How to use the tool: [README.md](README.md).

Developer-facing bug log, kept apart from the public GitHub issues (see [Why this exists](README.md#why-this-exists)). Fixed issues are never deleted, they are marked `fixed`.

**7 issues**: 2 report, 5 fixed

| id | status | issue | comment | scenarios |
|----|--------|-------|---------|-----------|
| [7f716d6](items/7f716d6727c7-auto-connect-overlap-between-two-trusted-devices.md) | fixed | **Auto-connect overlap between two trusted devices** (was 0001) | Auto-connect overlap resolved in the handshake (earlier fix: fingerprint tie-breaker, since replaced; see the update). | [WH-001](what_happens.md#wh-4f925ee9dc48) |
| [6c2cef3](items/6c2cef33b661-tray-quit-prints-a-webview2-teardown-error.md) | fixed | **Tray Quit prints a WebView2 teardown error** (was 0002) | IPC shutdown now runs at Tauri ExitRequested before WebView teardown. | [WH-016](what_happens.md#wh-48b109ba37d1) |
| [36a8ea7](items/36a8ea76e895-issue-0002-may-not-be-fixed-completely.md) | fixed | **Issue 0002 may not be fixed completely** (was 0003) | ↳ refers to [6c2cef3](items/6c2cef33b661-tray-quit-prints-a-webview2-teardown-error.md). re-report issue `0002` is not fixed correctly or it is fixed half — the issue need recheck carefully | [WH-016](what_happens.md#wh-48b109ba37d1) |
| [45257a7](items/45257a7ff1be-trust-revocation-is-local-only.md) | report(weak) | **Trust revocation is local-only** (was 0004) | Trust revocation is local-only. A peer retaining stale trust can reject a new pairing request, preventing re-pairing until its stale trust is also removed. | [WH-002](what_happens.md#wh-56106f835d88) |
| [a6f28d4](items/a6f28d450f5c-reconnect-from-the-disconnecting-device-can.md) | fixed | **Reconnect from the disconnecting device can stall in CONNECTING** (was 0005) | Reconnecting from the device that initiated a disconnect can stall because the device manager may reuse a stale transport connection ID. | [WH-006](what_happens.md#wh-3d0f0e622d35) |
| [c17420e](items/c17420eb5067-android-crashes-after-pairing-completes.md) | fixed | **Android crashes after pairing completes** (was 0006) | Android crashes after pairing completes | [WH-004](what_happens.md#wh-8656491cd35e) |
| [8f7e67b](items/8f7e67b56dcf-re-sending-an-already-received-file-creates.md) | report(medium) | **Re-sending an already received file creates duplicate copies** (was 0007) | Re-sending an already received file creates duplicate copies in internal storage. |  |
