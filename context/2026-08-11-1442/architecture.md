# Architecture — Observability Boundary

Libraries emit typed structured transition events. They do not choose global
formatting, filtering, or a backend. The server application owns subscriber
installation. Hot media paths increment counters and do not log each packet.

Final pipeline evidence carries state, packet-loss counters, queue depth/drop
counters, segment outcomes, clock epochs, and persisted bytes.
