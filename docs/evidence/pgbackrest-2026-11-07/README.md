# C4 pgBackRest/SFTP mechanism proof

Accepted run v4: seven checks PASS with PostgreSQL18.6 and pgBackRest2.58.0.
The actual game career10/portal11 migrations run on a new database. Two separate
containers on one internal Docker network provide the database and SFTP storage.
This proves the real transport/tool mechanism, not independent physical failure
domains, a Russian storage location, production RPO, or installation on RU.

All player receipts are explicitly synthetic direct-SQL fixtures. A physical
backup contains rating1000/history0. Subsequent WAL restores exactly1024/history1
at the named point and1048/history2 at the latest archived position. Restored
clusters must finish promotion and retain the original system identifier;
read-only hot-standby readiness is never accepted as successful recovery.

The repository reports AES-256-CBC. Missing and deliberately corrupted post-base
encrypted WAL both terminate target recovery rather than yielding stale success.
A wrong SFTP host pin returns a structured nonzero repository status; an
unreachable repository fails the live archive check. The script restores faulted
files and removes every owned database/container/network, verifying absence.
No key, passphrase, database directory or backup object is retained in this proof.
The small development image is retained for subsequent C4 integration checks.

The1MiB WAL segment is a fixture bound, not a production setting. pgBackRest's
native encryption/checksums are not GnuPG signatures. Existing signed logical
backups are unchanged. The first incomplete attempts are rejected in summary.json;
the earlier six-check checkpoint is superseded by stronger structured pin-status,
actual cipher, corruption and cleanup assertions.

## Reproduce only on a disposable local Docker engine

From the game repository root, build this development image and run into a fresh
output folder. The script guards8GiB free and never uses an existing database or
production connection. Packages are installed only inside the development image.

```sh
docker build -t omoba-mm-pitr-rehearsal:20261107 -f docs/evidence/pgbackrest-2026-11-07/Dockerfile docs/evidence/pgbackrest-2026-11-07
python3 docs/evidence/pgbackrest-2026-11-07/rehearse.py --output .agent/pitr-proof-fresh
```

## Still required for C4

Prepare and verify the actual host-kit patch: explicit remote configuration and
secret escrow, archive settings, physical backup/retention timers, backup health
integration and alerts, a bounded isolated restore drill and reviewable rollout.
The new production package and host changes require owner approval under AGENTS;
no such package/configuration has been added to the host kit or installed on RU.
An actual independent-storage drill and recovery/RPO acceptance remain required.
C4 stays open.

The implementation follows the PostgreSQL requirement for a physical base plus
continuous WAL; a logical dump cannot substitute for that base. See
[PostgreSQL continuous archiving](https://www.postgresql.org/docs/18/continuous-archiving.html),
[pgBackRest SFTP and recovery guide](https://pgbackrest.org/user-guide.html),
and [pgBackRest configuration reference](https://pgbackrest.org/configuration.html).
