# C4 combined backup host-kit review proof

The saved [15-file patch](../../plans/reviews/C4-PITR-2026-11-07.patch) is a tested
review draft, not applied to the tracked host kit or to any server. See the
[review scope](../../plans/reviews/C4-PITR-2026-11-07.md) and [source hashes](summary.json).

Accepted actual combined runs: PostgreSQL16.15 and18.6 with pgBackRest2.58.0,
18 checks each PASS. The production ru_backup coordinator from the draft actually
uploads a GnuPG-signed/encrypted logical archive and a separately AES-encrypted
physical base/WAL through strict-host-key SFTP. The actual career10/portal11
migrations and explicit direct-SQL synthetic receipts recover rating1024/history1
at a named point and1048/history2 at the latest archive from1000/history0.
The complete logical-plus-physical monthly drill proves promotion, original
system identity, schema histories, replay position and scratch cleanup.

Missing/corrupt WAL, wrong host pin and unavailable remote storage are rejected.
A failed physical phase still preserves the independent signed logical upload,
without claiming combined success. Actual four-full retention leaves three bases.
The minute-check CLI failure/recovery keeps its own alert journal and preserves
logical backup metadata byte for byte. Final health also requires actual escrow.
All owned test containers/network are removed and their absence checked. Keys
and database directories are generated only in disposable containers, not saved.

The [unit log](unit-tests.log) records156 tests:155 passed,1 expected optional
legacy E2E skip,0 failures. The actual PG16/18 integration is separate. Earlier
failed proof attempts are rejected: a root/postgres key-role mismatch, an existing
SFTP listing-parser defect (OpenSSH may print '?' for link count), and an invalid
container PID1 setup were corrected. Accepted runs use docker --init so GnuPG
children are reaped without confusing PostgreSQL process supervision.

The exact executed script and raw reports retain an overly specific PG18 scope
label; the PG16 report's actual postgres field and image ID prove16.15. Reports
were preserved unchanged. The1MiB WAL segment is a small fixture setting, not a
production recommendation. These are local containers on one engine, not
independent failure domains, real RU storage, production RPO, games, or UI proof.

## Reproduce in an isolated local checkout

Create a disposable checkout at the source_parent in summary.json, apply the
review patch there, and copy only its patched ops/vps/ru folder to
.agent/codex-ranked-readiness/pitr-draft/tree/ops/vps/ru (the exact script's input).
Do not apply this review patch to an active production checkout for a rehearsal.

From that checkout, using a copy of this evidence directory:

```sh
docker build -t omoba-mm-pitr-rehearsal:20261107 -f docs/evidence/pitr-kit-2026-11-07/Dockerfile.rehearsal docs/evidence/pitr-kit-2026-11-07
docker build -t omoba-mm-pitr-kit-rehearsal:20261107 -f docs/evidence/pitr-kit-2026-11-07/Dockerfile.kit docs/evidence/pitr-kit-2026-11-07
docker build -t omoba-mm-pitr-kit16-rehearsal:20261107 -f docs/evidence/pitr-kit-2026-11-07/Dockerfile.kit16 docs/evidence/pitr-kit-2026-11-07
python3 docs/evidence/pitr-kit-2026-11-07/rehearse_full_kit.py --image omoba-mm-pitr-kit-rehearsal:20261107 --output .agent/pitr-kit-fresh18
python3 docs/evidence/pitr-kit-2026-11-07/rehearse_full_kit.py --image omoba-mm-pitr-kit16-rehearsal:20261107 --output .agent/pitr-kit-fresh16
env PYTHONPATH=scripts NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1 python3 -m unittest test_ru_backup test_ru_host test_ru_monitor test_ru_pitr
```

Package versions are recorded from the actual accepted images. Future image
builds may obtain newer distribution packages; check their versions explicitly.
C4 remains open until owner approval, integration and actual independent-storage/
RU recovery acceptance. No CI workflow, secret, PostgreSQL setting or service on
RU was changed to prepare this review.
