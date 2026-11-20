---
id: a6f28d450f5cd3068ecf4a2b05782f939c03e5fd
legacy: 0005
title: Reconnect from the disconnecting device can stall in CONNECTING
status: fixed
comment: Reconnecting from the device that initiated a disconnect can stall because the device manager may reuse a stale transport connection ID.
---

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
