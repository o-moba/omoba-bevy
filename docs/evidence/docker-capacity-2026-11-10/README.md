# Docker capacity rehearsal, 4 CPU / 4 GiB — 2026-11-10

**Controlled-host result: 600 Casual clients / 60 arenas pass the unchanged
20 Hz input gate for 300 s; 800 / 80 fail it on repetition. This is native ARM64
loopback evidence, not RU x86_64-host, uplink, WAN or production-SLA acceptance.
The release decision remains NO SHIP and the RU unit was not changed.**

## Exact candidate and fixture

- Reviewed source parent: `546c1dae241726e29fef89dd4e0ca95a44fb0c6c`.
- Temporary branch `agent/docker-arm64-capacity` added only the archived ARM64
  workflow; its source check excluded that workflow and required every other
  path to match the source parent. GitHub run `42753061924` succeeded.
- Artifact `linux-arm64-capacity-546c1dae`: server SHA-256
  `fee88530f0c2db49243957fc7d0ffc629d972180a4d720db0034c1e207baac2f`,
  native bots `cb7f795a52f65699f3ccef512a7a2e524d9ef8573d02a85b4a1a674feac598d9`,
  Ranked helper `34bac34852224e67f22590618d55ce77882352853e0e2acb0a9f928982d68518`.
  All three were checked as ELF ARM aarch64 executables.
- One Docker container, `linux/arm64`, `cpu.max=400000 100000`,
  `memory.max=4294967296`, swap disabled, `pids.max=512` and Docker network
  `none`; only loopback was used. Lobby port 61000 and worker ports from 61010
  stayed outside the container's ephemeral range 32768–60999.
- PostgreSQL 16.15, lobby, workers and native generators shared the same cgroup.
  `max_connections=256`; each run created and removed disposable `omoba_test`
  and restricted roles with career schema 13, portal schema 11 and recorded
  migration/grant hashes. No production service, database, key or RU host was
  touched.
- Every accepted run used ordinary sign-in, unanimous consent, draft/load and
  signed 20 Hz native-AI movement input. Ranked/mixed controls used disclosed
  synthetic prior Casual receipts; they do not prove organic eligibility or
  rated settlement.

CPU is the mean of adjacent cgroup-sample interval percentages; 100% is one
logical CPU. Memory below is maximum `memory.current`, including PostgreSQL and
generators. Input means successful signed UDP sends at the producer. Snapshots
are accepted by native clients. Shared bootstrap ticks mean accepted/tick
fraction is not packet loss. Latest-transform age is not per-command ACK or
input-to-snapshot latency.

## Workload controls

| Profile | Clients / arenas | Seconds | Verdict | Input min–max /s | Snapshots p50 /s | Memory max MiB | CPU mean % |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| Casual | 10 / 1 | 30 | PASS | 20.0478–20.0478 | 17.817 | 174.40 | 10.91 |
| vs Bots | 10 / 10 | 30 | PASS | 19.9986–19.9986 | 17.728 | 240.70 | 30.79 |
| Ranked | 10 / 1 | 30 | PASS | 20.0486–20.0486 | 17.903 | 175.55 | 12.97 |
| Mixed | 30 / 12 | 30 | PASS | 19.9961–20.0485 | 17.691 | 265.63 | 41.44 |

All four controls had zero signed sequence gaps/restarts and zero OOM events.
They are smoke controls, not sustained per-workload capacity acceptance.

## Casual stepped results

Every accepted row below had zero signed sequence gaps/restarts, zero OOM kills,
exact durable rosters and complete process/database/role cleanup.

| Clients / arenas | Seconds | Verdict | Input min–max /s | Snapshots p50 /s | Memory max MiB | CPU mean % |
| ---: | ---: | --- | ---: | ---: | ---: | ---: |
| 40 / 4 | 120 | PASS | 19.9992–20.0125 | 17.804 | 219.05 | 40.80 |
| 60 / 6 | 120 | PASS | 20.0113–20.0128 | 17.747 | 245.39 | 48.35 |
| 80 / 8 | 120 | PASS | 19.9995–20.0006 | 17.659 | 266.24 | 60.27 |
| 100 / 10 | 120 | PASS | 19.9997–20.0019 | 17.615 | 294.94 | 71.15 |
| 120 / 12 | 120 | PASS | 19.9997–20.0137 | 17.692 | 325.50 | 81.96 |
| 140 / 14 | 120 | PASS | 19.9996–20.0113 | 17.736 | 353.96 | 89.69 |
| 160 / 16 | 120 | PASS | 19.9996–20.0005 | 17.826 | 378.72 | 103.32 |
| 180 / 18 | 120 | PASS | 19.9998–20.0134 | 17.951 | 404.92 | 116.32 |
| 200 / 20 | 120 | PASS | 19.9995–20.0003 | 18.087 | 438.79 | 127.06 |
| 220 / 22 | 120 | PASS | 19.9994–20.0007 | 18.191 | 464.38 | 139.15 |
| 240 / 24 | 120 | PASS | 19.9992–20.0000 | 18.225 | 490.53 | 145.26 |
| 260 / 26 | 120 | PASS | 19.9994–20.0005 | 18.243 | 522.62 | 156.55 |
| 280 / 28 | 120 | PASS | 19.9994–20.0121 | 18.271 | 547.43 | 165.92 |
| 300 / 30 | 120 | PASS | 19.9998–20.0126 | 18.277 | 576.85 | 174.24 |
| 400 / 40 | 120 | PASS | 19.9996–20.0003 | 18.297 | 705.62 | 237.17 |
| 500 / 50 | 120 | PASS | 19.9994–20.0002 | 18.364 | 842.14 | 288.49 |
| 600 / 60 | 120 | PASS | 19.9759–20.0000 | 17.454 | 976.21 | 344.95 |
| 700 / 70 | 120 | PASS, degraded | 19.7561–19.9847 | 12.686 | 1119.60 | 355.14 |
| 800 / 80 | 120 | **FAIL** | **13.9668–17.1602** | **9.141** | 1390.12 | 372.84 |

