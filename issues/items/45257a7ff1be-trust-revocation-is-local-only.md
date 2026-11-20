---
id: 45257a7ff1be787c6caf1aece1d3ae5e79589065
legacy: 0004
title: Trust revocation is local-only
status: report
weight: weak
comment: Trust revocation is local-only. A peer retaining stale trust can reject a new pairing request, preventing re-pairing until its stale trust is also removed.
---

**Observation**: Trust is stored independently on each device. When device A forgets device B, A removes B from its local trusted-device store and disconnects the active connection if one exists. However, this operation does not notify B or remove B's corresponding trust record.

*Consequently*, B can continue to regard A as trusted even though A no longer trusts B.

This creates a problem when A attempts to pair with B again. B receives the new `PairRequest`, checks its local trusted-device store, and rejects the request with `ALREADY_TRUSTED` because it still considers A trusted.

The rejection does not cause B to revoke its stale trust record. As a result, A cannot complete pairing while the two devices disagree about their trust relationship.

*Interestingly*, if B attempts to connect to A instead, A rejects the `ConnectChallenge` with `UNKNOWN_DEVICE`(`not a trusted device`). B handles this control code by removing its own stale trust record. Both devices then become untrusted and can pair again.

The same situation can occur when a device loses its trust records but retains the same identity key and fingerprint. A complete data wipe that also regenerates the identity key would be a different case because ClipX would recognize it as a new device.

Design question: Should trust revocation be explicitly communicated to the other device, ensuring both sides remove their trust records? Or is the current behavior intentional, where each device discovers the loss of trust only when it attempts to connect?

For now, this is recorded as a potential design issue rather than assuming that the current behavior is necessarily incorrect.
