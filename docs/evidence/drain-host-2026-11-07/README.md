# C2 host orchestration fault proof

See [the concrete patch review](../../plans/reviews/C2-DRAIN-2026-11-07.md) and
[hashes](summary.json). C4 and C2 remain unapplied review data. C2 composes on the
exact C4 draft instead of overwriting its host changes.

Actual Host/controller/CLI methods run a full bootstrap, initial ship, drain/update
and rollback against temporary directories and simulated apt/systemd/PostgreSQL/
backup commands.28 new cases cover freshness/ownership, malformed/special files,
PID/slot disagreement, timeout, backup/migration/new-start/health failure, rollback,
unexpected restarted workers, history write and marker-removal failures. A real
parent-directory fsync failure is injected after marker removal; its result must
be resume unconfirmed. CLI dry-run/read-only checks and held/uncertain nonzero
exits are exercised. Existing CI test discovery already includes the new filename;
no workflow is modified.

The [final combined log](combined-tests.log) records184 tests:183 passed,
1 expected optional legacy PostgreSQL E2E skip,0 failed. The [28-case verbose log](drain-tests.log)
precedes only removal of an extra EOF blank line; the184-case run uses the exact
saved source. The [rejected older draft](rejected-old-draft.log) proves the two
initially discovered failures: stale timestamp equality and missing operation
history. Setup errors/intermediate runs are not accepted evidence.

To reproduce, create a disposable local checkout at source_parent from summary,
apply the exact C4 patch, then C2 patch there. From that isolated checkout:

```sh
env PYTHONPATH=scripts NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1 python3 -m unittest test_ru_host test_ru_host_drain test_ru_backup test_ru_pitr test_ru_monitor
python3 -m unittest discover -s scripts -p 'test_ru_host_drain.py' -v
```

Use Python3.13, as in the accepted run. Never use a production checkout or service
for this rehearsal. No SSH or real host service is invoked by these tests. This
is orchestration fault coverage, not systemd/RU storage/migration/capacity, native
UI or WAN acceptance. Separate real signed drain runtime and C4 PG16/18 recovery
proofs retain their own scopes. Approval, integration, full CI and RU acceptance
remain required.
