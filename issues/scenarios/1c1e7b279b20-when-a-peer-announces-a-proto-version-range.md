---
id: 1c1e7b279b20a7d7c74c35d4c5a5e0af259cb600
legacy: WH-010
area: protocol
status: untested
question: When a peer announces a proto version range this build does not support, what do both devices show?
---

Use the emulator: --proto-versions 2-2. Expect control code 4009 and a message on the peer.
