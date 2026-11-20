# Native Linux capacity boundary on the shared dev VPS — 2026-11-09

**Accepted here: six ten-seat Casual arenas / 60 signed native AI clients for
120 s. The next step, eight arenas / 80 clients, is rejected.** This is a bounded
instrumented loopback result on the existing shared 2-vCPU dev VPS, not a long
soak, WAN test, real/identical RU-host measurement or production SLA. The RU
unit, arena count, firewall and production formula were not changed.

## Exact fixture and artifact binding

The server ran with `MemoryHigh=450M`, `MemoryMax=500M`, `CPUQuota=190%` and
`TasksMax=96`; the native generators also consumed host CPU outside that server
cgroup. PostgreSQL and the ordinary dev lobby/Account API remained present.
Each run created its own restricted `omoba_test` fixture with career schema 13,
portal schema 11 and current migration/grant hashes, then removed the database,
roles and owned processes. Final read-only checks found no capacity executable,
no `omoba_test` database and both ordinary dev services `active`.

- Recorded source parent: `97b119f15cf5d4cb4d44a5a02946bc61d0a3fb0b`.
- Release run `42603752686`; actual server SHA256
  `ecb7648e761f62d31d2c523d2076dd0cff64bbbedaf21853856d1d38e4a6f5ca`.
- Native independent ten-client generator SHA256
  `89660bc17cf77999aec89c3f2ec78052f9a7fdfd4ccc2c436a1398a3eda45c6d`.
- The report's `server_sha256` is the cgroup wrapper,
  `379d0e52ecec617a874247df2271e4fed2af7113406e3d4bac5b0dbe7f97967f`;
  the wrapper summary separately binds the actual server above.
- `native-source.tar.gz` SHA256
  `a32fd7bc46f3deb935d608d06f6b35785c8f2fe321b8a30a850cf91199b54b9e`.
  Its 15 files exactly match every report's source hash map; macOS AppleDouble
  sidecars from the original transport archive are excluded.

Every native peer used ordinary sign-in, unanimous consent, draft/load and signed
20 Hz movement input. There were zero server bots and ten distinct human seats
per Casual arena. Loopback-only admission allowances are disclosed in each
report and are not public-host settings. `OMOBA_LOAD_METRICS=1` was enabled.

## Native results

Input is successful signed UDP sends at the producer. Snapshots are accepted at
the native client; shared bootstrap ticks mean accepted/tick fraction is not a
packet-loss measurement. CPU is the mean of adjacent-sample interval percentages,
where 100% is one logical CPU. RSS is the peak summed lobby + worker RSS, not PSS.

| Run | Verdict | Input min–max /s | Accepted snapshots p50 / p05 /s | Wire p50 / p95 B/player/s | Server CPU mean | Server RSS peak |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 10 clients, 1 arena, 30 s | PASS | 20.0276 | 19.4890 / 19.4890 | 72 155 / 72 576 | 14.09% | 84.76 MiB |
| 40 clients, 4 arenas, 120 s | PASS | 19.9890–19.9933 | 19.2026 / 19.1382 | 86 437 / 88 372 | 50.64% | 213.70 MiB |
| 60 clients, 6 arenas, 120 s | PASS | 19.7334–19.8022 | 18.5699 / 18.4987 | 82 557 / 84 928 | 72.77% | 297.70 MiB |
| 80 clients, 8 arenas, 120 s | **FAIL** | 15.9626–16.5988 | 17.6672 / 17.5833 | 78 504 / 80 436 | 95.20% | 382.23 MiB |

The unchanged 19.5–20.5 Hz per-peer input gate accepts the first three runs and
rejects the fourth. Generator CPU means were 43.81%, 65.80% and 85.81% for the
4/6/8-arena runs; generator RSS peaks were 29.56/44.36/59.25 MiB. Thus the
8-arena rejection describes the whole shared-host fixture, not server CPU alone.

| Run | Application dispatches | Gameplay transforms | Complete snapshots | Signed sequence gaps / restarts | Broadcast gap mean; p95 / p99 upper | Latest-transform age mean; p95 / p99 upper |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 / 30 s | 7 897 | 6 781 | 6 734 | 0 / 0 | 51.276 ms; 75 / 75 ms | 23.141 ms; 45 / 55 ms |
| 40 / 120 s | 116 008 | 99 328 | 96 119 | 0 / 0 | 52.134 ms; 75 / 75 ms | 22.840 ms; 45 / 75 ms |
| 60 / 120 s | 172 409 | 147 586 | 140 566 | 0 / 0 | 53.896 ms; 75 / 75 ms | 25.785 ms; 55 / 150 ms |
| 80 / 120 s | 195 800 | 163 838 | 180 700 | **7 / 0** | 56.416 ms; 75 / 100 ms | 42.607 ms; 150 / 500 ms |

These are final cumulative process-lifetime metrics. Quantiles are exclusive
histogram bucket upper bounds, not exact percentiles; overflow has no finite
upper bound. Latest-transform age after a complete snapshot send is not a
per-command ACK, proof that coordinates changed, or an input-to-snapshot SLA.
All accepted runs had exact durable rosters and healthy final v2 workers. The
rejected run still reached and retained its Running population, but failed the
actual-input gate; its seven signed sequence gaps are retained rather than
hidden. Every wrapper cleanup finished completely.

## Why the older Node probe is not the capacity verdict

Two otherwise equivalent four-arena/40-client/180 s Node runs rejected the old
snapshot-gap p95 < 100 ms gate. Instrumented p50/p95/p99 were
1.687/578.944/669.937 ms; uninstrumented were 1.697/587.583/688.778 ms. Turning
server metrics off did not remove the client-observed stalls. In the instrumented
run, server broadcast mean was 53.043 ms with p95/p99 below 75 ms and zero signed
sequence gaps/restarts, while the single Node process produced only about 1.71
sends/s per peer. Independent native shards pass the same four-arena host step
at 19.989–19.993 sends/s with 19.20 accepted snapshots/s.

The A/B and native control identify the old single-process Node receive/input
loop as the bottleneck for this multi-arena acceptance profile. They do not prove
that all Node timing is invalid, nor do they turn the native result into WAN or
production evidence. Use the native generator for subsequent capacity gates.

## Consequences and remaining gates

The old developer-Mac estimate of about 16 MiB/arena is not transferable to this
Linux fixture. Peak summed server RSS divided by arenas is about 53.43 MiB at
four arenas and 49.62 MiB at six, including the lobby; it is not a pure marginal
per-arena cost. These observations require re-deriving RU memory sizing after a
real/identical RU-host measurement. They do **not** authorize keeping 16 arenas
under the existing 500 MiB cgroup or changing that unit automatically.

Next: a separately reviewed longer-duration native soak at the accepted six-arena
step with an uninstrumented comparison, then the actual or an identical isolated
RU host, vs Bots/mixed workloads and measured uplink/network budget. WAN,
multi-host, party/Ranked/Resume/fault, trust provisioning, UI/physical, C2/C4 and
owner-controlled release remain open. Original finding #13 stays open.

Run `python3 docs/evidence/linux-capacity-2026-11-09/verify.py`. It checks every
archive hash and exact source map, recomputes population/input/wire/producer and
server metrics from the selected worker logs, checks resource derivations,
rosters/health/cleanup and the seven-gap rejection. It starts no process or live
game. The packet contains 43 selected hashed evidence/helper files, not raw bot
logs, private keys, DSNs, binaries or whole unverified output directories.
The controller is a record of the remote fixture, not a portable one-click
production command. Raw outputs and retained binaries remain local under
`.agent/codex-ranked-readiness/linux-capacity/`.
