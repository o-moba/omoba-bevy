# Compact mobile HUD — 2026-10-29

The owner supplied OMOBA and annotated Wild Rift screenshots and requested a
smaller battle HUD with more visible world. Feature branch
`fix/compact-mobile-hud` started from dev `7ace97c` and includes current dev
`8f638f1` through merge `6cd7e69`. PR: https://github.com/o-moba/omoba-bevy/pull/141.

Combat artwork is now 80 px for attack, 50 px for skills and 38 px for utilities at the
reference scale, with independent capture regions of at least 44 px for secondary
actions. Recall and movement utilities sit along the bottom; the joystick keeps
its familiar size with an inward anchor and transparent base. Score, allies,
settings/chat, quick-buy and ability detail badges are smaller. Input ownership,
safe bounds, neighboring actions, saved offsets and upgrade priority are tested.
Menus and desktop retain their existing scale. No gameplay, routing, shared
tokens/assets, dependencies, deployment or CI changes are included.

Runtime HUD source is `d39331c`; `9223165` adds QA guard/test-fixture repairs.
Incoming dev registration/API changes leave the battle HUD sources unchanged.
Formatting and both Clippy modes passed. Integrated workspace Rust tests: 2401 passed,
88 opt-in ignored; client 1507 passed/1 ignored, including 37 mobile-control tests.
Python 3.14 with localhost proxy bypass passed 612 script tests (9 expected skips)
and 55 iOS tooling unit tests in a clean Git worktree, matching CI topology.
Those script/tooling sources are unchanged after the dev sync.

A plain local full gate still observes pre-existing CJK-font drift in the adjacent,
independently modified omoba-ui checkout: the game base already contains 456860
bytes from `7e6ec79`, while that handoff has 455528 (its HEAD 455412). Fonts, manifests,
token locks and synchronization guards remain unchanged. The clean run retains
all installed-asset and token-lock validation; no guard was mocked or disabled.

One fresh native-renderer English phone preview at 844×384 passed visual review;
the actual frame and an embedded-reference wipe/opacity comparison are retained.
It is a desktop presentation fixture, not physical Android or multiplayer proof.
The signed arm64 Android candidate 0.45.0 build 450009 is from clean `6cd7e69`, package
`space.ekza.omoba.beta`, using the existing playtest signer. Signature, ZIP 16 KiB
alignment and ELF 16 KiB LOAD alignment pass. SHA256:
`145f8a62ed5d4991c76ccc9eedc3394ac9c7d13d9cf70ec0e63dc1a6eab923b3`.
The physical phone is disconnected, so installation and two-thumb play remain
unverified. No release/publication or server deployment was performed.

Local artifacts and full receipts are under
`.agent/tasks/MOBILE-HUD-COMPACT-20261029/`: `evidence.md`, `evidence.json`,
`raw/native/01-iphone-gameplay-layout.png`, `comparison-standalone.html`, and
`android/Omoba-0.45.0-compact-hud-6cd7e69-android-arm64.apk`. Temporary verification
worktrees, copied key, scratch configuration and package intermediates were
removed; shared build caches were preserved. The feature checkout is retained
for review and device iteration.
