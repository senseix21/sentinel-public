# Architecture Context — 2026-08-10 23:06 +06

The tested encoded-media dependency flow is:

```text
RTP/H.264 payload -> camera depayloader -> media access-unit assembler
                  -> media distributor -> recorder consumer -> H.264 sink
```

The test uses complete access units at the distribution boundary and proves
that live queue eviction does not alter recorder delivery.
