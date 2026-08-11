# Architecture — Validated Construction Boundary

```text
OneCameraSettings (untrusted, complete transaction)
                    |
                    v
OneCameraConfig (all domain invariants validated)
                    |
                    v
camera + bounded distributor + recorder + recovery
```

No runnable partial configuration is returned. The byte budget is conservative:
`(recorder items + live items) * max access-unit bytes + max segment bytes`.
