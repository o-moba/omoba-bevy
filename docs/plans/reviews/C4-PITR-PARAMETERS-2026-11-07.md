# C4 revision: preserve PostgreSQL recovery limits

Review-only replacement for the original
[C4 proposal](C4-PITR-2026-11-07.md), retaining its integration/rollout boundary.
[Exact revised patch](C4-PITR-PARAMETERS-2026-11-07.patch) and
[candidate hashes](../../evidence/pitr-recovery-limits-2026-11-07/candidate.json).
The original patch and historical evidence are preserved.

A raised-limit physical fixture fails recovery: its source uses80 connections,
while the isolated restored cluster hardcodes20. Inspection finds the same
hardcoded value in the proposed ru_pitr drill. Earlier passing fixtures used20
on both sides and did not cover this failure. PostgreSQL requires sufficient
shared-memory dimensions while replaying WAL; all five relevant settings must
be considered, as described in the
[official recovery requirements](https://www.postgresql.org/docs/18/hot-standby.html#HOT-STANDBY-ADMIN).

The revised helper reads the current primary's five numeric settings and the
restored base's pg_controldata requirements, and uses the greater value for each.
It validates the exact allowlist, integer types/ranges and unambiguous control
fields before writing its isolated configuration. Missing, duplicate, malformed
or injected values fail closed. Source connection/preload/archive configuration
is still discarded; network listening stays disabled for the drill. Limits are
recorded in the drill report. No source PostgreSQL setting is changed.

The patch applies cleanly to current f8510b57 in an isolated copy. That composed
candidate passes177 cases:176 passed,1 optional legacy integration skip. The
actual raised-limit physical/worker rehearsal is recorded in the
[separate evidence pack](../../evidence/worker-physical-recovery-2026-11-07/README.md);
its acceptance status must be read from the report, not inferred from unit tests.

The new proposal changes only the two newly introduced candidate files relative
to the original15-file package. It is not applied to tracked ops/scripts or RU.
The existing owner decision to integrate pgBackRest and the host kit remains
required under the user-provided AGENTS rules for production dependencies and
infrastructure changes. It does not authorize installation, provisioning,
archive settings/restart, real database recovery or deployment. C2 still needs
composition against the final accepted host helper.
