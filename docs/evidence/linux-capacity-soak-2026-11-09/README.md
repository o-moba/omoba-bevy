# Native Linux five-minute soak and metrics A/B — 2026-11-09

**The six-arena/60-client uninstrumented run passes 300 s. The otherwise
corresponding instrumented run fails the unchanged per-peer input gate at
19.4589 sends/s.** Both retained their complete Running population. This is a
borderline shared-dev-host result, not an unconditional six-arena acceptance,
a real/identical RU-host measurement, WAN test or production SLA. The RU unit,
16-arena setting, firewall and production formula were not changed.

## Exact fixture and artifact binding

The same release server, native generator and cgroup wrapper as the preceding
[capacity-boundary packet](../linux-capacity-2026-11-09/README.md) were used.
The server ran with `MemoryHigh=450M`, `MemoryMax=500M`, `CPUQuota=190%` and
`TasksMax=96`; generators consumed additional host CPU outside that cgroup.
Each run independently created and removed a restricted `omoba_test` fixture
with career schema 13, portal schema 11 and the recorded migration/grant hashes.
Final checks found no `omoba-server`/`bots` process, no `omoba_test` database and
both ordinary dev services `active`.

- Recorded source parent: `4a0f2ca616a678371bd4d6f2798e117e03357f57`.
- Release artifact run `42603752686`; actual server SHA256
  `ecb7648e761f62d31d2c523d2076dd0cff64bbbedaf21853856d1d38e4a6f5ca`.
- Native independent ten-client generator SHA256
  `89660bc17cf77999aec89c3f2ec78052f9a7fdfd4ccc2c436a1398a3eda45c6d`.
- Reported server-wrapper SHA256
  `379d0e52ecec617a874247df2271e4fed2af7113406e3d4bac5b0dbe7f97967f`.
- The 15-source archive SHA256 is
  `10b812ba55db0d9d3c9d335ea335cadcadf819ba711c835058aadf163788494b`.
  It includes the reviewed runner accepting 15–300 s and matches both reports;
  AppleDouble sidecars are excluded. Rust sources and measured binaries did not
  change between the preceding capacity packet and this source parent.

Every peer used ordinary sign-in, unanimous consent, draft/load and signed 20 Hz
movement input. There were zero server bots and ten distinct human seats per
Casual arena. Loopback-only admission allowances are disclosed in each report.

## Results

Input means successful signed UDP sends at the producer. Snapshots are accepted
at the native client; shared bootstrap ticks mean accepted/tick fraction is not
packet loss. CPU is the mean of adjacent-sample interval percentages, where
100% is one logical CPU. RSS is peak summed lobby + worker RSS, not PSS.

| Server metrics | Verdict | Input min–max /s | Accepted snapshots p50 / p05 /s | Wire p50 / p95 B/player/s | Server CPU mean | Server RSS peak | Generator CPU mean |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Enabled | **FAIL** | 19.4589–19.5355 | 18.3411 / 18.3115 | 86 898 / 89 040 | 72.27% | 300.60 MiB | 66.98% |
| Disabled | PASS | 19.5349–19.6209 | 18.4171 / 18.3927 | 87 783 / 89 275 | 72.46% | 299.86 MiB | 69.11% |

The first run missed the lower 19.5 Hz bound by only 0.0411 Hz, but the gate was
not relaxed and its failure is retained. The second passed the same gate. This
single A/B does **not** establish server instrumentation as the root cause:
server CPU was 0.19 percentage points higher without metrics and generator CPU
was 2.13 points higher. The minimum input rate moved by only 0.0760/s. Treat the
outcome as marginal shared-host variability requiring host-specific acceptance,
not proof that enabling or disabling metrics fixes capacity.

Both runs retained all 60 signed peers, six exact durable rosters and healthy
final v2 workers. Both completed process/database/role cleanup. In the
uninstrumented run, signed server dispatch/cadence is explicitly unverified;
absence of `LOAD_METRICS` is checked rather than presented as zero gaps.

## Instrumented server observations

The rejected run's worker logs still permit an independent server-metric
reconstruction even though the runner stopped at the input assertion before
copying `server_load` into its report:

- 416 943 verified application dispatches, 356 001 gameplay Transform handler
  entries and 337 456 complete snapshot sends;
- **0 signed sequence gaps and 0 restarts**;
- 33 897 world broadcasts; broadcast-gap mean 54.497 ms, p95/p99 exclusive upper
  bounds 75/100 ms;
- latest-transform age at snapshot mean 26.038 ms, p95/p99 exclusive upper
  bounds 55/150 ms.

These are cumulative process-lifetime measurements. Quantiles are exclusive
histogram bucket upper bounds, not exact percentiles; overflow has no finite
upper bound. Latest-transform age is not per-command ACK latency, proof that a
Transform changed coordinates, or an input-to-snapshot SLA.

## Network scope and remaining gates

At 60 players, median per-player wire rates correspond to aggregate application
traffic of about 41.711 and 42.136 Mbit/s for the two runs. Multiplying each
p95 per-player rate by all 60 players gives 42.739/42.852 Mbit/s. These figures
include account/framing traffic but exclude IP/UDP/NIC overhead and describe
loopback, not measured uplink capacity or Internet loss/latency.

Next remains an actual or identical isolated RU host, vs Bots/mixed workloads,
measured uplink/network budget and an acceptance decision that does not hide
the instrumented rejection. WAN, multi-host, party/Ranked/Resume/fault, trust
provisioning, UI/physical, C2/C4 and owner-controlled release remain open.
Original finding #13 stays open.

Run `python3 docs/evidence/linux-capacity-soak-2026-11-09/verify.py`. It checks
all 20 selected evidence/helper files and hashes, the exact source map, remote
fixture binding and cleanup, recomputes input/wire/population/resources and the
instrumented server metrics, and verifies that the uninstrumented logs contain
no server metric records. It preserves the FAIL/PASS distinction and starts no
server or live game. Raw outputs and binaries remain local under
`.agent/codex-ranked-readiness/linux-capacity/`.
