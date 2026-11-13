# Architecture documentation and launch readiness

Goal: give the project one current, citable description of its architecture,
methodology, pipelines and protocols, and a verified list of what is missing
before OMOBA can run as a self-sustaining public product (RU market first).

## Changes

- `docs/architecture/`: README (index, glossary, known drift), system,
  identity-and-accounts, protocols, data, build-and-release, methodology and the
  ADR log `decisions.md`. Each page was drafted from the code, fact-checked claim
  by claim against the code by a separate reviewer, revised, then the whole set
  was reviewed for contradictions, duplicated facts and broken links and fixed.
- `docs/plans/launch-readiness.md`: launch-readiness audit of eight areas, every
  gap re-checked against the code by a skeptic, plus a completeness pass over the
  player and operator journeys; merged into 66 specs (M0 launch blockers, M1
  first public weeks, M2 growth) with acceptance criteria, sizes, owners, the
  critical path and open owner decisions.
- Fixes found on the way: `ops/vps/dev/account_api_dev.py` required portal
  schema 1–6 and would refuse every release after migration 7 (now 1–7, test
  updated); the RU runbook and `ops/vps/ru/README.md` now warn that the RU kit is
  not deployable before LR-02 and the other M0 ops specs; stale statements in
  `docs/supporter.md`, `account-api/README.md`, `CONTRIBUTING.md`,
  `ops/vps/README.md` and the `docs/ARCHITECTURE.md` crate map (missing `common`)
  corrected; the 2026-09-25 snapshots are marked historical.

## Checks

- Spot checks of the audit against the code (LR-02 ingress key, LR-08 queue
  preferences, LR-09 arena capacity, LR-10 route planning): confirmed.
- `python3 -m unittest discover -s scripts -p 'test_account_api_dev.py'` — pass.
- Documentation only otherwise; no build was run.

## Remaining

- The specs in `docs/plans/launch-readiness.md` are not implemented; LR-02 (the
  RU portal ↔ Account API ingress) is the first code fix.
- `account-api/docs/openapi.json` still lags some routes (see the architecture
  README).
