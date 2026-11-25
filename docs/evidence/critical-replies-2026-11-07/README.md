# Bounded critical replies — 2026-11-07

Local acceptance for the source hashes in [summary.json](summary.json), based on
`bc08eb5262bb395cb708563919b94d7154662c81`. This packet removes blocking Ready and
Flush completion sends from the actual career worker. It does not change career
schema13, portal schema11 or game protocol16. Anonymous operational health becomes
v2; deploy its producer and monitor together through the reviewed drained rollout.

## Failure and correction

[Before](before.log), a full live reply receiver stalls the worker: only2.986547s
remain on a15s match lease after12s. [The same test after the fix](after.log)
passes the unchanged>60s threshold and delivers the exact Ready and Flush replies.

The worker retains at most256 critical completions in FIFO completion order,
preserving work/sequence, errors, flush generation and success. Before taking any
job it reserves space for a complete4-item Ready batch. At capacity it yields for
10ms without taking more jobs; SQL, heartbeat, retries and health remain active.
Existing input256/output512 channel caps are unchanged. Known disconnected
recipients release abandoned callbacks; SQL/outboxes remain authoritative. These
in-memory callbacks do not provide persistence across process crashes.

[Bounded-pressure proof](bounded-pressure.log) first ACKs Start and removes its
spool, then establishes Ready ownership before filling the live output channel.
It retains256 completions, accepts255 further queued jobs behind the flush
barrier and holds consumption for12s. The independent heartbeat leaves82.964s
on the lease; health advances. Reading resumes and receives all256 original work
sequences and the exact successful barrier. Unpublished consent expires normally:
public admission refuses it, with no played game or dodge penalty.

## Accepted checks

- [Full server run](server-all.log):575 passed,0 failed/ignored, including16 cases
  using real PostgreSQL, serial execution. [Strict native checks](clippy.log),
  [production link](production-link.log) and [test link](test-link.log) passed
  using the existing pinned dependency graph and shared cache, without a new
  workspace graphics build.
- [Monitor](monitor.log):42 passed; [host tools](host-tools.log):74 passed.
  No external notifications were sent. The monitor validates healthv2, the256
  reply limit, known receiver closure, freshness and sustained blocking.
- [Signed ready journey](ready-check.json):15s unanimous confirmation, timeout,
  preserved FIFO age, late refusal and all10 replacement humans actually Running;
  fresh authenticated v2 reports from lobby and worker have matching epochs and
  no pending critical replies or known receiver closure.
- [Actual worker termination](termination.json):SIGTERM, durable outbox and
  Interrupted outcome. This one-client bot-practice shutdown proof does not
  replace the separate ten-human ready journey.
- The prior [101-case store proof](../storage-health-2026-11-07/store-proof.json)
  was not rerun: its Rust/SQL source hashes still match. Compiler arguments are
  referenced and hashed in summary.json, along with current source/binary hashes.

[Rejected attempts](rejected-attempts.json) distinguish the expected red test,
the unseeded fixture's legitimate ownership-retry refusal and an unsupported
termination wrapper preset. Neither rejected setup is acceptance. The two exact
owned rejected schemas were inspected and removed; see
[cleanup](rejected-cleanup.json). No unrelated schema was touched.

## CI and remaining release gates

[Historical parent CI](parent-ci.json), head1e7607b7, passed all7 jobs. Artifact
13561237197 passed size/SHA256, safe ZIP paths and CRC inspection; all20 public
stages passed and the private fixture was removed. This is schema13 retention
evidence, not current backpressure/v2 CI acceptance. Fresh complete CI is required
for this packet.

Still open: current-schema13 physical/WAL/off-host recovery and restored rights/
replay, sustained current-source load/cadence and RU capacity, current UI and
physical Android checks, public trust packaging/provisioning, actual multi-host/
WAN party/Resume/fault acceptance, existing C2/C4 decisions and approved rollout.
The original68 ledger stays62 done,5 partial,1 open. No RU migration, grant,
settings/credential change, alert delivery or deployment occurred.
