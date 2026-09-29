# Illustrated skill actions — 0.30.2

- [x] Audit all 64 Q/W/E/R skills across the 16 selectable classes.
- [x] Generate sixteen coordinated, action-specific four-icon sheets with
  Higgsfield GPT Image 2.5, high quality, 2k.
- [x] Review every icon at 64px with a circular crop; identify Longshot and
  Sheltering Leap for a clearer second pass.
- [x] Integrate the two refined single-icon generations and recheck their crops.
- [x] Preserve the three runtime atlas paths and ability-ID mapping; ship 256px
  RGBA cells and remove the old placeholder generator and unused SVG.
- [x] Record prompts, jobs, source hashes, packing steps and licensing.
- [x] Check the final assets, mapping tests and repository gate.
- [x] Inspect representative native English 1280×720 hotbars.

Scope is art only: no changes to skill effects, recipes, balance or input timing.
Originals and raw proof stay local in `.agent/tasks/SKILL-ART-2026-09-29/`.
Other languages and physical mobile devices are outside this focused check.

During the legacy hotbar capture, the Python UDP capture helpers rejected the
current protocol-4 frame header because they still hard-coded version 2. The
three receiver checks now read `catalog.protocol_version()`, matching their
hello packets and the Rust transport. The real-UDP regression fixture follows
the same version and still verifies rejection of an older header. No game
transport or protocol version changed. The failed first capture is preserved.

The combat observer now joins the target's blue team for normal shared vision;
it remains passive and sends no movement, attack or skill commands. An unjoined
endpoint correctly receives empty prejoin snapshots, so it cannot prove combat.
The final Ranger capture passes the full independent event/projectile/HP checks.
The generic hello-only observer retains its original semantics.

Verification: 1,331 Rust tests passed (39 existing environment-dependent tests
ignored); final Python checks passed 150 tooling tests and 43 iOS tooling tests.
Formatting, both Clippy configurations and native client/server build passed.
All 64 art cells were reviewed at 64px; final native captures cover Dawnweaver
(3 inspection states), five roster classes and Ranger (3 combat states), all
in English at 1280×720. Final native summaries pass and logs have no runtime
errors. Actual mobile hardware and other locales were not tested.
