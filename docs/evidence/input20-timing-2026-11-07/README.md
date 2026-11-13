# Actual input20 repetitions and producer timing — 2026-11-07

**All completed1000-client120s attempts remain REJECTED by the unchanged
19.5–20.5 successful signed transforms/player/s gate.** The nonblocking receive
change improved some observations but did not establish full-profile capacity.
All attempts retained1000 signed AI peers in100 ten-seat Casual arenas, with
zero server bots, matching durable rosters and no observed authority resets.
This is local controlled protocol-client evidence, not real humans, Ranked,
physical devices, WAN, sustained SLA or RU-host qualification.

| Attempt |Control |1000-client verdict |Actual input min /median /max Hz |Received snapshots median Hz |
|---|---|---|---|---|
|v2, original bounded nonblocking driver |10/30s PASS20.001Hz |FAIL |18.771 /19.196 /19.583 |11.731 |
|v3, cumulative timing added |10/30s PASS20.054Hz |NOT STARTED: occupied UDP port |Not measured |Not measured |
|v4, same timed binary, fresh36500/36600 ports and independent DB |Reuse v3 control; no new control |FAIL |19.346 /19.603 /19.869 |14.391 |
|v5, navigation/session timing and signed-gap regression |No new control; diagnostic repetition |FAIL |18.819 /19.215 /19.611 |12.798 |

V2 reaches the full population after44.397s and ends164.591s after launch;
v4 reaches it after47.125s and ends167.326s; v5 after46.168s/166.440s. The earlier
blocking v1 failure remains unchanged in the prior input20 handoff packet.
Every fixture here confirms complete cleanup, empty final owned-process
inventory and removal of its owned disposable PostgreSQL container/volume.
V3's preserved original preflight log proves address-in-use before any large
server starts. Its controller subsequently fails reading the absent report;
that secondary exception is not a game/admission failure. No foreign process
was stopped. Preflight now reports the actual unavailable port with a clear
exit2; its real occupied-socket test proves SQL/process startup is not reached.

## New measurements

The producer now records bounded cumulative wall-duration histograms. The
capacity runner subtracts the exact per-shard baseline before aggregation;
warm-up stalls cannot dilute soak quantiles. Exclusive bucket upper bounds
are1/5/10/25/45/55/75/100/150/250/500/1000ms, plus unbounded overflow. These are
**quantile bounds**, not exact p95/p99 durations. Wall time includes OS
descheduling and blocking, so it cannot establish the CPU cost of a function.

Successful signed Transform sends record actual inter-send gaps, including
idle/dead poses. Ping, absent authority and failed sends do not fabricate gap
samples. Every soak's gap-count delta must equal its actual transform-count
delta. Input rate and snapshot receive rate remain separate. A legacy producer
without the new timing explicitly reports that timing was not measured.

V5 records exactly230680 measured shard loops and2306800 receive/session
calls. Loop work averages5.240ms with p99 below150ms. Receive work averages
131.7µs and session work4.22µs, both p99 below1ms.1065696 navigation calls
average9.61µs, p99 below1ms.
The producer rebases an overrun3966 times during soak. Actual send-gap p95 is
below75ms and p99 below150ms;129 gaps are in500–1000ms. No gap reaches the
unbounded≥1000ms bucket in this interval. These observations rule out attributing
the bulk of stalls to navigation without more evidence; rare navigation or OS
stalls still exist. V4 separately records2719 rebases and p99 loop work below100ms.

A single verified owned v5 generator (index54, PID/UID/process group/executable
checked) was sampled for3s at1ms during the diagnostic run. Its call graph is
retained without the machine header/image inventory.2060 of2412 captured stacks
are under sleep; most other sampled stacks are receive/decode, with a smaller
signed-send path. A3s sample does not identify all rare stalls or qualify other
generators. V5 includes this profiling overhead and is not an uninstrumented
performance comparison with v2/v4.

**Next:** instrument signing/serialization, actual UDP send and telemetry output
separately before choosing an optimization. Use a short explicitly diagnostic
full-population interval to locate stalls; repeat the full120s acceptance profile
after a substantiated fix. Keep the per-peer gate and real game/auth/consent/
recovery timers unchanged. Do not infer packet loss from the shared snapshot
sequence, which reserves unsent bootstrap ticks. Per-command server acceptance,
input-to-snapshot latency, long-duration/Linux/RU/uplink/WAN remain open.

## Provenance and verification

Production server binary remains SHA256
`5c6f4e6fc0795e5008cdd423168b17c515ed93c7b2eda3cd839aa514925c0edd`.
The opt1 manifest contains248 non-harness Rust sources, all rechecked unchanged
before this continuation. Its263 total included15 harness sources; older prose
calling all263 production sources overstates that set. Harness subsequently
changes here; no production source, dependency, authority, schema, grant, CI
configuration or RU setting changes in this packet.

`nonblocking-build.json.gz` belongs to v2; `timed-build.json.gz` to v3/v4;
`build.json.gz` to v5. Each report binds its actual bot hash. Builds reuse the
existing coherent opt1 graph and read-only dependencies; no Cargo/Bevy/SDK or
platform build, third Cargo cache or cleanup of shared artifacts ran. Root
remains approximately12GiB free, below the heavy-build threshold.38 whole-harness,
4 real-UDP,25 Python and4 strict Clippy targets PASS. The new deterministic
histogram tests cover boundaries/saturation, Python tests reject backwards/
malformed counters and fabricated send-gap counts; the signed UDP regression
checks one genuine gap after two real poses, with Ping/reset adding none.
One initial occupied-port test used an absent `/bin/true` on this Mac and failed
before its intended assertion; the portable `sys.executable` fixture passes.

Run `python3 docs/evidence/input20-timing-2026-11-07/verify.py` for archive/hash,
counter/rate/timing, exact rejection and cleanup verification. This verifies
archived evidence and runs no live game. The controller/check copies document
the actual commands and depend on the existing local graph. Owned raw outputs
and retained v2/v4/current binaries are under
`.agent/codex-ranked-readiness/load-input20/`; no local probe remains running.
No private keys, raw signed bot logs or fixture DSN are published here.

Original68 remains62 done/5 partial/1 open. Critic/UI/physical/off-host/trust/RU
release gates and existing C2/C4 owner decisions retain their full scope.
