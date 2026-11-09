# Server dispatch, latest-transform age and broadcast cadence — 2026-11-09

**PASS for the instrumented loopback profile: 1000 unique signed native AI
clients in 100 ten-seat Casual arenas remain Running for 120 s, every client
sends 19.834–20.000 transforms/s, and all 100 workers report zero signed
sequence gaps and zero sequence restarts.** The unchanged input gate is
19.5–20.5 Hz per peer. A 10-client/30 s control also passes at 20.026 Hz.

This advances the previous producer-only measurement. It is not a production
SLA, per-command ACK-latency measurement, physical-player test or RU-host
capacity qualification.

## What the server now measures

`OMOBA_LOAD_METRICS=1` enables diagnostics; otherwise the server retains no
per-endpoint metric state. No wire fields, gameplay timers or rules change.
Once per second each worker writes a cumulative `LOAD_METRICS` JSON record to
stderr. The capacity runner requires one valid sequence of records from every
live worker, rejects malformed/missing records and fails the run on any signed
sequence gap or restart.

- `transport_commands`: verified signed commands reaching application dispatch;
  includes commands other than Transform.
- `gameplay_transforms`: Transform commands entering the ordinary gameplay
  handler. It does not prove that coordinates changed.
- `latest_transform_age_at_snapshot`: age of the latest gameplay Transform when
  a complete snapshot finishes kernel send. It is **not** a separate latency or
  ACK for every Transform.
- `broadcast_gaps`: actual world-broadcast start intervals, including broadcasts
  with no recipients. Snapshot counts include only complete fragment sends.

The report retains per-worker and per-endpoint counters/histograms, source and
binary hashes, exact scope, and explicit unverified packet-delivery/WAN/RU/SLA
boundaries. Quantiles are exclusive histogram upper bounds, never exact
percentiles. Server histograms cover process lifetime rather than a soak-only
baseline subtraction.

## Observed full profile

| Measurement | 1000 clients / 120 s |
|---|---:|
| Actual signed Transform sends, min / median / max | 19.834 / 19.968 / 20.000 Hz |
| Verified signed application dispatches | 3,166,007 |
| Gameplay Transform handler entries | 2,712,856 |
| Sequence gaps / restarts | 0 / 0 |
| Complete snapshot kernel sends | 1,858,997 |
| Latest-transform-age observations | 1,854,112 |
| Latest-transform age mean | 34.868 ms |
| Latest-transform age p95 / p99 exclusive upper bounds | 250 / 500 ms |
| World broadcasts / measured gaps | 188,205 / 188,105 |
| Broadcast gap mean | 73.596 ms |
| Broadcast gap p95 / p99 exclusive upper bounds | 250 / 500 ms |
| Client accepted snapshots, median / p05 | 13.025 / 12.018 Hz |
| Aggregate received application/framing traffic | 62.458 MB/s = 499.660 Mbit/s |
| Per-player received traffic p50 / p95 | 61,424 / 75,215 B/s |

Aggregate traffic is the sum of per-peer average receive rates over the soak;
it includes account traffic and excludes IP/UDP/NIC overhead. One thousand
clients each at the observed per-player p95 would require about 601.723 Mbit/s
before network headers; this is a scaling calculation, not a measured aggregate
p95 or an uplink guarantee. Shared snapshot sequence includes unsent bootstrap
reservations, so accepted/tick ratios are not packet-loss measurements.

The opt-in per-second JSON reporting adds instrumentation overhead. Together
with macOS loopback scheduling and the explicit opt1/debug-assertion build, it
can affect cadence; this run must not be presented as an uninstrumented release
benchmark. Received cadence remains below the nominal 20 Hz input profile and
is evidence for further host/network investigation, not something hidden by the
PASS verdict.

All 100 final worker receipts have matching healthy v2 state, SQL/journal ready,
zero critical/dodge/floor/unpersisted-result backlog and no closed/blocked reply
recipient. Durable rosters contain exactly 1000 human seats and zero server
bots. Observed authority resets are zero. The disposable PostgreSQL 18.6 fixture
uses career schema 13, portal schema 11 and restricted runtime grants. Owned
process groups and the container/volume are removed; final cleanup inventory is
empty.

## Verification and artifacts

- `cargo fmt --all` passes.
- The full direct server test binary passes **562 tests, 0 failed, 16 ignored**.
- Python capacity/summarizer tooling passes **12 tests**, including missing and
  malformed metrics and explicit nonzero-gap reporting.
- The same small coherent opt1 direct-rustc helper builds the changed server and
  its tests without a heavy Cargo/Bevy/SDK build or another Cargo cache.
- `python3 docs/evidence/server-load-metrics-2026-11-09/verify.py` checks every
  compressed hash, binary/source binding, population, input/wire/producer summary,
  roster, health receipt and cleanup result. It loads the packet's own archived
  `public_playing_capacity.py` and imports its helper from the packet's `sources/`
  snapshot. All 15 source hashes, including `public_bots_rehearsal.py`, resolve
  only to archived copies: the current checkout is neither imported nor used as
  a fallback. The verifier reconstructs the observation-time metric logs from
  archived `LOAD_METRICS` lines and recomputes every server aggregate; it starts
  no live game. Both normal verification and a sandbox with every current source
  replaced and both current Python helpers raising on import pass.

The packet contains complete compressed controller reports and isolated-fixture
summary, observation-time server metric records, build manifests, test logs,
copies of the controller/build/test helpers and the complete 15-file historical
source snapshot. Raw outputs and retained binaries
remain under `.agent/codex-ranked-readiness/load-server-metrics/`; no private
keys, complete signed bot logs or fixture DSN are archived. Full worker logs can
gain ordinary lines after the metric summary and before teardown, so their final
local hashes need not equal the observation-time hashes stored in the report.

Longer-duration Linux/real-or-identical RU host and uplink tests, WAN/multi-host,
party/Ranked/Resume/fault, directory/trust provisioning, independent off-host
recovery, current UI/physical-device checks and owner-controlled release remain
open. Original findings remain 62 done/5 partial/1 open; C2/C4 still require the
existing owner decisions. Nothing is merged, released or deployed by this packet.
