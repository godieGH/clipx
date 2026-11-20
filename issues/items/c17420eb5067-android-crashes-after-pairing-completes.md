---
id: c17420eb50679f25574575d75d6b4ecb5b635638
legacy: 0006
title: Android crashes after pairing completes
status: fixed
comment: Android crashes after pairing completes
---

**Observation**: just after pairing process is finished successful, android app activity crashes with a crash log:
```log
Process: com.godiegh.clipx, PID: 21610
java.lang.IllegalArgumentException: Key "2afeeba80c14a2f151b54116b1187f230768a66daa47c6233fc46ba0bbe44ddb" was already used. If you are using LazyColumn/Row please make sure you provide a unique key for each item.
```

This log seems to report a reuse of an existing item key in the list items ui components. not sure which component exactly and in which code or file, but it is related to the device screen.

**Normally**: In the device screen there are two lists ui components, one that show trusted devices plus their states i.e connect, disconnect or unavailable. And the other is the seen devices (discovered device). Pairing moves device from available list to trusted list. These lists use items keys(fingerprint) as their item id. What is known about jet compose LazyColumn/Row each list maintains its own keys/id, even if they tend to appear same in the two lists they are not a problem, unless there is a key duplicate on a single list. I think that is where the crash comes from, but the question is how?

This bug has to be fixed, although it might seem non-serious at the moment, in terms of there is no obvious data loss observed(re-launching the app tends to continue from where it crashed on and not from pre-pairing state), but the crash happens.
