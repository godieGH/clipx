---
id: 7f716d6727c79b2cee8821850447d5a4fb0b927f
legacy: 0001
title: Auto-connect overlap between two trusted devices
status: fixed
comment: Auto-connect overlap resolved in the handshake (earlier fix: fingerprint tie-breaker, since replaced; see the update).
---

**Observation:** when two devices start and both have already trusted each other they both simultaneously try to connect if auto-connect is turned on for both of them — sometimes one might win but sometimes both can stuck is the connecting state which delay/stops the connection.
Fixed by deterministic connection arbitration: when both trusted peers have auto-connect enabled, only the lexicographically smaller device fingerprint initiates the connection. The other peer stays passive and accepts the inbound handshake.

**Update:** the fingerprint tie-breaker described above was later replaced. Overlapping auto-connect attempts are now resolved in the handshake itself (`device/manager.rs`, `on_connect_challenge`): a request that arrives before our own challenge is sent wins; if both challenges are already in flight, the lower random nonce wins and the other side backs off. Status stays `fixed`.
