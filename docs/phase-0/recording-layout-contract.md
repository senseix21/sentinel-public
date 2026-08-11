# Recording Layout Contract

Date: 2026-08-10

`storage` owns recording placement. The first layout is:

```text
<root>/<camera-id>/YYYY/MM/DD/HH-MM-SS--<recording-id>.partial.mp4
```

Normal finalization removes only `.partial`. Camera and recording identifiers
are validated lowercase ASCII domain IDs, so neither can inject separators,
parent components, or platform-specific path syntax. UTC calendar components
are validated, including Gregorian leap-day rules, before formatting.

The recording ID is required even though the timestamp is present: a reconnect
or rapid failure can open more than one segment during the same UTC second.
Filesystem creation, symlink defense, permissions, atomic rename, and durability
remain adapter responsibilities and are not claimed by lexical construction.
