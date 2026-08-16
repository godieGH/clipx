# /tests

Project-wide tests that don't belong to a single sub-project's own test
suite. Rust unit/integration tests stay inside `desktop/`'s workspace (e.g.
`desktop/clipx-core/src/device/identity.rs` already has `#[cfg(test)]`
tests); Android/Kotlin tests will live under `android/` once that project
exists; iOS tests under `ios/`. This folder is for tools and tests that
exercise the project as a whole, across process/device boundaries.

## Emulator

A standalone CLI tool that speaks clipx's discovery, transport, and
pairing/connect wire protocol directly — reusing `clipx-core`'s real
identity/crypto and protobuf types as a library dependency, but with no
automatic state machine. You control every stage manually via a command
loop, or trigger a full automated flow when you just want the end state.

It is **not** part of the `desktop/` Cargo workspace — it's its own crate,
built and run separately:

```
cargo run --manifest-path tests/emulator/Cargo.toml -- --name alice
```

Run a second instance in another terminal to pair two emulators against
each other, or point one at a real running `clipx-core` instance.

Type `help` once it starts for the full command list. Highlights:

- `disco broadcast on` / `disco listen on` — run just the discovery
  components in isolation
- `listen-inbound on` + `dial <label> <addr>` — open real TCP+WS
  connections, tagged with a label you choose
- `send <label> pair-challenge` — send exactly one stage of the pairing
  flow and stop there, to inspect how the other side reacts
- `pair auto <label>` / `respond auto on` — run the full initiator or
  responder flow end to end in one command

### Why it doesn't reuse `net::discovery`/`net::transport` directly

Those modules bind fixed ports (UDP 9999, TCP from `config::get_ws_port()`
= 8080) — fine for the real app, but it means a second instance on the same
machine can't bind them too. The emulator reimplements the same
broadcast/listen/dial/accept logic with configurable ports (default 9999
for discovery to interoperate with a real instance, via SO_REUSEADDR; 8081
for its own inbound listener to avoid colliding with a real instance's
8080) — same wire behavior, just parameterized. If you want multiple
emulator instances *and* a real `clipx-core` all discovering each other on
one machine, this is why it works.
