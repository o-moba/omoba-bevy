# DEV account wiring — frozen 2026-10-16

Task: prepare the integrated DEV contour for owner playtesting by wiring native account and portal defaults and keeping Ekza account-library consent in DEV.

## Acceptance criteria
- AC1: Desktop, iOS and Android DEV packages compile `https://account.dev.omoba.io` and `https://app.dev.omoba.io`; inherited shell defaults cannot replace them. Production packager behavior and production iOS source guard remain unchanged.
- AC2: A compiled DEV native account transport rejects production or mixed-contour runtime endpoint overrides before sending a request. Existing ordinary/local transport behavior remains available outside packaged DEV.
- AC3: Account API configured with the exact DEV portal uses DEV Registry and Studio; malformed/alternate origins in DEV DNS zones fail closed. Existing non-DEV production behavior and explicit local fixture validation remain unchanged.
- AC4: Focused Rust and desktop/mobile packaging regressions pass; fresh review confirms scope and release isolation.

- AC5 (root rollout finding): game deployment preserves an already enabled portal database login and password; newly created portal roles remain NOLOGIN and the migration owner remains locked.

## Constraints and non-goals
No new dependencies, workflow changes, production deployment, public stable release, billing-provider activation, or gameplay changes. Root coordinates deployment and fresh review. Build immutable candidates after the code is frozen, record source SHA/checksums, and distinguish built, uploaded, processed, available, and physically tested. Artifact delivery evidence is separate from the source-fix criteria.

## Verification
Rust account transport and EkzaConfig unit tests, relevant Python release/Android/iOS tooling tests, formatter and strict package Clippy. Existing full integrated gameplay CI is baseline evidence; no unrelated gameplay rerun required for this isolated wiring change.
