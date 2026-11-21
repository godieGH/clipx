---
id: efc03763e723912dd1e57f8a78c8aa0e4a6e2662
title: Staging a large file blocks the clipboard manager and is copied twice on Android
status: report
weight: medium
created: 2026-11-21T17:19:55Z
---

store_file_offer_from_path runs fs::copy inside the manager's event loop, so a large file stalls history requests and incoming items while it copies. On Android the file is also copied twice: into the app cache, then into the core staging folder. The UI only shows Preparing with no progress.
