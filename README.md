# ClipX

ClipX is a cross-device clipboard synchronization project built around a shared Rust core and platform-specific clients. It allows a user to share text, rich text, images, and files between trusted devices on the same network, with a desktop UI and a mobile/native bridge in the same codebase.

At a high level, the repository is organized as a monorepo:

- `desktop/` contains the desktop runtime, Tauri app, shared Rust core, and command-line tooling.
- `mobile/` contains the Android/iOS native bridge and mobile integration layer.
- `protos/` contains the shared protocol definition used by desktop and mobile clients.
- `tests/` contains emulator or integration tests for the sync stack.
- `issues/` tracks project-specific bug notes and engineering issues.

---

## What this project contains

### 1. Shared runtime: `desktop/clipx-core`
This is the core engine of the project. It owns:

- clipboard watching and mirroring
- peer discovery on the local network
- trusted device tracking and pairing flow
- transport setup and connection management
- file transfer orchestration
- notifications and UI event forwarding

The main runtime coordination lives in:

- `desktop/clipx-core/src/service.rs`
- `desktop/clipx-core/src/device/`
- `desktop/clipx-core/src/clipboard/`
- `desktop/clipx-core/src/netio/`

This is the most important place to start when trying to understand the actual behavior of the app.

### 2. Desktop UI: `desktop/clipx-app`
This is the user-facing desktop application built with Tauri + React.

- `desktop/clipx-app/src/App.tsx` is the main app shell and interaction layer.
- `desktop/clipx-app/src-tauri/` contains the Rust side of the Tauri application.
- The app communicates with the core runtime over IPC and surfaces events back to the React UI.

### 3. Desktop CLI: `desktop/clipx`
This folder contains a Rust CLI that talks to the same IPC service used by the desktop app. It is useful for:

- checking seen devices
- listing trusted peers
- pairing devices
- connecting to peers
- debugging runtime state outside the UI

### 4. Shared protocol: `protos/clipx.proto`
This file defines the protocol for:

- device announcements
- trust/pairing requests
- connection challenge exchange
- clipboard payloads
- file transfer and download requests
- IPC messages between UI and core service

If you want to understand how messages move between devices and the desktop app, this is the contract to read first.

### 5. Mobile support: `mobile/`
The mobile workspace is set up for native Android/iOS integration via a Rust mobile FFI bridge. It depends on the same core logic and is intended to provide the same clipboard and peer features across mobile devices.

---

## Repository layout

```text
clipx/
├── README.md
├── VERSION
├── assets/
├── desktop/
│   ├── Cargo.toml
│   ├── clipx/
│   │   ├── Cargo.toml
│   │   └── src/
│   ├── clipx-app/
│   │   ├── package.json
│   │   ├── src/
│   │   └── src-tauri/
│   └── clipx-core/
│       ├── Cargo.toml
│       ├── src/
│       └── tests/
├── mobile/
│   ├── android/
│   ├── ios/
│   └── native/
├── protos/
│   └── clipx.proto
├── scripts/
├── tests/
├── issues/
│   └── issue.md
└── .github/
```

---

## How to understand the codebase

A good reading order is:

1. `protos/clipx.proto`  
   Learn the wire format and the operational model of the project.

2. `desktop/clipx-core/src/service.rs`  
   This shows how the runtime is constructed and which subsystems are started together.

3. `desktop/clipx-core/src/device/`  
   This is where trusted devices, pairing state, and identity logic live.

4. `desktop/clipx-core/src/clipboard/`  
   Learn how clipboard changes are detected and synchronized.

5. `desktop/clipx-core/src/netio/`  
   This covers local discovery, transport, and peer communication.

6. `desktop/clipx-app/src/App.tsx`  
   This is the main desktop UI and is the best place to understand how app state is presented.

7. `desktop/clipx-app/src-tauri/`  
   This connects the Tauri app to the Rust backend and OS integrations.

8. `mobile/native/` and related Android/iOS pieces  
   These wire the shared runtime into mobile targets.

---

## Key concepts

### Device identity and trust
Each device has an identity and fingerprint. Devices can be discovered, paired, and trusted before they are allowed to sync clipboard content or transfer files.

### Clipboard sync flow
Clipboard changes are detected locally, serialized, and sent through the transport layer to trusted peers. Remote clipboard entries may also be written back into the destination clipboard.

### Network discovery
The desktop runtime discovers peers using LAN/local-network mechanisms and then orchestrates connection setup, handshake, and session startup.

### File transfer
File payloads are not just plain blobs in the main clipboard payload. The protocol uses a file-offer flow with chunked transfer and acknowledgements so large payloads can be sent reliably.

### IPC contract
The UI and service layer communicate through a structured IPC protocol; the `protos/clipx.proto` definition is the canonical contract for those messages.

---

## Prerequisites

You will typically need:

- Rust toolchain
- Cargo
- Node.js and pnpm
- A desktop OS target supported by the project (Windows/Linux/macOS for the desktop app)

---

## Project automation, scripts, and build tooling

### Release and version automation
The repo includes automation for release preparation and version sync:

- `scripts/release.mjs` handles version bumps, checks, build verification, and optional release packaging.
  - supported bump values: `patch`, `minor`, `major`, or a literal `x.y.z`
  - useful flags include `--dry-run`, `--yes`, `--no-push`, `--skip-tests`, `--bundle`, `--android`, and `--allow-branch`
  - it verifies git state, Node/Rust toolchain availability, Android prerequisites when needed, and then runs the workspace checks/build
