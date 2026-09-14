# Game audio acceptance evidence

Task: GAME-AUDIO-2026-09-14. Base: `747b991`. Release: `0.19.0-rc.8`.
Verification and publication: **PASS**. All six acceptance criteria pass.

| Criterion | Result | Current evidence |
| --- | --- | --- |
| AC1: Licensed assets | PASS | Seventeen Vorbis files, 1,859,827 bytes; CC0 music, two Kenney adaptations and fourteen original synthesized effects. Decode, sample levels, exact notices and hashes pass in `raw/asset-audit.json`; native tracked-asset inventory passes. |
| AC2: Actual bounded playback | PASS | Seventeen audio policy/headless tests; real native music/effect sinks and advancing music position in three scenarios. Baselines, dedupe, invalid/distant receipts, late assets, no device, rates and focus/mute are covered. |
| AC3: Persistent desktop/mobile controls | PASS | Seven preference/UI tests; normal handlers change music 25% → 20%, mute/resume and save preferences. All nine audio buttons fit in all six rendered captures. |
| AC4: Customizable catalog | PASS | All stable cues and finite gains validated under local audio paths. No protocol, ranked, NFT ownership, server or production dependency changes. |
| AC5: Fresh verification | PASS | Workspace regression plus final client tests, native build, strict Clippy, formatting, source whitespace, Python parsing, asset measurements and native visual review. |
| AC6: Release docs and publication | PASS | Feature `8a2716a` published to GitHub main; both local mains and canonical origin/main agree after fetch. Four unrelated files preserved byte-for-byte. `raw/git-publication.json` records the result. |

## Exact checks and their scope

- `cargo test --workspace --all-targets --locked --offline`: **628 passed**, zero failed, **14 PostgreSQL tests ignored**, across 21 test targets (`raw/workspace-tests-2.log`). No PostgreSQL integration run was performed for this client audio change.
- After client-only final refinements, `cargo test -p client --lib --locked --offline`: **341 passed**, zero failed (`raw/client-tests-final.log`). This includes the final additional music runtime test. These numbers describe two overlapping runs, not 969 unique tests.
- Final native client/server build: PASS (`raw/native-build-final.log`). Final `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: PASS (`raw/clippy-final.log`). `cargo fmt --all -- --check`: PASS (`raw/fmt-final.log`, empty successful output).
- Cargo reuses the leased MATCH-PROGRESSION task target directory with `CARGO_INCREMENTAL=0`. Test/build invocations use debug=0 for dev/test and opt-level=0 for dev and its package wildcard; this avoids another multi-gigabyte build cache. No dependency was added.
- Python AST parsing of the palette builder and three proof helpers: PASS (`raw/python-source-check.log`). Scoped `git diff --check` excludes raw machine output and byte-preserved third-party notices (`raw/source-diff-check.log`). Source/docs/catalog checks include every staged change outside those exclusions.
- `raw/asset-audit.json`: 17/17 files decode as 44.1 kHz Vorbis, all samples finite, no clipped samples, no decoder diagnostics. Music is stereo, effects mono; total 1.86 MB. Original archive/member/output hashes and both exact notices verified. `raw/package-audio-inventory.json` verifies the native packager selects all 17 Ogg plus six metadata/notice files; this is not a full distributable build.

## Native playback and visual proof

`run_native_qa.py` starts and cleans up its own local Practice server/client per case. It uses ordinary menu/input handlers, scripted touch start/end on phone layouts and one explicit UI confirmation cue; it does not synthesize combat events. Fresh isolated configs are checked for persisted music=0.2 and mute=false.

| Mode | Size | Result | Captures |
| --- | --- | --- | --- |
| Desktop, 3D | 1280×720 | PASS | `raw/desktop-3d/01-audio-settings.png`, `02-muted-settings.png` |
| Phone preview, 3D | 844×390 | PASS | `raw/phone-3d/01-audio-settings.png`, `02-muted-settings.png` |
| Phone preview, 2D | 844×390 | PASS | `raw/phone-2d/01-audio-settings.png`, `02-muted-settings.png` |

Each directory includes real client/server logs and `qa-summary.json`: all 17 assets loaded, real music/effect sinks observed, advancing playback position, focus/input unlock, music volume change and mute/resume. Both capture receipts assert all nine audio controls fit the actual panel/window. All six final screenshots were visually reviewed: labels readable, four volume rows and mute visible without clipping, matching 25%/20% and mute states. Older captures revealed a small layout issue; `problems.md` records the fix and final rerun.

## Audio preview, reproducibility and limits

`raw/audio-preview.mp3` is a **32-second authored palette demonstration**, assembled from the bundled music and all sixteen effects. `raw/preview-timeline.json` records cue timings. It is not a recording of the game output and does not prove subjective listening quality.

`sha256.json` binds the curated source/assets/docs/proof to their bytes. The reproducible stdlib/FFmpeg palette authoring script is in `scripts/build_audio_palette.py`; asset audit and preview helpers are retained here. Audit source-archive checks require the separately downloaded original archives, which remain ignored. License pages and exact hashes are in the packaged provenance records.

Physical Android/iOS audio routes, Bluetooth latency, browser autoplay, subjective listening and public deployment/load have not been verified. The 134.4-second music preserves source frames and author-designated loop boundaries; numeric seam measurements do not establish perceptual seamlessness. No database migration or new production dependency is part of this task.

## Publication

Feature `8a2716a91880bd09d8ba9547cb4296a98fcf27a6` was pushed successfully to GitHub main. Both
local main checkouts and the fetched remote main match. Four pre-existing untracked
cinematic files have identical before/after SHA-256 values. The independent
read-only reviewer verified all 78 inventory entries and 79 checkpoint files,
found no staged runtime/download/cache/credential artifacts and confirmed AC1–AC5.
This final publication receipt and acceptance status are a documentation-only
follow-up; tested game source and assets remain unchanged.
