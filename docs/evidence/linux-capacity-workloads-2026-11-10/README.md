# Exact-Release Linux Casual/Ranked/vs Bots/mixed workloads — 2026-11-10

This packet records three 30-second Running-population workload controls on the
shared 2-vCPU Linux dev VPS. It is **not** an isolated RU-host, uplink, WAN,
long-soak, physical-device, organic Ranked-eligibility, rated-settlement or
production-SLA acceptance. #13 remains open and the RU per-arena table remains
unfilled.

## Candidate and live results

All binaries came from Release run `42657406745` at source parent
`73651bc6281aa5eac03d0094b3061f6cee1cadf8`. The Release run completed
successfully for macOS, Windows, Linux and Android. The Linux artifact contained
both `bots` and `examples/qualify-ranked-fixture`; the server came from the same
run's Linux package.

| Workload | Signed humans | Generator shards | Real arenas | Server bots | Actual sends/s, min–max | Gameplay transforms | Sequence gaps / restarts | Result |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| vs Bots | 10 | 1 | 10 | 90 | 19.9967–19.9967 | 6927 | 0 / 0 | PASS |
| Ranked | 10 | 1 | 1 | 0 | 19.9991–19.9991 | 6761 | 0 / 0 | PASS |
| Mixed | 30 | 3 | 12 | 90 | 19.9941–19.9985 | 20671 | 0 / 0 | PASS |

Mixed contained exactly one Casual shard, one Ranked shard and one vs Bots
shard: one Casual + one Ranked + ten vs Bots workers. Every expected durable
roster and signed metric worker was present. Selected health receipts are v2
with SQL and journal ready. Ranked and mixed explicitly seeded ten synthetic
prior Casual receipts per Ranked profile; this is not organic eligibility proof.

The server wrapper requested the previous diagnostic cgroup limits:
`MemoryHigh=450M`, `MemoryMax=500M`, `CPUQuota=190%`, `TasksMax=96`. Sampled
aggregate server RSS peaked at 474.46 MiB for vs Bots, 84.25 MiB for Ranked and
549.64 MiB for mixed. These are process samples on a shared diagnostic host,
not a production memory-limit or per-arena capacity certification.

Aggregate world-broadcast mean gaps were 51.72 ms (vs Bots), 51.27 ms (Ranked)
and 52.62 ms (mixed). The p99 exclusive histogram upper bounds were respectively
75 ms, 75 ms and 75 ms. Latest-transform age at snapshot had p99 exclusive upper
bounds of 75 ms, 100 ms and 250 ms. These are cumulative instrumented loopback
histograms, not per-command ACK latency, coordinate-change proof or WAN SLA.

## Binary and source binding

- Actual server SHA256: `ecb7648e761f62d31d2c523d2076dd0cff64bbbedaf21853856d1d38e4a6f5ca`.
- Native generator SHA256: `8c528c587c23910b00fa82bd181e2b030cbaa15fe2207831c10ac818de1a7695`.
- Ranked fixture SHA256: `67d257c2e9a96eb7f27f01e1805f4bb4f14e1848f11250bad647ba6dfa2a0373`.
- Server cgroup-wrapper SHA256: `dace56eab1356ac5e34b7fb3a14cfae6f50fef35646916ca308072ff0cdfb604`.
- All 19 source files named by each report are under `sources/`.
- `native-source.tar.gz` additionally preserves the 24 migration/grant inputs.
  `fixture.json`, binary hashes and the remote controller bind the fixture to the
  exact candidate and Release run.

`verify.py` is standalone stdlib-only code. It imports no current-checkout
module, recomputes archived payload hashes and every worker's final
acceptance-time signed endpoint counters from its metric log, then checks
workloads, arena/bot counts, rosters, joined sessions, health, qualification,
input cadence and cleanup.

The selected on-disk status files were copied after owned teardown and can say
`settling` or `failed`. Acceptance used the report's pre-teardown Running
receipt. Worker logs can contain post-acceptance lines, so verification uses the
first `metric_records` observations and does not equate the later log hash with
the acceptance-time hash.

Run:

```sh
python3 docs/evidence/linux-capacity-workloads-2026-11-10/verify.py
```

Expected: three PASS rows, `source_files: 19` and Release run `42657406745`.
The same verification also passed with `python3 -S`.

## Cleanup and remaining gates

Only selected reports, manifests/status/health, worker metric logs, owned-PID
lists, wrapper summaries, cleanup records and source snapshots are archived.
Raw generator logs, private keys, DSNs and binaries are excluded. The final host
check found no owned capacity process, no `omoba_test` database and no disposable
capacity role; both ordinary dev services remained active. The newly uploaded
remote fixture and its archive were then removed. No production host, public
port, firewall, ordinary dev database or dev service configuration was changed.

The next gate is an isolated actual or identical RU host: stepped and sustained
Casual/vs Bots/Ranked/mixed measurements, measured uplink including IP/UDP/NIC
overhead, WAN/multi-host/fault scenarios and an explicit capacity decision.
Devices, independent recovery, C2/C4 decisions and human review remain separate
release gates. This packet does not authorize merge, release or RU rollout.
