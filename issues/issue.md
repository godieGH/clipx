# issues
This document will be used for listing/tracking sudden obvious bugs and unexpected behavior decoupled from github issues style.

It is shipped with the project and goes on with the commit history line to describe some bugs fixed or not fixed at a point in a specific commit `HEAD` and when they were first discovered.

Developers of this codebase must ensure to report any observed issues in here, as this is a top priority issue box and every issues in here is seen as to be one of the next to cover `milestones`.

***Why this approach and not github issue*** — because this codebase is private in github. we'd make issues in github public for app users to report to and we want to decouple issues reported between users of the app and issues reported by developers of the app — we believe developers are too close to obvious issues in hand to fix — whilst users issues are more or improvement bugs or some bugs which are not obvious to need user experience to expose them.

***Note:*** Fixed issues aren't to be erased in here hence they're to be labeled `fixed/completed`

## The Table
> This table acts as table of content for issues reporters, they have to put there issues number here before they report it — the number is incremental.

| issues | status | comment |
|--------|--------|---------|
| [0001](#0001) | fixed | Auto-connect overlap resolved in the handshake (earlier fix: fingerprint tie-breaker, since replaced; see `0001` update). |
| [0002](#0002) | fixed | IPC shutdown now runs at Tauri ExitRequested before WebView teardown. |
| [0003](#0002) | fixed | re-report issue `0002` is not fixed correctly or it is fixed half — the issue need recheck carefully|
| [0004](#0004) | report(weak)| Trust revocation is local-only. A peer retaining stale trust can reject a new pairing request, preventing re-pairing until its stale trust is also removed.|
| [0005](#0005) | fixed | Reconnecting from the device that initiated a disconnect can stall because the device manager may reuse a stale transport connection ID.|

**issues** = incremental number of issue reported it is a link will take user direct to issue content
**status** = can be either report or fixed if done fixed — one can also use some tag to give a weight to issue eg. report(strong|weak etc.) — but that is what the comment field is for.
**comment** = the comment field is used to write some extra infos about the issue at a specific point — can be changed.



## 0001
**Observation:** when two devices start and both have already trusted each other they both simultaneously try to connect if auto-connect is turned on for both of them — sometimes one might win but sometimes both can stuck is the connecting state which delay/stops the connection.
Fixed by deterministic connection arbitration: when both trusted peers have auto-connect enabled, only the lexicographically smaller device fingerprint initiates the connection. The other peer stays passive and accepts the inbound handshake.

**Update:** the fingerprint tie-breaker described above was later replaced. Overlapping auto-connect attempts are now resolved in the handshake itself (`device/manager.rs`, `on_connect_challenge`): a request that arrives before our own challenge is sent wins; if both challenges are already in flight, the lower random nonce wins and the other side backs off. Status stays `fixed`.

## 0002
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

## 0004

**Observation**: Trust is stored independently on each device. When device A forgets device B, A removes B from its local trusted-device store and disconnects the active connection if one exists. However, this operation does not notify B or remove B's corresponding trust record.

*Consequently*, B can continue to regard A as trusted even though A no longer trusts B.

This creates a problem when A attempts to pair with B again. B receives the new `PairRequest`, checks its local trusted-device store, and rejects the request with `ALREADY_TRUSTED` because it still considers A trusted.

The rejection does not cause B to revoke its stale trust record. As a result, A cannot complete pairing while the two devices disagree about their trust relationship.

*Interestingly*, if B attempts to connect to A instead, A rejects the `ConnectChallenge` with `UNKNOWN_DEVICE`(`not a trusted device`). B handles this control code by removing its own stale trust record. Both devices then become untrusted and can pair again.

The same situation can occur when a device loses its trust records but retains the same identity key and fingerprint. A complete data wipe that also regenerates the identity key would be a different case because ClipX would recognize it as a new device.

Design question: Should trust revocation be explicitly communicated to the other device, ensuring both sides remove their trust records? Or is the current behavior intentional, where each device discovers the loss of trust only when it attempts to connect?

For now, this is recorded as a potential design issue rather than assuming that the current behavior is necessarily incorrect.

## 0005

**Observation**: When two devices are connected, disconnecting device A and subsequently trying to reconnect from A can leave A permanently stuck in the `CONNECTING` state.

*Interestingly*, when device B initiates the connection instead, the connection can succeed and resolve the situation.

Code investigation: The current implementation suggests that this happens because the device manager retains a stale transport connection ID after a locally initiated disconnect.

The disconnect operation removes the active connection from the transport's connection registry. However, the corresponding entry in the device manager's `transport_connections` map is not explicitly removed.

*Normally*, this map is cleaned up when the transport reports a `Disconnected` event. However, the transport removes the connection before its connection task reports closure. When the subsequent `Registration::Closed` event arrives, the transport no longer finds the connection in its active registry, so it does not emit the expected `Disconnected` event.

*Consequently*, the device manager can retain an outdated connection ID.

When A tries to connect again, "handle_connect" sees that cached entry and assumes an existing transport connection can be reused. It advances the connection session and attempts to send a new `ConnectChallenge` using the old connection ID, even though the transport no longer has that connection.

The handshake therefore cannot progress, leaving the device stuck in `CONNECTING`.

A new connection initiated by B can provide a fresh inbound transport connection. The handshake arbitration can then replace the stale attempt if B's challenge wins.

Expected behavior: Disconnecting a device must clean up both the logical connection state and its corresponding transport connection mapping. A subsequent connection attempt from either device should establish a fresh, valid transport connection without requiring the other device to initiate the connection.