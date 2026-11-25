# Buffered telemetry restores the measured input20 workload — 2026-11-07

**PASS: all1000 unique signed native AI peers in100 ten-seat Casual arenas
remain Running120s and each sends19.711–20.001 signed transforms/s.** The original
20Hz±2.5% per-peer gate is unchanged. This accepts the local measured input and
population profile; server latency/cadence, long-duration/WAN/RU and the other
release gates retain their scope.

## Reproduced cause and fix

Short diagnostic v6 uses1000 clients/30s. It remains FAIL at18.211–20.000Hz,
median19.256. New cumulative stage measurements isolate telemetry formatting
and output: mean64.484ms, p95 below250ms and p99 below500ms (exclusive histogram
bucket bounds). Prepare/sign/encode/UDP-send p99 are each below1ms; signed
preparation/signing/encoding/send means are0.42/18.77/1.03/8.32µs. Navigation,
session service and receive p99 remain below1ms. V6 loop p99 is below150ms.

Streaming `serde_json::Value` through formatted stdout amplifies small sink
writes. The harness now encodes the complete existing `BOT_SAMPLE ` JSON line
into a buffer, appends its newline and calls `write_all`. It keeps every field,
escapes and complete record, retries interrupted/short writes and propagates
output failure. It does not skip telemetry, inputs or AI behavior to pass.
The regression fixture exceeds4KiB and proves byte-for-byte equality with the
old formatted record, one accepting sink write versus more than20 old formatter
writes, exact reconstruction after interrupted/7-byte writes, and WriteZero
propagation. Sink-call counts are not a trace of kernel syscalls.

V7 repeats a10-client30s control and the full1000-client120s profile after the
fix. The same explicit opt1 graph, current server, restricted PG18.6/schema13/11
fixture and ordinary sign-in/15s unanimous consent/draft/load/game/auth timers
are used. Per-IP/profile loopback allowances remain disclosed in the report;
none is a real public-host setting. There are zero server bots. This is signed
protocol-client Casual AI, not physical human players or Ranked acceptance.

| Observation |v6 diagnostic,1000/30s |v7 full profile,1000/120s |
|---|---|---|
|Verdict |FAIL |PASS |
|Actual input min /median /max Hz |18.211 /19.256 /20.000 |19.711 /19.973 /20.001 |
|Telemetry format/output mean |64.484ms |5.276ms |
|Telemetry format/output p99 exclusive upper bound |500ms |75ms |
|Loop work p99 exclusive upper bound |150ms |25ms |
|Send-gap p95 /p99 exclusive upper bounds |75 /150ms |75 /75ms |
|Overrun rebases, all shards |926 in30s |267 in120s |

V7 control passes20.00365Hz for each of10 peers. The large run reaches all
Running after54.297s and ends174.464s after launch. Its2,397,490 actual transform
gap samples match successful transform deltas;2,795,788 command-stage samples
match actual signed gameplay command deltas.589562 transforms reuse cached
current Running snapshots, at least370 per peer; cached use never advances
receive freshness. Observed authority resets are zero, which the verifier checks directly.
The diagnostic and acceptance intervals differ in length and game progression;
these timings describe the observed local fix, not a statistical host benchmark.

All100 final worker receipts have matching epochs and health v2 with SQL/journal
ready, zero critical/dodge/floor/unpersisted-result backlog, no blocked-since or
closed reply recipient. Durable rosters contain exactly1000 human seats,
100 endpoints and zero server-bot seats. Both owned v6/v7 fixtures are removed;
their final process inventories are empty.

## Actual remaining limits

V7 receives complete snapshots at median14.706Hz/p0513.864Hz. Wire bytes are
p9579786.64B/player/s, aggregate71.567MB/s or572.539Mbit/s. This is received
application/framing traffic, including account traffic and excluding IP/UDP/NIC
overhead; it does not prove a RU uplink or monthly traffic allowance. Shared
snapshot sequence advances for unsent bootstrap reservations, so its ratio is
not a packet-loss measurement. Send success is kernel acceptance, not a server
ACK, and send-gap bounds are not input-to-snapshot latency or an SLA.

The native server process group averages4.662 CPU cores with1577.75MiB peak
summed RSS; generators average3.644 cores with633.95MiB peak. These are this Mac's
process observations, exclude PostgreSQL/container resource use, and do not size
the2-vCPU/4GB RU host. Longer-duration, real/identical Linux/RU hardware/uplink,
server cadence/latency, WAN/party/Ranked/Resume/fault and trust packaging remain.
No production source, dependency, authority, schema, grant, CI configuration,
secrets or RU setting changes here. Production source248 non-harness hashes
still match the prior opt1 manifest; server SHA256 remains
`5c6f4e6fc0795e5008cdd423168b17c515ed93c7b2eda3cd839aa514925c0edd`.

## Verification and artifacts

41 whole-harness tests,4 real-UDP tests,25 Python capacity/lifecycle tests and
4 strict Clippy targets PASS. Command timing counts cannot substitute for sends;
the summarizer rejects contradictory stage counters and retains compatibility
with producers predating these optional diagnostics. Existing v1–v5 failures
remain their own immutable failures. No heavy Cargo/Bevy/SDK build ran; the
small actual harness/library, bot and UDP test reuse the read-only coherent
optimized graph. Root remains below20GiB, so the heavy-build restriction applies.

Run `python3 docs/evidence/input20-buffered-2026-11-07/verify.py` to validate
archive hashes, exact build/bot bindings, recomputed wire/input/timings, all
per-peer bounds, health/rosters and cleanup. It executes no live game. Complete
commands/source hashes are in the manifests. Controllers/check/build helpers
are copied here and still depend on the existing local graph. Raw outputs and
retained binaries remain under `.agent/codex-ranked-readiness/load-input20/`;
no private keys, raw signed bot logs or fixture DSN are archived here.

Original68 remains62 done/5 partial/1 open, plus all critic/UI/physical/off-host/
WAN/trust/RU release requirements. C2/C4 remain review-only with existing owner
decisions pending. Nothing is merged, released or deployed by this packet.
