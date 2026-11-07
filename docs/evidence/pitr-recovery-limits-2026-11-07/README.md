# Recovery parameter regression — review candidate

The original C4 proposal restores an isolated cluster with max_connections=20.
A source configured with80 reproduces PostgreSQL's fatal insufficient-parameter
recovery refusal. The native restoration fixture reaches this failure before its
later combined-helper drill; the identical constant in that helper was then
confirmed by source inspection. This is not a claim that the rejected fixture
successfully executed the old helper's complete drill.

The revised [15-file review patch](../../plans/reviews/C4-PITR-PARAMETERS-2026-11-07.patch)
preserves all five required shared-memory dimensions using validated numeric
primary/base-control values. See [candidate identity](candidate.json),
[review scope](../../plans/reviews/C4-PITR-PARAMETERS-2026-11-07.md) and
[PostgreSQL requirements](https://www.postgresql.org/docs/18/hot-standby.html#HOT-STANDBY-ADMIN).
It remains unapplied to the tracked host kit and RU.

Tests cover higher primary/base values, mixed maxima, all five dimensions and
refusal of absent, malformed, duplicate or injected data. The composed candidate
is tested against current sources, including healthv2 monitoring. Live raised-
limit physical restoration and actual current worker acceptance have their own
[runner/report](../worker-physical-recovery-2026-11-07/README.md). Read the actual
status there; a passing unit suite alone is not physical recovery acceptance.

The earlier schema13/portal11 PG16/18 runs remain valid for their original20-
connection fixtures. They do not certify raised limits. Historical patch/code/
reports are retained; the revised patch does not silently replace them.


Current-source [unit log](unit-tests.log):176 pass,1 expected optional skip,0 failed.
Actual PG18 combined helper/physical/current-worker run passes22 stages with all
five elevated dimensions; its drill report records80/12/14/4/128.
[Before fatal recovery log](before.log) preserves the rejected20-versus80 case.
PG16.15 also passes22 stages with the same raised limits, actual worker restore/Resume
and221.63s crash recovery. Terminal-spool and independent-storage/RU acceptance
remain open.
