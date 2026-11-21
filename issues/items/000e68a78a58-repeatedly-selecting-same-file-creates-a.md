---
id: 000e68a78a58e10f11adc588fb2c48fbf1b5b0f1
title: Repeatedly selecting same file creates a duplicate staged copies
status: fixed
weight: medium
created: 2026-11-21T13:06:52Z
comment: Selecting the same unchanged file multiple times creates unnecessary duplicate copies in the sender staging.
---

**Observation**:
On device A, selecting a file for sending copies it into ClipX's sender-side staging directory.

Selecting the same unchanged file again while tge earlier one staged offer is still present creates another staged copy,
even though the file might have the same contents and are identical in size

**Expected behavior**:
ClipX should avoid storing another full copy of identical staged content when an existing staged copy can be reused

**Update:** Fixed by comparing bytes only when sizes match: Android pending list, desktop pending list, and core staging (shares the blob via hard link). Each send still creates it's own offer. Fixed in 4cc6c44 and 5a183b6
