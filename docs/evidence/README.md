# Evidence archive and tracked-file policy

Large evidence payloads are **not stored in the main Git repository**. The complete
Ranked-matchmaking snapshot for game `dad1e4f6` is a private draft release asset in
`o-moba/omoba-bevy-archive`, not a game release. Its exact identity is recorded in
[ARCHIVE.json](ARCHIVE.json).

## Restore and verify

A fresh clone needs GitHub CLI authentication with access to the private archive
repository. From the game repository root:

```bash
python3 docs/evidence/restore.py
```

Or verify/use a previously downloaded archive:

```bash
python3 docs/evidence/restore.py --archive /path/to/omoba-ranked-evidence-dad1e4f6.tar.gz --check-only
python3 docs/evidence/restore.py --archive /path/to/omoba-ranked-evidence-dad1e4f6.tar.gz
```

The archive restores payloads below their original `docs/evidence/<packet>/` paths.
Tracked READMEs, summaries and verifiers are always left untouched. Existing
identical payloads are accepted; differing payloads are not overwritten unless
`--overwrite` is explicitly supplied. Then run a packet's retained verifier, for
example:

```bash
python3 docs/evidence/server-load-metrics-2026-11-09/verify.py
python3 -S docs/evidence/server-load-metrics-2026-11-09/verify.py
```

Payload links in packet READMEs intentionally resolve after restoration. Do not
`git add -f` restored payloads back into the main repository.

## What may be tracked

- `README.md` and other concise Markdown conclusions, limitations and runbooks;
- `verify*.py` recomputing verifiers, at most 128 KiB each;
- `manifest.json`, at most 64 KiB;
- small summary JSON files, at most 16 KiB each;
- this archive manifest and restore helper.

Raw logs, load-report payloads, capture geometry, archived source trees,
`Cargo.lock`, compressed payloads, images, binaries and rehearsal copies belong in
an immutable external archive. New archives need a new release asset and manifest;
do not silently replace an accepted archive. `scripts/evidence_policy.py` checks
these rules against the Git index and runs in the existing Python tooling CI job.
