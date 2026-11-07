# Current worker against physically restored schema13

Accepted PG18.6 andPG16.15 runs:22 stages each PASS. [Summary and hashes](summary.json),
[PG18 physical report](pg18/summary.json), [PG18 signed journey](pg18/runtime/report.json),
[PG16 physical report](pg16/summary.json), [PG16 signed journey](pg16/runtime/report.json),
[rejected setups](rejected-attempts.json). Real crash recovery takes223.47s onPG18
and221.63s onPG16. Both preserve the same signed human/worker/epoch through restore.

The PG16 runner now waits for archive promotion after connection readiness.
A server may accept read-only queries while still recovering; game traffic
remains isolated until pg_is_in_recovery() is false. The earlier immediate
assertion failed and is rejected, without weakening the writable-primary gate.
See [PostgreSQL startup semantics](https://www.postgresql.org/docs/16/app-pg-ctl.html).
PG18 runner provenance remains pinned to9ed77bd8 in summary.json; the current
runner adds this bounded readiness observation.

[Runner](rehearse_worker_restore.py) extends the earlier isolated schema13
rehearsal with a real signed public bot-practice game, using the exact previously
verified native server binary and current production source hashes. It uses the
[revised review-only C4 helpers](../pitr-recovery-limits-2026-11-07/README.md),
applied only to an isolated copy of current source. Nothing is installed or
configured on RU.

The source has elevated values for all five recovery dimensions: connections80,
workers12, WAL senders14, prepared transactions4 and locks per transaction128.
The initial synthetic checks still verify named/latest recovery, exact current
consent/discipline/grants, corrupted/missing WAL, pin/outage refusal, backup
retention, logical fallback, escrow and cleanup. The actual combined helper must
complete its own drill with those limits; the report records the chosen values.

The separate live stage starts the actual public lobby and worker, with one
signed human protocol client and nine ordinary server bots. The game uses a
generated local SCRAM login with the actual game-role grants. Its profile,
consent, admission and result are created by the running game, not direct SQL.
The SQL fixtures used by the preceding backup checks remain separate.

After the real game reaches Running and its gameplay-start checkpoint is
durable, the runner takes a physical backup and archives WAL. Recovery secrets
are streamed directly between its own containers, never written to a host
artifact. It stops only the owned source container and restores into a third
owned container. A loopback-only fixture proxy keeps the application's database
address stable while dropping its old connections and switching to the restored
cluster. Source system identity, schema13 and the actual running admission must
survive before the correlated restore receipt is released.

The protocol probe then reconnects the same authenticated human to the same
worker, profile and epoch, requires Running with advancing snapshots and fresh
authenticated storage health, restarts only the lobby and proves adoption.
It subsequently SIGKILLs only the owned worker, waits the ordinary stale-owner/
recovery deadlines and requires one original Interrupted history/profile update,
without Elo/XP, followed by a new assignment. A final SQL read from the restored
cluster must agree with the actual saved result. These assertions are unchanged
from the existing crash-recovery probe except for the preceding physical restore.

## Reproduce and interpret

Prepare the revised candidate in an isolated current-source copy at
`.agent/codex-ranked-readiness/pitr-parameters/current-tree/`. The runner checks
its helper hashes against candidate.json before starting. Reuse the existing
version-verified kit image and accepted native server; no graphics/Cargo rebuild
is needed. Choose a fresh output directory and free loopback game ports:

```sh
python3 docs/evidence/worker-physical-recovery-2026-11-07/rehearse_worker_restore.py --native-server .agent/codex-ranked-readiness/ready-runtime/server --output .agent/worker-restore-fresh18
python3 docs/evidence/worker-physical-recovery-2026-11-07/rehearse_worker_restore.py --image omoba-mm-pitr-kit16-rehearsal:20261107 --native-server .agent/codex-ranked-readiness/ready-runtime/server --output .agent/worker-restore-fresh16
```

The report's status is authoritative. Failed/rejected attempts are retained
separately and are not acceptance. The first preparation also checked the wrong
gameplay field: immutable draft allocation versus the durable gameplay-start
checkpoint. That check was corrected before any accepted live stage; the first
run failed earlier on insufficient recovery dimensions. A subsequent setup used
an internal Docker network that could not publish the loopback DB port; another
required an explicit true field that canonical MatchResult serialization omits.
The accepted run uses a uniquely named bridge with only loopback-published ports
and the exact canonical checkpoint semantics.

This is one local engine, one human protocol client and bot practice. It does not
certify independent failure domains, RU RPO/RTO, Ranked ten-human/party WAN
recovery, physical devices, public trust provisioning or sustained capacity.
The crash path proves actual restored SQL/checkpoint recovery; terminal receipt
spool fault/replay requires its own explicit evidence. C2/C4 integration and real
rollout retain their existing owner-decision boundary. Do not close those gates
from this local runner or from historical green CI.


The separate [actual terminal-file/duplicate proof](../terminal-spool-replay-2026-11-07/README.md)
now covers that local gap onPG16. This packet's own empty-outbox reports retain
their narrower SQL/checkpoint scope; their historical remaining fields are not
rewritten as if they had exercised terminal replay.
