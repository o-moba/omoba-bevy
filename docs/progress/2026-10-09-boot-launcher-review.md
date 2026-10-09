# Boot launcher merge review — 2026-10-09

PR #75 originally treated every non-empty unlisted `OMOBA_*` variable as an automation switch. `scripts/beta_launcher.py` sets the asset path, match mode, team size and `OMOBA_DEBUG_UI=0` for ordinary player sessions, so those sessions incorrectly skipped the splash.

The splash now accepts these launcher settings and uses the debug console's existing flag parser to distinguish a disabled debug UI from an enabled one. Unknown non-empty automation switches still bypass it, as do sandbox launches. Empty values remain inactive.

Regression tests cover practice and release launcher environments, an automation switch alongside player settings, disabled and enabled debug values, and empty capture variables. No gameplay, protocol, version or deployment settings change.

Validation: `make check` passed (formatting, workspace clippy, client clippy without QA, workspace tests and Python tooling tests). The client suite includes both new regression tests: 965 passed, one ignored. Device performance and physical iPad preview behavior were not rechecked for this environment-filter change.
