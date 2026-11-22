# Core logging and diagnostics

*This came with a patch that instruments the desktop `clipx-core` host. Applied on ClipX `main` at commit `7976e176a270248669f1941bdcedad22debcf549`
(2026-11-22)*. 

The core uses `tracing` events with the standard five levels. The default filter
is `info`; set `RUST_LOG` before starting the desktop `clipx-core` executable to
increase verbosity. The filter is applied at startup, so restart the core after
changing it.

Examples (Windows PowerShell):

```powershell
$env:RUST_LOG = "clipx_core=debug"
```

Enable all core events, including per-frame and per-chunk timing:

```powershell
$env:RUST_LOG = "clipx_core=trace"
```

Enable detailed transport diagnostics while keeping the rest of the core at
INFO:

```powershell
$env:RUST_LOG = "clipx_core::netio::transport=trace,clipx_core=info"
```

The same environment variable works in a POSIX shell with `export RUST_LOG=...`.

## Host scope

The Android FFI bridge has a separate logger in
`mobile/native/rust-mobile-ffi-bridge/src/logger.rs`. It currently installs
fixed TRACE-level filters and keeps a bounded in-memory diagnostic buffer.
This desktop patch does not add runtime level selection to that mobile logger;
`RUST_LOG` examples above should not be assumed to configure Android.

## Event-level contract

| Level | Intended use in ClipX |
| --- | --- |
| ERROR | An operation failed in a way that prevents a required operation or persistence step from completing. |
| WARN | A recoverable failure, invalid peer input, unavailable subsystem, rejected operation, or transfer failure needing attention. |
| INFO | Core/service lifecycle, device discovery, pairing/connection outcomes, and completed state changes. |
| DEBUG | Operation-level diagnostics, command outcomes, aggregate transfer profiles, and recoverable failures useful during routine investigation. |
| TRACE | High-frequency internal decisions, individual protocol frames/chunks, queue/ACK timings, polling details, and detailed state transitions. |

Transfer summaries remain at DEBUG. Per-message transport timing, inbound
dispatch timing, and per-chunk upload/download timing are TRACE so a normal
DEBUG session does not generate one record for every chunk.

## Output and analysis

The core retains its existing rolling text log format, normal log directory,
and daily rotation/retention policy. Events use named structured fields that
can be searched and inspected offline. Debug builds also write human-readable
events to stderr; the current formatter does not emit JSON Lines.

Fields are deliberately structured (for example `device_id`, `file_id`,
`message_type`, `seq`, `offset`, `bytes`, and timing values ending in `_us`).
Clipboard text, image bytes, pairing codes, private keys, and notification
message bodies must not be logged. Identifiers and file metadata are included
only where they help correlate a flow; treat logs as diagnostic data rather
than public telemetry.

`tracing` events are local structured logs, not an OpenTelemetry exporter or a
remote telemetry upload. A future exporter can consume the same events without
changing the call sites.
