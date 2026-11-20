# ClipX

**Copy on one device, paste on another.**

ClipX keeps the clipboards of your own devices in sync over your local network. Copy some text, an image, or a file on your PC and paste it on your phone (or the other way around) without emailing yourself, signing in to anything, or sending your data through someone else's server.

> **Status:** early and actively developed (current version is in [`VERSION`](VERSION)). Windows and Android are the main targets. See [Supported platforms](#supported-platforms) for the honest picture.

---

## Table of contents

- [What is ClipX?](#what-is-clipx)
- [Which part of this README is for me?](#which-part-of-this-readme-is-for-me)
- [Supported platforms](#supported-platforms)
- **Using ClipX** (regular users)
  - [Installing](#installing)
  - [Pairing two devices](#pairing-two-devices)
  - [Everyday use](#everyday-use)
  - [Send with ClipX (Windows right-click menu)](#send-with-clipx-windows-right-click-menu)
  - [Running in the background](#running-in-the-background)
  - [Privacy and security](#privacy-and-security)
  - [Troubleshooting](#troubleshooting)
  - [Where ClipX keeps things](#where-clipx-keeps-things)
- **Developer guide**
  - [Architecture at a glance](#architecture-at-a-glance)
  - [Repository layout](#repository-layout)
  - [Prerequisites](#prerequisites)
  - [Getting started](#getting-started)
  - [Day-to-day workflow](#day-to-day-workflow)
  - [Command-line tool](#command-line-tool)
  - [Testing](#testing)
  - [Android development](#android-development)
  - [Protocol and core concepts](#protocol-and-core-concepts)
  - [Platform-specific code](#platform-specific-code)
  - [Known limitations](#known-limitations)
  - [Contributing](#contributing)
- **Maintainer guide**
  - [Versioning](#versioning)
  - [Making a release](#making-a-release)
  - [What CI does](#what-ci-does)
  - [Signing and secrets](#signing-and-secrets)
  - [Tracking issues](#tracking-issues)
  - [Release checklist](#release-checklist)
- [Glossary](#glossary)
- [References](#references)
- [License](#license)

---

## What is ClipX?

ClipX lets you share these between devices you trust, as long as they are on the same network (for example the same home Wi-Fi):

- plain text
- formatted ("rich") text
- images
- files (offered to the other device, which downloads them when you ask)

There are no accounts and no cloud. Devices find each other on the local network, you **pair** them once (a short approval step on both sides), and from then on they can connect and share.

Under the hood, ClipX is a shared Rust engine (the "core") with thin apps around it: a desktop app (Tauri + React), a command-line tool, and an Android app. Developers can jump to the [Developer guide](#architecture-at-a-glance).

## Which part of this README is for me?

| I want to... | Go to |
|---|---|
| Install and use ClipX | [Installing](#installing), then [Pairing two devices](#pairing-two-devices) |
| Fix something that isn't working | [Troubleshooting](#troubleshooting) |
| Know what happens to my data | [Privacy and security](#privacy-and-security) |
| Build ClipX, fix a bug, or add a feature | [Prerequisites](#prerequisites) and [Getting started](#getting-started) |
| Understand how the code fits together | [Architecture at a glance](#architecture-at-a-glance) and [Protocol and core concepts](#protocol-and-core-concepts) |
| Publish a new version | [Making a release](#making-a-release) |
| Look up a word I don't know | [Glossary](#glossary) |

## Supported platforms

| Platform | Status | Notes |
|---|---|---|
| **Windows** (10/11) | Main platform, full feature set | Includes notifications, the right-click "Send with ClipX" entry, and Wi-Fi Direct (experimental). Installers are not code-signed yet. |
| **Android** (10 or newer, 64-bit ARM) | Supported | Signed release APKs are attached to each release. |
| **Linux** | Early: runs, but pairing doesn't work yet | The app and core start and run. Pairing asks for approval through system notifications, which are only a placeholder on Linux for now, so a new pairing can't complete there yet. Wi-Fi Direct is Windows-only. |
| **macOS** | Builds, not tested | Built by the release pipeline but not covered by regular checks. Treat as experimental. |
| **iOS** | Not available | Planned. |

---

# Using ClipX

## Installing

Installers are attached to each **release** on the project's Releases page. Every release also includes a file called `SHA256SUMS.txt` (see [Verify your download](#verify-your-download-recommended)). If you can't see a Releases page, ask whoever shared ClipX with you for the installer.

**Windows**

1. Download the `.exe` setup file (or the `.msi`).
2. Run it. Windows may warn you, see [About the unsigned warning](#about-the-unsigned-warning).

**Android**

1. Download the `.apk` file on your phone.
2. Open it and allow installing from your browser/files app when Android asks.
3. Needs Android 10 or newer on a 64-bit ARM phone (nearly all phones from the last several years).

**Linux**

Pick the file that matches your system:

- `.deb` for Ubuntu, Debian, Linux Mint: `sudo apt install ./the-file.deb`
- `.rpm` for Fedora and similar
- `.AppImage` for anything else: make it runnable (`chmod +x the-file.AppImage`) and double-click or run it

**macOS**

Open the `.dmg` and drag the app to Applications. See [About the unsigned warning](#about-the-unsigned-warning).

### About the unsigned warning

The Windows and macOS installers are **not code-signed yet**. Signing certificates cost money every year, and ClipX doesn't use one at this stage. The app is safe to run if you got it from this project's Releases page, but your system doesn't know that, so it warns you the first time.

**Windows (SmartScreen):** you may see "Windows protected your PC" with "Unknown publisher".

1. Click **More info**.
2. Click **Run anyway**.

**macOS (Gatekeeper):** macOS may say the developer can't be verified.

1. Right-click (or Control-click) the app and choose **Open**, then confirm.
2. Or open **System Settings > Privacy & Security** and click **Open Anyway** after the first blocked attempt.

**Linux:** no warning is expected.

**Android:** release APKs are signed with the project's release key.

### Verify your download (recommended)

Because the installers are unsigned, you can check that your file is exactly what was published. Compare its checksum with the matching line in `SHA256SUMS.txt` from the same release. If they differ, don't run the file.

Windows (PowerShell):

```powershell
Get-FileHash .\<downloaded-file> -Algorithm SHA256
```

macOS / Linux:

```bash
sha256sum <downloaded-file>   # on macOS: shasum -a 256 <downloaded-file>
```

Only download ClipX from the project's Releases page.

## Pairing two devices

You do this once per pair of devices. Both devices need ClipX installed and must be on the same network.

1. Open ClipX on both devices.
2. On the first device, look at **Available Devices**. If the other device isn't listed, press **Scan**.
3. Press **Pair** next to the other device.
4. On the *other* device, a pairing request appears (a notification on Windows). Approve it.
5. Both devices show a short **code**. Check that it is the same on both screens, and confirm.
6. The device now appears under **Paired Devices**. Press **Connect** (or turn on **Auto-connect** so it connects by itself whenever both are around).

Pairing requests expire after about a minute and a half. If you miss it, just start again.

> **Linux:** pairing can't be completed on Linux yet, because the approval prompts rely on system notifications that ClipX doesn't have for Linux so far.

## Everyday use

**Automatic sharing.** When two paired devices are connected, what you copy on one is sent to the other. Items that arrive are also kept in **History**.

**Sending on purpose.** Press the **+** button in the title bar to open the **Send** screen:

- **Clipboard:** press *Add clipboard* to pick up what you currently have copied.
- **Files:** drop files anywhere in the window, or click the box to choose them. Files stay on your device until you press **Send now**.

**Files are offers.** When you send a file, the other device gets an *offer*. They press the download button in **History** when they want it. An offer stays available for up to 24 hours, and an interrupted download can pick up where it left off.

**History.** The clipboard icon in the title bar shows what has arrived (up to the 200 most recent items). You can copy an item again, download a file, remove one item, or **Clear all**.

**Device details.** Click a paired device to see its address, turn **Auto-connect** on or off, **Disconnect**, or **Forget** it (which un-pairs it).

**Size limits.** Very large items can be rejected. Roughly: text up to 4 MB, formatted text and images up to 8 MB. For anything bigger, send it as a file.

## Send with ClipX (Windows right-click menu)

The Windows installer adds a **Send with ClipX** entry to the file right-click menu.

- **Windows 10:** right-click a file and choose **Send with ClipX**.
- **Windows 11:** right-click a file, choose **Show more options** (or press **Shift+F10**), then choose **Send with ClipX**. It doesn't appear in the new compact menu yet.

It sends one file at a time, to a paired device. Folders are not supported yet.

## Running in the background

ClipX is meant to keep running quietly so syncing just works.

- **Closing the window only hides it.** ClipX keeps running in the system tray (the small icons near the clock).
- **To open it again,** click the tray icon, or choose **Open** from its menu.
- **To quit completely,** right-click the tray icon and choose **Quit**.
- **The tray menu** also has **Start with system** (launch at login), **Restart core** / **Stop core**, **Help & diagnostics**, and **Open logs folder**.
- **At login, the window stays hidden** so ClipX doesn't pop up every time you start your computer. If your Linux desktop has no tray, ClipX detects that and shows the window instead, so it never becomes unreachable.

> **Linux tip:** some desktops (for example stock GNOME) don't show tray icons without an extension such as "AppIndicator and KStatusNotifierItem Support".

## Privacy and security

**What stays local.** ClipX has no accounts, no cloud, and no servers. Your clipboard goes directly from one of your devices to another over your own network. Your clipboard history is stored on your own device.

**Who can connect.** Every device has its own secret key. Two devices only exchange clipboard data after you've paired them, and they prove who they are with that key each time they connect. A stranger on the same Wi-Fi can see that ClipX is running, but they can't pair without you approving it.

**An important limit.** At the moment, the connection between your devices is **not encrypted**. That means someone who can watch traffic on your network could read what you send. So:

- Use ClipX on networks you trust, like your home Wi-Fi.
- Avoid it on public or shared Wi-Fi (cafes, airports, hotels).
- Be careful copying passwords or other secrets while it's running on an untrusted network.

Encrypting the connection is planned future work.

**Reporting a security problem.** Please don't post it publicly. Contact the maintainer directly.

## Troubleshooting

**My other device doesn't show up under Available Devices.**

- Make sure both devices are on the same Wi-Fi or network, and ClipX is running on both.
- Press **Scan** again.
- Some networks (guest Wi-Fi, offices, public hotspots) block devices from seeing each other. Try your home network or a phone hotspot.
- Check your firewall. On Windows, allow ClipX on **private** networks when asked. Devices find each other using UDP port 9999, and connect using TCP port 8080 (it can pick another port if 8080 is taken, so allowing the app is better than allowing one port).
- Turn off VPNs while testing. They often hide you from the local network.

**Pairing never completes.**

- The request expires after about 90 seconds. Start again.
- On the other device, check you didn't miss the approval. On Windows, "Focus assist" / Do Not Disturb can hide notifications.
- Both devices must confirm the same code.
- On Linux, pairing isn't available yet (see [Supported platforms](#supported-platforms)).

**I can't find the ClipX window.**

- Click the tray icon or choose **Open** from its menu: closing the window only hides it.
- On Linux without a tray, start the app again from your applications menu: a manual launch always shows the window.

**ClipX says the core failed to start / "Address already in use".**

- Another copy of ClipX is probably still running. Quit it from the tray menu (or end it in Task Manager / `pkill clipx-core`) and open ClipX again.

**It connects, but nothing arrives.**

- Check the device shows **Connected** (not just paired).
- For a file, remember it arrives as an *offer*: download it from **History**.
- Items over the size limits above can be rejected. Send big things as files.

**I need to share diagnostic information.**

Open **Help & diagnostics** (the **?** button in the title bar, or the tray menu). You can view recent logs and copy a report. Logs can contain device names, file paths, and network details, so read the report before posting it.

## Where ClipX keeps things

| What | Windows | Linux |
|---|---|---|
| Logs | `%LOCALAPPDATA%\godiegh\clipx\data\logs` | `~/.local/share/clipx/logs` |
| Device identity, paired devices, history | `%LOCALAPPDATA%\godiegh\clipx\data` | `~/.config/clipx` |
| Files you download from other devices | `Downloads\Clipx` | `~/Downloads/Clipx` (or `$XDG_DOWNLOAD_DIR/Clipx`) |

Forgetting a device in the app is the normal way to un-pair. Don't delete the identity file unless you want this device to become a new, unrecognized device that has to be paired again.

---

# Developer guide

## Architecture at a glance

ClipX is split into a long-running **core** (the engine) and thin **clients** that talk to it.

```text
 ┌───────────────┐   ┌─────────────┐
 │ Tauri + React │   │ clipx (CLI) │          desktop clients
 │   (tray/UI)   │   │             │
 └───────┬───────┘   └──────┬──────┘
         │  local IPC (name "clipx"):          Protobuf messages
         │  WebSocket framing over a           defined in protos/clipx.proto
         │  local socket / named pipe
         ▼                  ▼
 ┌─────────────────────────────────────────┐
 │                clipx-core               │   Rust + Tokio
 │ clipboard • discovery • pairing/trust   │
 │ transport • file transfer • notifications│
 └───────┬─────────────────────────┬───────┘
         │ UDP 9999 (discovery)    │ TCP 8080 (WebSocket, falls back
         │ broadcast every 2 s     │ to an OS-assigned port)
         ▼                         ▼
              other devices on the same network
              (Windows, Linux, Android ...)
```

Key points:

- **One core per machine.** The core owns networking, pairing, trust, and clipboard sync. The UI, tray, and CLI are IPC clients.
- **The tray app launches the core.** On startup it tries to connect over IPC; if nothing answers it spawns `clipx-core` from the same folder as its own executable, then retries for about five seconds (`desktop/clipx-app/src-tauri/src/core_supervisor.rs`). A watchdog restarts the core if it dies.
- **Android has no separate daemon.** The Kotlin app calls the same Rust core through a [UniFFI](https://github.com/mozilla/uniffi-rs) bridge (`mobile/native`).
- **One protocol.** `protos/clipx.proto` defines both device-to-device messages and UI-to-core IPC messages.

## Repository layout

```text
clipx/
├── README.md
├── VERSION                  single source of truth for the version
├── assets/                  brand marks (SVG); the app icon set is generated from these
├── desktop/                 Cargo workspace for everything desktop
│   ├── Cargo.toml           workspace: clipx-app/src-tauri, clipx-core, clipx
│   ├── clipx-core/          the engine (service.rs is the entry point)
│   │   └── src/{clipboard,device,netio,notification}/
│   ├── clipx/               CLI + IPC client library (clipx-send for the Windows context menu)
│   └── clipx-app/           Tauri + React app
│       ├── src/             React UI (App.tsx)
│       └── src-tauri/       Rust side of the Tauri app, core supervisor, bundle config
├── mobile/
│   ├── android/             Kotlin + Jetpack Compose app (Gradle)
│   ├── ios/                 not active yet
│   └── native/              separate Cargo workspace: UniFFI bridge to the core
├── protos/clipx.proto       the wire and IPC contract
├── scripts/                 release.mjs, sync-version.mjs
├── tests/                   emulator tool (separate crate), see tests/README.md
├── issues/issue.md          developer-facing bug log (see "Tracking issues")
└── .github/workflows/       ci.yaml, release.yaml
```

Three separate Cargo projects exist: `desktop/`, `mobile/native/`, and `tests/emulator/`. They each build on their own.

## Prerequisites

| Tool | Why | Notes |
|---|---|---|
| **Rust** | Core, Tauri app, CLI | Use a recent stable toolchain. CI pins `1.96.0`. The code uses Rust edition 2024. |
| **protoc** (Protocol Buffers compiler) | `clipx-core`'s build script compiles `protos/clipx.proto` | The build fails without it. Windows: `winget install protobuf`. Ubuntu/Debian: `sudo apt install protobuf-compiler`. macOS: `brew install protobuf`. |
| **Node.js 22** and **pnpm** | Frontend and Tauri CLI | `package.json` pins `pnpm@11.6.0`: run `corepack enable` and it will use that version. |
| **Windows only:** MSVC Build Tools | Compiling Rust/Tauri | Install the "Desktop development with C++" workload. WebView2 ships with Windows 11. |
| **Linux only:** system libraries | Tauri's webview and tray | `sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf` |

For Android work, see [Android development](#android-development).

## Getting started

```bash
git clone <repository-url>
cd clipx/desktop/clipx-app

pnpm install
pnpm tauri icon ../../assets/clipx-brand-mark-squircle.svg   # needed on a fresh clone
pnpm tauri:dev
```

**Why the icon step?** `src-tauri/icons/*` is gitignored, because the icons are generated from the SVG in `assets/`. A fresh clone has none, and without them `pnpm tauri:dev` stops with "Source icon not found" and the Tauri build fails on the missing icon files. CI and `scripts/release.mjs` run this command for the same reason.

**What `pnpm tauri:dev` does:** it first builds `clipx-core` into `desktop/target/debug/` (so it sits next to the app binary), starts the Vite dev server, then starts the app. The app's core supervisor then finds and launches that core automatically. In debug builds the core's logs print in your terminal.

**Check it works:**

- The window appears (and the tray icon, on systems that have one).
- The **?** button opens *Help & diagnostics*, which shows the core's recent logs.
- With the core running, the CLI can talk to it:

```bash
cargo run --manifest-path desktop/clipx/Cargo.toml -- seen
```

## Day-to-day workflow

**Run only the core, without the UI:**

```bash
cd desktop
cargo run -p clipx-core
```

**Verbose logs.** The core logs at `info` by default. Use the standard `RUST_LOG` variable, for example `RUST_LOG=debug`. Logs are also written to a rolling daily file (see [Where ClipX keeps things](#where-clipx-keeps-things)). In **release** builds the core prints nothing to the terminal, so read the log file or *Help & diagnostics* instead.

**Fixed ports.** The core binds UDP **9999** (discovery) and prefers TCP **8080** (falls back to a random free port; discovery has no fallback). Only one real core can run per machine. If you see `Address already in use`, an earlier core is probably still running: `pkill clipx-core` (Linux/macOS) or end it in Task Manager (Windows). The tray app doesn't always stop a core it spawned if the app crashes.

**Build a local installer** (slower, only when you need one):

```bash
cd desktop/clipx-app
pnpm release
```

This runs `scripts/prepare-core.mjs` to build and stage `clipx-core` and `clipx-send` as bundled sidecars (the release config in `src-tauri/tauri.release.conf.json` lists them under `externalBin`), then bundles the installers.

**Maintenance helper:** `desktop/clipx-app/scripts/manage-tauri.mjs` can sync the icon into `public/` (`--sync-icons`), remove `dist/` (`--clean-dist`), or run `cargo clean` (`--cargo-clean`, `--cargo-clean-all`, `--all`).

## Command-line tool

`desktop/clipx` builds a command-line client, `clipx`, that talks to the same core over the same IPC. It's handy for debugging without the UI. The core must be running (start the app, or `cargo run -p clipx-core`).

```bash
cargo run --manifest-path desktop/clipx/Cargo.toml -- <command>
```

| Command | What it does |
|---|---|
| `seen` | List devices currently discovered on the network |
| `trusted` | List trusted (paired) devices |
| `identity` | Show this device's identity and fingerprint |
| `pair start <device-id>` | Start pairing with a discovered device |
| `pair pending` | List pending pairing requests (with codes) |
| `pair approve <device-id> [--deny]` | Approve (or reject) a pending request |
| `connect ...` | Connect to or disconnect from a trusted device |
| `device ...` | List or manage paired devices |
| `clipboard ...` | Manage clipboard history and downloaded files |
| `send ...` | Send content to a connected device |
| `download <entry-id>` | Download a clipboard file by history entry id |

Run `clipx <command> --help` for the options of each command. A second binary, `clipx-send`, powers the Windows right-click entry.

## Testing

Run the same checks CI runs. From `desktop/`:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

And from the repository root:

```bash
node scripts/sync-version.mjs --check
```

For the mobile bridge (from `mobile/native/`) run the same three cargo commands.

**Testing pairing without a second device.** `tests/emulator` is a standalone tool that speaks ClipX's discovery, transport, and pairing protocol directly, reusing the real core's identity code and protobuf types, but with a manual command loop so you can stop at any stage and inspect how the other side reacts:

```bash
cargo run --manifest-path tests/emulator/Cargo.toml -- --name alice
```

Run a second one to pair two emulators, or point one at a real running `clipx-core`. It uses configurable ports, so it can coexist with a real core on the same machine. Type `help` inside it for the command list. Details are in [`tests/README.md`](tests/README.md).

**Testing on real hardware.** Two real devices on one network is the best test. A few warnings:

- Don't run two real cores on one machine (fixed ports, see above).
- On Windows with WSL2 in *mirrored* networking mode, WSL shares the host's ports, so a core inside WSL will collide with ClipX on Windows.
- WSL2 in its default NAT mode can't receive LAN broadcasts, so discovery from inside WSL won't see your other devices. WSL is fine for compiling and for checking that the core starts, not for end-to-end testing.

## Android development

The Android app lives in `mobile/android` and the Rust bridge in `mobile/native`. Gradle builds the Rust bridge (with `cargo ndk`) and generates the Kotlin bindings automatically as part of a normal build.

**You need:**

- JDK 21
- Android SDK (and the NDK that Gradle asks for)
- `rustup target add aarch64-linux-android`
- `cargo install cargo-ndk`
- `protoc` (as above)

**Build and install:**

```bash
cd mobile/android
./gradlew assembleDebug -PclipxAbis=arm64-v8a
./gradlew installDebug
```

By default only `arm64-v8a` is built. To include the emulator ABI too:

```bash
./gradlew assembleDebug -PclipxAbis="arm64-v8a,x86_64"
```

Debug builds need no signing key. The app targets Android 10 and newer (`minSdk` 29).

**Running on an emulator:**

```bash
emulator -list-avds
emulator -avd <device_name>
./gradlew installDebug
```

**Checking the Rust side without Gradle** (this is what CI does):

```bash
cd mobile/native
cargo ndk -t arm64-v8a build --package rust-mobile-ffi-bridge --release
```

## Protocol and core concepts

Read `protos/clipx.proto` first. It is the canonical contract for both device-to-device messages and UI-to-core IPC.

**Identity and fingerprint.** Each device generates an ed25519 key pair the first time it runs. The private key never leaves the device. A device's **fingerprint** (its ID everywhere in the app) is the SHA-256 hash of its public key.

**Discovery.** Every 2 seconds each core broadcasts an `Announce` (fingerprint, name, device type, WebSocket port) over UDP port 9999, to the broadcast address of every network interface. Listeners ignore their own fingerprint.

**Pairing and trust.** Pairing is a request/response exchange followed by a signed-nonce challenge, with a confirmation code shown on both sides. After pairing, the other device's public key is stored in the trusted-devices list.

**Connecting.** Every connection between trusted devices starts with a signed challenge, so each side proves it holds the key it paired with. *Auto-connect* is triggered when a trusted device newly appears on the network (not on every reconnect). Both sides may try at once. The handshake settles the overlap: a request that arrives before the other side has sent its own wins, and if both are truly simultaneous the random nonces break the tie, so exactly one connection survives. Failed or dropped attempts are retried a limited number of times. (`issues/issue.md` entry 0001 still describes an earlier fingerprint tie-breaker that the code no longer uses.)

**Transport.** Peers talk Protobuf `PeerMessage`s over a plain WebSocket on TCP. The connection is not encrypted (see [Known limitations](#known-limitations)).

**Clipboard sync.** A watcher polls the local clipboard (text, rich text via HTML, images, and `file://` lists). Changes are broadcast to connected trusted peers; incoming content is applied through the platform's clipboard sink and recorded in history (the 200 most recent entries).

**Files.** Files never travel inside a clipboard message. The sender publishes a short-lived `FileOffer` (id, name, type, size, expiry). The receiver asks for it with a `FileDownloadRequest` (with an offset, so it can resume), and the sender streams `FileChunk`s with a SHA-256 and waits for `FileChunkAck`s. Stalled downloads re-ask the sender a few times before reporting the transfer as interrupted.

**IPC.** The UI and CLI connect to the core on a local socket named `clipx` (an abstract/namespaced socket on Linux, a named pipe on Windows), wrap it in a WebSocket, and exchange `IpcRequest`/`IpcResponse` messages. The core can also push events (`devices_changed`, `clipboard_changed`, pairing and file-transfer progress), so the UI re-fetches on demand instead of polling.

**Suggested reading order for the code:**

1. `protos/clipx.proto`
2. `desktop/clipx-core/src/service.rs` (which subsystems start together)
3. `desktop/clipx-core/src/device/` (identity, pairing, trusted devices, the device manager)
4. `desktop/clipx-core/src/clipboard/` (watching and applying the clipboard)
5. `desktop/clipx-core/src/netio/` (discovery, transport, IPC server)
6. `desktop/clipx-app/src-tauri/src/` (core supervisor, tray, Tauri commands)
7. `desktop/clipx-app/src/App.tsx` (the UI)
8. `mobile/native/` and `mobile/android/` (mobile integration)

## Platform-specific code

The core compiles for Windows, Linux, and macOS, with platform pieces behind `cfg` gates:

| Area | Windows | Linux / macOS |
|---|---|---|
| Notifications (pair prompts, "clipboard received", progress) | `notification/windows_engine.rs` | `notification/stub_engine.rs`: logs the prompt and auto-denies / ignores it |
| Wi-Fi Direct | `netio/p2p_windows.rs` | not implemented |
| Clipboard | `arboard` | `arboard` |
| App window effect | Mica backdrop | opaque background |
| Explorer right-click entry | installer fragment (`src-tauri/windows/`) | n/a |
| Core startup error dialog | native dialog | printed to stderr |

Because the Linux notification engine is a stub, **pairing can't complete on Linux yet**. Both sides of a pairing ask the user through `ask_pair` (first "allow this device?", then "does the code match?"), and the stub answers `NoResponse` immediately. The device manager treats `NoResponse` like a denial and aborts the session (`on_pair_approval_result` and `on_code_confirm_result` in `device/manager.rs`). The IPC protocol already has `PairPending` / `PairApprove` requests, with matching CLI commands, but with the stub the session is aborted before they could help. A real Linux notification engine, or an in-app approve/confirm dialog driven by those IPC requests, is the key piece of work for Linux support and a good first contribution.

On Linux, the tray app decides whether to start hidden at login by asking the session bus whether a StatusNotifier host exists (`start_hidden` in `src-tauri/src/lib.rs`, using the `dbus-send` tool). If it can't confirm a tray, the window is shown.

## Known limitations

- **No encryption in transit.** Devices authenticate each other, but traffic is plain WebSocket. This is the most important thing to fix before recommending ClipX on untrusted networks.
- **Linux:** pairing can't complete (placeholder notification engine, see above); no Wi-Fi Direct.
- **Wi-Fi Direct (Windows and Android)** is experimental, and the offline link can be unstable.
- **Installers aren't code-signed** on Windows and macOS.
- **macOS** is built by the release workflow but not tested by regular CI.
- **Android** builds only `arm64-v8a` by default.
- **Folders** can't be sent, only files, and one at a time from the Windows right-click menu.
- **WSL** isn't a supported runtime (graphics and NAT networking get in the way).
- **Quitting on Windows** can still print a Chromium/WebView2 teardown message to a terminal (issue 0002 in `issues/issue.md`). It's treated as an upstream diagnostic.

## Contributing

1. Read the [Protocol and core concepts](#protocol-and-core-concepts) section, then trace the code from `service.rs`.
2. Create a branch and make your change. Keep protocol (`protos/clipx.proto`) and implementation changes aligned: they're consumed by desktop and mobile alike.
3. Run the checks in [Testing](#testing). CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and the tests on Ubuntu and Windows for every pull request, so run them locally first.
4. Don't edit version numbers by hand. See [Versioning](#versioning).
5. Found a bug you won't fix right now? Log it in `issues/issue.md` (see [Tracking issues](#tracking-issues)).
6. Open a pull request against `main`.

---

# Maintainer guide

## Versioning

`VERSION` at the repository root is the single source of truth. `scripts/sync-version.mjs` copies it to:

- `desktop/Cargo.toml`
- `mobile/native/Cargo.toml`
- `desktop/clipx-app/src-tauri/tauri.conf.json`
- `desktop/clipx-app/package.json`

(The Android app reads `VERSION` directly.) CI runs `node scripts/sync-version.mjs --check` and fails if anything has drifted, so never edit these by hand.

## Making a release

Use the release script from the repository root:

```bash
node scripts/release.mjs <patch|minor|major|x.y.z> [flags]
```

Typical use:

```bash
node scripts/release.mjs patch --dry-run     # see what it would do, then revert
node scripts/release.mjs patch               # real release (asks before committing)
node scripts/release.mjs minor --yes         # skip the confirmation prompt
```

**What it does, in order:**

1. Checks your tools (git, node, rustc, cargo, protoc, pnpm; plus Java, `cargo-ndk` and the Android Rust target with `--android`).
2. Checks git state: clean working tree, on `main`, not behind the remote, and the new tag doesn't already exist.
3. Bumps `VERSION`, runs `sync-version.mjs` (and its `--check`), then `cargo update -w` in both Cargo workspaces.
4. Runs the tests: `cargo test --workspace` in `desktop/` and `cargo check` on the mobile bridge.
5. Optionally builds (see flags below).
6. Shows the diff, asks for confirmation, then commits `release: vX.Y.Z`, creates the tag, and pushes both in one atomic push.

If anything fails or you decline, it reverts its changes. Pushing the tag is what starts the real release (see [What CI does](#what-ci-does)).

**Flags:**

| Flag | Effect |
|---|---|
| `--dry-run` | Do everything, show the result, then revert. |
| `--yes` | Don't ask for confirmation. |
| `--no-push` | Commit and tag locally only; it prints the push command. |
| `--skip-tests` | Skip the test step. |
| `--build` | Also build the workspace in release mode. Required for the two flags below to do anything. |
| `--bundle` | With `--build`: also produce desktop installers (`pnpm release`). |
| `--android` | With `--build`: also run `gradlew assembleRelease`. Also makes the tool check require Java and `cargo-ndk`. |
| `--allow-branch` | Allow releasing from a branch other than `main`. |

> The usage comment at the top of `scripts/release.mjs` lists `--skip-build`, but the code actually builds only when `--build` is passed. Consider updating the comment.

## What CI does

**`ci.yaml`** runs on pushes to `main` and on pull requests:

| Job | What it checks |
|---|---|
| `version` | All version numbers agree. |
| `desktop` (Ubuntu 22.04 and Windows) | Frontend build, then `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`. |
| `ffi` | The mobile bridge: fmt, clippy, tests, an Android cross-compile, and Kotlin binding generation. |
| `android` | Kotlin unit tests and a debug APK build. |
| `emulator` | The emulator tool compiles and passes clippy. |

**`release.yaml`** runs when a tag matching `v*` is pushed (or manually, as a dry run that builds everything but publishes nothing):

1. `check-version`: the tag must equal `VERSION`, and versions must be in sync.
2. `desktop`: builds installers on Ubuntu 22.04, Windows, and macOS (`.exe`/`.msi`, `.deb`/`.rpm`/`.AppImage`, `.dmg`).
3. `android`: builds the APK, signed if the keystore secrets exist, unsigned otherwise.
4. `release`: collects every artifact, writes `SHA256SUMS.txt`, and creates the GitHub release with generated notes.

## Signing and secrets

- **Android:** signing uses these repository secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`. Without them, CI still builds, but publishes `...-unsigned.apk`.
- **The keystore file** (`clipx-release.jks`) is listed in `.gitignore` and must never be committed. Keep a safe backup elsewhere: if it's lost, Android won't accept updates to already-installed copies of the app.
- **Windows and macOS** installers are currently unsigned (see the user-facing warning notes above). If signing is added later, update [Installing](#installing).

## Tracking issues

Developer-facing bugs are logged in [`issues/issue.md`](issues/issue.md), separately from any public issue tracker. The reasoning there is that developers are close to the obvious problems, while users report different, experience-driven ones, so the two streams are kept apart.

The rules:

- Add a row to the table at the top **before** writing up the issue; numbers are incremental (`0001`, `0002`, ...).
- Each issue has an anchor so the table can link to it.
- Never erase a fixed issue. Mark it `fixed` / `completed` instead, so the file doubles as history.
- If an issue turns out to be only half-fixed, log a new row that points back to the original (as `0003` does for `0002`).
- Anything obvious and cheap to fix should simply be fixed right away.

## Release checklist

- [ ] `main` is green in CI.
- [ ] `issues/issue.md` reviewed: nothing blocking is open.
- [ ] Dry run is clean: `node scripts/release.mjs <bump> --dry-run`.
- [ ] Android keystore secrets are still present in the repository settings.
- [ ] Run the real release (omit `--dry-run`) and confirm the commit and tag.
- [ ] Watch the **Release** workflow finish for every platform.
- [ ] Download an installer, verify it against `SHA256SUMS.txt`, and install it on a clean machine.
- [ ] Smoke test: pair two devices, send text, send a file.
- [ ] Update the README if behavior, platforms, or limitations changed.

---

## Glossary

| Term | Plain meaning |
|---|---|
| **Pairing** | The one-time approval that makes two devices trust each other. |
| **Trusted / paired device** | A device you've approved. Only these can sync with you. |
| **Fingerprint** | A long code that uniquely identifies a device (a hash of its public key). Shown in device details. |
| **Auto-connect** | Setting that makes two paired devices connect by themselves when both are around. |
| **Core** | The background engine that does the actual syncing (`clipx-core`). The app windows are just controls for it. |
| **Tray** | The row of small icons near the clock (Windows/Linux) or menu bar (macOS) where ClipX stays running. |
| **LAN** | Local area network: your home or office Wi-Fi/Ethernet. |
| **File offer** | A notice that a file is available. The other device downloads it when ready. |
| **IPC** | How the app window talks to the core running on the same computer. |
| **Sidecar** | A helper program bundled inside an installer (the core is one). |
| **Protobuf** | The compact message format ClipX uses between devices and between the app and core. |

## References

**In this repository**

- [`protos/clipx.proto`](protos/clipx.proto): the protocol
- [`tests/README.md`](tests/README.md): the emulator tool
- [`issues/issue.md`](issues/issue.md): developer issue log
- [`scripts/release.mjs`](scripts/release.mjs) and [`scripts/sync-version.mjs`](scripts/sync-version.mjs): release tooling
- [`.github/workflows/`](.github/workflows): CI and release pipelines

**Technologies used**

- [Tauri 2](https://tauri.app/): desktop app shell
- [Rust](https://www.rust-lang.org/) and [Tokio](https://tokio.rs/): the core
- [Protocol Buffers](https://protobuf.dev/) and [prost](https://github.com/tokio-rs/prost): messages
- [tokio-tungstenite](https://github.com/snapview/tokio-tungstenite): WebSocket transport
- [interprocess](https://github.com/kotauskas/interprocess): local-socket IPC
- [arboard](https://github.com/1Password/arboard): desktop clipboard access
- [UniFFI](https://github.com/mozilla/uniffi-rs) and [cargo-ndk](https://github.com/bbqsrc/cargo-ndk): Rust-to-Android bridge
- [Jetpack Compose](https://developer.android.com/jetpack/compose): Android UI
- [React](https://react.dev/) and [Vite](https://vite.dev/): desktop UI

## License

A license has not been chosen yet. Until one is added to this repository, don't assume you have permission to copy, modify, or redistribute the code.
