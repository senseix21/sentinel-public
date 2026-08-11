# Architecture Context — 2026-08-10 23:04 +06

Dependency direction is now `common <- storage <- recorder`, with recorder also
depending on media. Cross-domain identifiers are safe lowercase ASCII newtypes.

Recording placement is:
`root/camera-id/YYYY/MM/DD/HH-MM-SS--recording-id.partial.mp4`. The unique
recording identity prevents overwrite when multiple segments begin within one
UTC second. Durable naming removes only `.partial`.