- `scripts/sync-version.mjs` keeps the repo version synchronized across:
  - `VERSION`
  - `desktop/Cargo.toml`
  - `mobile/native/Cargo.toml`
  - `desktop/clipx-app/src-tauri/tauri.conf.json`
  - `desktop/clipx-app/package.json`

Example:

```bash
node scripts/release.mjs patch --dry-run
node scripts/release.mjs minor --android --bundle
node scripts/sync-version.mjs
```

### Tauri desktop helper scripts
The desktop app contains its own launcher script:

- `desktop/clipx-app/scripts/manage-tauri.mjs`
  - `--sync-icons` copies the generated Tauri icon into the Vite public folder
  - `--clean-dist` removes the frontend build output
  - `--cargo-clean` or `--cargo-clean-all` clears Cargo artifacts
  - `--all` runs the full maintenance sequence

This is used as part of the desktop app lifecycle and build preparation.

### Assets, icons, and configuration
The project keeps branding and runtime metadata in a few places:

- `assets/` contains the design assets for the app branding
- `desktop/clipx-app/src-tauri/icons/` contains the Tauri icon set used for the native desktop app
- `desktop/clipx-app/public/` is used for frontend-published assets and gets an icon copy when `--sync-icons` runs
- `VERSION` is the source-of-truth version string for the repo

This is important because the desktop app and Rust workspace version numbers are kept in sync automatically.

---

## Android build workflow

The Android app is under `mobile/android`, and the Rust mobile bridge is under `mobile/native`.

### Prerequisites
For Android builds you typically need:

- Android SDK + platform tools
- JDK
- Rust target for Android:

```bash
rustup target add aarch64-linux-android
```

- `cargo-ndk` if the Android bridge is being built through the Rust NDK flow:

```bash
cargo install cargo-ndk
```

The project also includes a Gradle property for ABI selection:

```properties
clipxAbis=arm64-v8a
```

This is used in `mobile/android/gradle.properties`, and the comment notes that you can build both targets with:

```bash
cd mobile/android
./gradlew assembleDebug -PclipxAbis="arm64-v8a,x86_64"
```

### Android build commands
Common commands:

```bash
cd mobile/android
./gradlew assembleDebug
./gradlew assembleRelease
./gradlew installDebug
```

The release script also supports Android packaging by running:

```bash
node scripts/release.mjs patch --android
```

which eventually triggers:

```bash
./gradlew assembleRelease
```

from the Android project directory.

### Emulator workflow
The Android project is meant to be run against emulators or physical devices. A typical flow is:

```bash
# list installed emulators
$ANDROID_HOME/tools/bin/avdmanager list avd

# or via emulator binary
emulator -list-avds

# start one
emulator -avd <device_name>
```

Then run the app from Android Studio or Gradle:

```bash
cd mobile/android
./gradlew installDebug
```

The repo also contains an emulator-focused test harness in `tests/README.md` for exercising protocol-level discovery and pairing without needing the full desktop UI.

---

## Emulator and protocol test harness

The `tests/` folder is designed for cross-process validation beyond a single app module.

The emulator tool is documented in `tests/README.md` and is intended for:

- discovery tests
- pairing flow simulation
- connection challenge verification
- protocol debugging without the full desktop app

Example:

```bash
cargo run --manifest-path tests/emulator/Cargo.toml -- --name alice
```

Run a second instance in another terminal to test multiple peer interactions without port collisions. The emulator intentionally uses configurable bind ports so it can coexist with a real `clipx-core` instance on the same machine.

---

## Quick start

### Install dependencies

```bash
# Rust dependencies are managed by Cargo
# Node dependencies are managed with pnpm
```

### Run the desktop app in development mode

```bash
cd desktop/clipx-app
pnpm install
pnpm tauri:dev
```

This starts the React front end and the Tauri desktop app together.

### Build the Rust workspace

```bash
cargo build --manifest-path desktop/Cargo.toml
```

### Run the CLI

```bash
cargo run --manifest-path desktop/clipx/Cargo.toml -- seen
```

### Run Rust tests for the core runtime

```bash
cargo test --manifest-path desktop/clipx-core/Cargo.toml
```

---

## Development notes

- The desktop workspace is defined in `desktop/Cargo.toml`.
- The `clipx-core` crate is the shared execution environment for all desktop and bridge logic.
- The Tauri app is mostly a shell around the native backend and event system.
- The protocol file is the best place to check compatibility between peers.
- The repo uses a shared `VERSION` file and script automation to keep build metadata consistent.
- Known project notes and issue history live in `issues/issue.md`.

---

## Suggested contribution workflow

1. Start from the protocol contract in `protos/clipx.proto`.
2. Trace the runtime starting point in `desktop/clipx-core/src/service.rs`.
3. Follow the relevant subsystem (`device`, `clipboard`, `netio`, `notification`).
4. Verify the desktop UI behavior in `desktop/clipx-app/src/App.tsx`.
5. Keep protocol and implementation changes aligned when touching shared behavior.

---

## Maintainer note

This repository is a real-world multi-platform project rather than a toy sample. The most important thing to understand is that the logic is split between a shared Rust core and thin platform integrations. The protocol and service layer are the central source of truth; the desktop UI and mobile wrappers are consumers of that runtime.

If you're onboarding to the project, start with the protocol and the service coordinator before moving to UI-specific behavior.
