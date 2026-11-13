# Native Casual/Ranked/vs Bots/mixed runner smoke — 2026-11-10

This packet proves the generalized capacity runner's small live loopback
contract. It is **not** a host-capacity, long-soak, RU, WAN, uplink, physical
device, organic Ranked-eligibility or rated-settlement measurement. #13 remains
open and the RU per-arena table stays unfilled.

## Live results

Exact current Rust sources matched the accepted local server/bots source set;
only the Python runner and its tests were dirty. All runs used independent
20 Hz signed native clients, loopback PostgreSQL `omoba_test`, opt-in server
metrics and a 15-second Running-population soak.

| Run | Signed humans | Generator shards | Real arenas | Server bots | Actual sends/s, min–max | Gameplay transforms | Sequence gaps / restarts | Result |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| vs Bots | 10 | 1 | 10 | 90 | 20.0066–20.0066 | 3746 | 0 / 0 | PASS |
| Ranked | 10 | 1 | 1 | 0 | 20.0978–20.0978 | 3440 | 0 / 0 | PASS |
| Mixed | 30 | 3 | 12 | 90 | 19.9965–20.0051 | 11561 | 0 / 0 | PASS |

Mixed is exactly one Casual shard, one Ranked shard and one vs Bots shard. Its
12 arenas are one Casual + one Ranked + ten vs Bots workers; ten vs Bots humans
are never miscounted as one arena. Every expected durable roster and signed
metric worker was present. All selected health receipts are v2 with SQL and
journal ready. No owned process remained alive in the post-run check.

Ranked and mixed used the explicit helper to seed ten synthetic prior Casual
receipts per Ranked profile in the disposable test database. This is disclosed
in each report as `ranked_qualification.status=seeded`; it does not prove the
organic ten-game journey or rated settlement.

## Binding and verification

- Reported source parent: `36acb77129011a3799867f9755ca9684f153bc7e`.
- Server SHA256: `f8c6daad7fbd556f1b64769ce99240e127033c9c9b254345d6576ad2b747e2d8`.
- Native generator SHA256: `5ee8d341de8c0d4e2b3f063d561f14408c33cf5623f66d73a2cb68a8dafc3234`.
- All 19 source files named by the reports, including the new runner,
  qualification-helper source and match-allocation/service contracts, are under
  `sources/`; `verify.py` resolves their hashes only there and imports no
  current-checkout code.
- The verifier recomputes every worker's final acceptance-time signed endpoint
  counters from its archived metric log, then the aggregate totals. It checks
  exact workloads, arena/bot counts, manifests, joined sessions, health,
  qualification disclosure, input gate and cleanup record.
- The selected on-disk status files were copied after owned teardown and can
  say `failed`; acceptance used the report's **pre-teardown** Running receipt.
  Worker logs can similarly contain post-acceptance lines, so verification uses
  the first `metric_records` observations. It does not pretend the later file
  hash equals the acceptance-time log hash.

Run:

```sh
python3 docs/evidence/capacity-workloads-smoke-2026-11-10/verify.py
```

Expected: three PASS rows and `source_files: 19`.

Static suites also pass: 19 capacity tests, 72 Release tests and 14 public
rehearsal tests. The Release workflow now builds and uploads
`examples/qualify-ranked-fixture` beside `bots` in `linux-capacity-bots` so a
reviewed Linux candidate has the exact Ranked fixture dependency.

Only selected reports, manifests/status/health, worker metric logs, owned-PID
lists, cleanup records and source snapshots are archived. Raw generator logs,
private keys, DSNs and binaries are excluded. The next gate remains the exact
Release candidate on an isolated actual/identical RU host, stepped and sustained
Casual/vs Bots/Ranked/mixed measurements, followed by real uplink/WAN overhead
and capacity decisions. No production configuration or service was changed.
