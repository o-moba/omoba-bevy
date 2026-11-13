# Current optimized1000-playing population — 2026-11-07

**PASS:1000 unique signed native AI peers,100 ten-seat Casual arenas,120s of
simultaneous Running, zero server bots and zero observed auth resets.**
This establishes the local population/roster/SQL-health gate. It does not establish
full performance,20Hz input, RU capacity, physical users or release readiness.

## Build and scope

Source parent30f69c5e. All263 recorded server/store/shared/passport/common source
hashes still match the earlier terminal-replay proof. No production code,
authority, consent, refresh TTL, game/recovery timer, schema, grants or dependency
was changed to achieve this result. The two preceding unoptimized1000 rejections
remain failures in their own packets.

Only eight renderer-free workspace targets were compiled: shared, common,
passport, career-store, server, harness, bots and the transport test. Each uses
explicit opt-level1 with debug assertions and overflow checks enabled, matching
the workspace dev optimization level. Common excludes test-support. Existing
coherent SDK/third-party libraries remain read-only; no Cargo, SDK, Bevy, client
or platform build ran. Source/command/artifact hashes are in `build.json.gz`.
New artifacts occupied75.65MiB; minimum root free space12.24GiB. This bounded
headless compile did not launch a prohibited heavy graphics build or create a
third Cargo cache. It is an explicit local dev-level graph, not a Cargo release
or cross-platform performance certification.

The same fresh PG18.6 fixture runs the10-client control and1000-client probe with
current career13/portal11 migrations, actual restricted game/portal grants and
a nonsuperuser game login. Its160-connection setting is isolated; no existing DB
setting or live RU infrastructure changes. Ordinary sign-in/15s acceptance,
draft/load, server timers and SQL consent are preserved. All ten seats in each
arena are protocol clients; these are not ten physical human players or Ranked.
Existing disclosed per-IP/profile allowances permit the one-source local probe.

## Measured result

| Observation |10-client30s control |1000-client120s population |
|---|---|---|
|All Running after start |18.01s |44.96s |
|Received complete snapshots/player/s, median |17.949 |12.726 |
|Received complete snapshots/player/s, p05 |17.949 |11.969 |
|Received bytes/player/s, p95 |67574 |74123 |
|Observed authority resets / stale Career replies |0 /0 |0 /0 |

The large run ends165.21s after launch.1000 current/fresh/error-free peers and
100 independent Running manifests are checked, with exact identities, endpoints,
joined-session counts and manifest hashes. Every peer's accepted snapshot count
advances under the same epoch/match. All100 final worker health-v2 reports have
matching epochs, SQL/journal ready, no blocked-since, zero critical/dodge/floor/
unpersisted-result backlog and no closed reply recipient. This is an end sample
plus continuous population/freshness checks, not a continuous SQL-latency trace.

Over120.045s of process sampling, server/lobby+workers use3.644 mean CPU cores and
1694.67MiB peak summed RSS;100 generator processes use2.764 cores and625.64MiB.
Summed RSS can double-count shared pages and is not physical RAM or cgroup PSS.
Aggregate observed incoming UDP application/framing payload is60.72MB/s,
485.77Mbit/s, excluding IP/UDP/Ethernet overhead. It is not measured WAN/NIC
capacity. Do not multiply a per-player p95 into a claimed measured total.

## Remaining qualification gaps and metric correction

The generator in `harness/src/bin/bots.rs` sleeps100ms after each iteration and
only drives transforms following received gameplay snapshots. Its full native
input cadence is not measured here and cannot sustain the real client's20Hz/
50ms update cadence (`client/src/net/mod.rs`). **A20Hz real-client input workload
still needs a suitable measured generator and another full-target test.**
Received cadence is also below the nominal20Hz and the10-peer control. No
independent send timestamps, latency distribution, long-duration soak, real RU
host/uplink, WAN, physical-device or rated-party fault acceptance is implied.

The legacy `observed_broadcast_tick_span`/accepted-fraction fields require care.
`runtime/tick.rs::prepare_tick` calls `send_lobby_snapshots` for public workers;
`match_service.rs` increments the shared snapshot sequence every bootstrap pass
**before** filtering authenticated gameplay principals. It advances even when
no player receives a bootstrap. World broadcasts advance the same sequence.
Thus the roughly1Hz sequence/count difference is not proof of packet loss, and
the recorded0.928 accepted/sequence fraction is not a delivery ratio. Archived
numeric values are retained; no claim of zero network loss is made either.
The runner now states these qualification limits explicitly. Those additive
scope labels postdate this live run; exact original reports/hashes are preserved.

Changing optimization across all workspace roots together was a discriminating
comparison against the prior unoptimized graph: the previous auth-reset/stall
symptom did not reproduce here. It does not isolate a particular crate, prove
that overload can never expire auth, or authorize increasing TTL/bypassing checks.

## Cleanup, integrity and reproduction

The corrected disposable controller waits for post-exit group inventory to settle,
records cleanup even on refusal and signals only verified UID/executable/group
members. Transient `(server)` identity labels are left untouched while waiting;
their disappearance no longer aborts audit persistence. The final inventory is
empty and the owned DB/volume is removed. No foreign process was signalled.
The original base DB/image, shared caches and previous raw evidence are retained.

All exact reports/logs and executed build/controller scripts are gzip-compressed
with SHA256 and original byte sizes in `summary.json`. No fixture credential,
auth/session nonce, signature or private key is published. Verify/recompute:

```sh
python3 docs/evidence/optimized-playing-1000-2026-11-07/verify.py
```

This checks the archive and original native deltas; it does not start a new live
test. The exact scripts require this Mac's verified cached graph/prior local
compile records. Raw data and reusable opt1 binaries remain under
`.agent/codex-ranked-readiness/load-opt1/` (server and bots hashes in the summary).
Before rerunning, choose new owned output/container names, retain the isolation
checks and use the assigned cache/disk rules. On another machine use a coherent
headless build from the same source and record its actual profile/provenance.

Original68 counts stay62 done,5 partial,1 open. Full current CI and all critic,
UI/physical/WAN/trust/recovery/RU release gates retain their original scope.
