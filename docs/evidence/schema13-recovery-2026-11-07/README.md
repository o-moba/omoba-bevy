# Current-schema13 isolated recovery rehearsal

Current career13/portal11 migrations and actual runtime grants are exercised by
[the runner](rehearse_schema13.py), based on the earlier
[full-kit runner](../pitr-kit-2026-11-07/rehearse_full_kit.py). The reviewed C4
backup/PITR helpers are used only in disposable containers; their exact hashes
must match the saved review candidate before execution. The candidate remains
unapplied to the tracked host kit and to RU. This packet introduces no production
dependency, credentials, database setting or deployment change.

See [summary.json](summary.json) for exact accepted runs, current migration/grant
hashes, image identities, runner hash, cleanup and remaining acceptance gates.

## What the rehearsal verifies

The actual combined backup coordinator uploads an independently signed/encrypted
logical backup and an AES-encrypted physical base/WAL to strict-host-key SFTP.
After the base backup, direct SQL synthetic fixtures change rating/history,
approve consent and then consume it, release its claim and add a queue-dodge
receipt/cooldown. These are deliberately synthetic database records, not played
games or authorization obtained through matchmaking.

Named-point recovery must return the approved consent, exact membership and live
claim without the later dodge. Latest-archive recovery must return consumed
consent, the released claim and exact subsequent discipline records. Both compare
the complete13/11 migration histories and exact fixture JSON with the source at
that point, alongside rating/history1024/1 and1048/2 respectively. The restored
system identifier must match the source and the cluster must finish recovery.

Source and both restored clusters exercise the actual game and portal grants:
game can invoke bounded retention and append queue events, while neither role
can create schema objects or close seasons. Game cannot directly delete consent
or queue events; portal cannot read consent or execute retention. Four forbidden
SQL operations must fail with permission denied. Retention executes as the game
role in a rolled-back transaction, deleting only the eligible old closed receipt;
its definer search path must remain public,pg_temp. Test roles are local NOLOGIN
roles, not production identities or an authentication test.

The complete coordinator drill additionally checks signed logical restoration,
physical recovery, source identity/schema/replay and scratch cleanup. The
inherited fault stages reject missing/corrupt post-base WAL, wrong host pin and
unavailable storage; preserve an independent logical upload when PITR fails;
exercise full-backup retention and archive-check alert/resolution metadata; and
require encrypted secret escrow before combined health can pass. Alert checks
use only the fixture's journal, with no external recipient configured.

## Reproduce

Use the existing isolated C4 candidate preparation described in
[the historical evidence](../pitr-kit-2026-11-07/README.md). Its patched
ru_backup.py and ru_pitr.py must be available at
`.agent/codex-ranked-readiness/pitr-draft/tree/ops/vps/ru/`; this runner checks
their exact saved hashes. Do not apply the candidate to a production checkout.
Reuse the matching existing images where available; the historical Dockerfiles
describe their construction and package versions must be checked explicitly.

From the current game checkout, choose fresh output directories:

```sh
python3 docs/evidence/schema13-recovery-2026-11-07/rehearse_schema13.py --image omoba-mm-pitr-kit-rehearsal:20261107 --output .agent/schema13-recovery-fresh18
python3 docs/evidence/schema13-recovery-2026-11-07/rehearse_schema13.py --image omoba-mm-pitr-kit16-rehearsal:20261107 --output .agent/schema13-recovery-fresh16
```

Runs are sequential. The runner creates only uniquely named local containers and
an internal network, generates keys inside them, then removes and checks absence
of its own resources. It never addresses an existing game database or RU host.
The1MiB WAL setting is a small-fixture setting, not a production recommendation.
Only sanitized reports and recovery logs are retained; no private keys/database
directories are published.

## Limits

These two containers share one local engine. The result establishes the current
schema/grant recovery mechanism; it does not establish independent failure
domains, production RPO/RTO, real off-host/RU storage, runtime authentication,
actual worker receipt replay/Resume after restoration, or public WAN capacity.
Those release gates remain open, along with current complete CI, UI/physical
acceptance and the existing C2/C4 integration/rollout decisions. Older schema10
evidence keeps its historical scope and is not relabeled as current.