The 700 row formally passed the 19.5–20.5 input gate but is not a comfortable
production step: snapshot cadence dropped sharply, with 1281 newly throttled
periods and about 742.8 s of cumulative throttled-time delta. At 800 the second
run failed the input gate; reconstruction of its retained worker logs found
707 signed sequence gaps and zero restarts. This confirms missing application
dispatch as well as low producer cadence, but does not by itself identify
where packets or commands were lost. The 800 fixture reached 498 PIDs, close to the 512 limit. The
already failed 800 step made 900/1000 on this fixture unnecessary and proves no
1000-client capacity for this host profile.

At 500 clients, median per-player application traffic corresponds to about
338.70 Mbit/s aggregate; the 600-client soak gives 397.99 Mbit/s. These are
loopback application-byte estimates excluding IP/UDP/NIC overhead, not measured
uplink capacity.

## Sustained runs and natural victories

| Label | Requested | Result | Input min–max /s | Snapshots p50 /s | Memory max MiB | CPU mean % |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| `casual-600-300` | 600 / 60, 300 s | **PASS** | 19.9616–20.0000 | 17.331 | 1226.10 | 370.36 |
| `casual-500-300` | 500 / 50, 300 s | FAIL: natural victory | — | — | 891.60 | 293.27 |
| `casual-500-300-v2` | 500 / 50, 300 s | FAIL: natural victory | — | — | 875.58 | 291.47 |

The 600 soak had zero gaps/restarts/OOM kills and full cleanup, but used 370.36%
mean CPU, recorded 2073 newly throttled periods and about 596.56 s of cumulative
throttled-time delta. It is the upper sustained step passing the current input
gate, not a recommended production limit with headroom.

The runner requires every peer to remain `running` for the full requested soak.
In the first 500 attempt one arena legitimately ended with Blue's victory at
156.656 s; 49 other arenas were still running. In the repetition Green won at
130.148 s. These are retained as scenario FAILs, **not capacity failures and not
hidden successes**. They show that a future long-lived capacity scenario must
handle completed games/requeue explicitly rather than repeat until no arena
wins. Consequently 500/50 has a strong 120 s result and resource headroom, but
no completed 300 s continuous-Running acceptance here.

For this controlled fixture, 50 arenas is therefore a candidate operating step
with headroom, while 60 is the demonstrated upper input-gate step and 70 is
already degraded. The existing RU setting of 16 arenas / 500 MiB is not changed
or accepted: 160/16 passed for 120 s at 378.72 MiB, but sustained 16-arena and
native x86_64 RU-host evidence is still required.

## Preserved rejected fixtures and transient read

The following are diagnostics, not additional game-capacity failures:

1. `casual-10-control`: incomplete source copy (`harness/src/bin/bots.rs`
   missing); corrected before `casual-10-control-v2`.
2. `casual-80-120`: the game probe passed, but the old process-argument cleanup
   touched a PostgreSQL backend and no wrapper summary was written. Cleanup now
   owns only `/work/omoba-server` and `/work/bots` executable identities;
   `casual-80-120-v2` passed with full cleanup.
3. `casual-120-120`: the controller omitted `--allow-large`; no game report was
   produced. `casual-120-120-v2` passed.
4. `casual-240-120`: worker ports overlapped ephemeral UDP ports; 23 arenas
   worked and repeated workers logged `AddrInUse`. Moving the range to
   61000/61010 made `casual-240-120-v2` pass.
5. `casual-800-120`: final verification transiently could not read an
   allocation's `status.json`; pre-cleanup diagnostics observed that same
   allocation `running`, and the file existed after teardown. This attempt is
   retained as a transient-read diagnostic, not a performance verdict.
   `casual-800-120-v2` then supplied the actual input-cadence FAIL.

## Verification and remaining release gates

Run:

```bash
python3 docs/evidence/docker-capacity-2026-11-10/verify.py
python3 -S docs/evidence/docker-capacity-2026-11-10/verify.py
```

The standard-library verifier checks every archived byte/hash, all 30 cgroup
summaries and cleanup records, the 43-source snapshot, 19-file report-source map
and exact binary/workflow
binding. For ten selected runs it restores the archived runner, recomputes
population, input, wire, producer timing and server metrics from reports and
worker logs, and checks the natural-victory and transient-read diagnostics. It
starts no server, database or load. The packet contains no binaries, private
keys or DSNs.

Original finding #13 remains open. Still required: sustained per-workload
acceptance on the actual or identical isolated RU x86_64 host, external uplink
with network overhead, WAN/multi-host/fault scenarios, independent recovery,
UI/physical-device gates, C2/C4 decisions, human review and owner-controlled
release. No merge, release publication, production-limit change or RU rollout
resulted from this rehearsal.
