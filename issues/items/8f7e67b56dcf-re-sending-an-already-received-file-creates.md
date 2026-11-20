---
id: 8f7e67b56dcf7b1c72661ca77f851a458864b6ba
legacy: 0007
title: Re-sending an already received file creates duplicate copies
status: report
weight: medium
created: 2026-11-20T20:52:10Z
comment: Re-sending an already received file creates duplicate copies in internal storage.
---

**Observation:** When a device downloads a file offered by another device, ClipX stores it in internal storage. If the same file is sent again later, ClipX does not detect the existing copy and stores it again.

Expected behavior: ClipX should detect whether identical file content already exists before storing another copy. If it does, it should avoid creating an unnecessary duplicate. Detection should be based on file content rather than filenames alone.
