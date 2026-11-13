# Real terminal spool and duplicate delivery through physical SQL restoration

Accepted PG16.15 run:22 stages PASS. [Summary/hashes](summary.json),
[complete physical report](pg16/summary.json), [signed protocol report](pg16/runtime/report.json).
SIGTERM to fresh assignment takes248.22s, including two ordinary recovery
deadlines; duplicate-worker start to fresh assignment takes125.46s.

The [runner](rehearse_terminal_replay.py) uses the exact accepted current native
server, actual public game-role grants/SCRAM and the unchanged review-only C4
helpers in an isolated current-source copy. It creates only its own local
containers/network, loopback proxy, lobby and workers. No host integration or RU
operation is performed. The preceding synthetic backup fixtures are separate
from the real signed public bot-practice match.

## What this checks

1. Start a real public match with one signed protocol human and nine ordinary
   server bots. Require Running and its durable gameplay checkpoint, then take
   an encrypted physical base and archive WAL with schema13/portal11.
2. Disconnect only the owned SQL proxy. SIGTERM the actual worker and require
   its durable-shutdown marker, process exit and real Settle JSON. Confirm its
   hash and that the SQL source still has a running match without terminal intent.
   The file's end time must postdate the SQL checkpoint.
3. Physically restore the known SQL admission and source system identity into a
   third owned container. Wait for writable promotion before reconnecting the
   proxy. The normal lobby automatically launches its recovery worker; the real
   terminal file must be acknowledged/deleted and one Interrupted history/profile
   credit saved without Elo/XP. Do not synthesize a result or restore admission
   by manually changing SQL.
4. After that worker exits, write the **same original bytes** back with file and
   directory fsync. Start a second actual public recovery worker using the same
   manifest, role and outbox. Require another acknowledgement, an authoritative
   correlated Profile response with still one credit and unchanged rewards, and
   retirement under its ordinary120s recovery deadline. Only then accept a new
   allocation. A second read alone would not prove duplicate delivery.
5. Read the final result directly from the physically restored SQL cluster.
   Compare identity, timestamps, duration, outcome/reason and all ten participants'
   stats against the actual file. The additional [whole-payload verifier](verify_payload.py)
   checks every serialized field: only saved=false becomes saved=true.

The original real receipt, final SQL result, native protocol report, full physical
report and source/binary/probe/runner hashes are preserved. Private keys, runtime
database directories and DSNs are not published. All owned containers/network
must be removed before the overall run can pass.

## Reproduce

Reuse the review helper preparation and local images from
[the dual-version worker proof](../worker-physical-recovery-2026-11-07/README.md).
The runner verifies helper hashes against the current C4 candidate and the exact
accepted production binary against the current source manifest. No graphics/
Cargo/dependency rebuild is required. With free loopback46530/46630 and a fresh
output directory:

```sh
python3 docs/evidence/terminal-spool-replay-2026-11-07/rehearse_terminal_replay.py --native-server .agent/codex-ranked-readiness/ready-runtime/server --output .agent/terminal-spool-fresh
python3 docs/evidence/terminal-spool-replay-2026-11-07/verify_payload.py
```

The second command verifies the committed raw evidence, not a new live run.
The public probe exposes this scenario only with its loopback one-human recovery
fixture and a correlated control directory inside that run's output. No supplied
command or arbitrary SQL is executed by the probe.

## Scope

This is PG16.15, the current schema13/portal11 and public bot practice on one local
engine, including actual terminal-file and duplicate replay. Earlier PG16/18
SQL/checkpoint/Resume proofs keep their separate scopes; their empty outboxes did
not prove this case. This does not establish independent storage/RU RPO/RTO,
Ranked/party WAN faults, physical devices, public trust provisioning or sustained
host capacity. The C4/C2 proposals remain unapplied and require their existing
owner integration/rollout decisions. Current complete CI is still required.
