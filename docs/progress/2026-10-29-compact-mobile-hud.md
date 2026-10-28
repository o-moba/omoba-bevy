# Compact mobile HUD — 2026-10-29

The owner supplied OMOBA and annotated Wild Rift screenshots and requested a
smaller battle HUD with more visible world. Feature branch
`fix/compact-mobile-hud` is based on dev `7ace97c`.

Combat artwork is now80px for attack,50px for skills and38px for utilities at the
reference scale, with independent capture regions of at least44px for secondary
actions. Recall and movement utilities sit along the bottom; the joystick keeps
its familiar size with an inward anchor and transparent base. Score, allies,
settings/chat, quick-buy and ability detail badges are smaller. Input ownership,
safe bounds, neighboring actions, saved offsets and upgrade priority are tested.
Menus and desktop retain their existing scale. No gameplay, routing, shared
tokens/assets, dependencies, deployment or CI changes are included.

Runtime HUD source is `d39331c`; `9223165` adds only QA guard/test-fixture repairs.
Formatting and both Clippy modes passed. Workspace Rust tests:2396 passed,
82 opt-in ignored; client1502 passed/1 ignored, including37 mobile-control tests.
Python3.14 with localhost proxy bypass passed612 script tests (9 expected skips)
and55 iOS tooling unit tests in a clean Git worktree, matching CI topology.

A plain local full gate still observes pre-existing CJK-font drift in the adjacent,
independently modified omoba-ui checkout: the game base already contains456860
bytes from7e6ec79, while that handoff has455528 (its HEAD455412). Fonts, manifests,
token locks and synchronization guards remain unchanged. The clean run retains
all installed-asset and token-lock validation; no guard was mocked or disabled.

One fresh native-renderer English phone preview at844×384 passed visual review;
the actual frame and an embedded-reference wipe/opacity comparison are retained.
It is a desktop presentation fixture, not physical Android or multiplayer proof.
The signed arm64 Android candidate0.45.0 build450009 is from clean9223165, package
`space.ekza.omoba.beta`, using the existing playtest signer. Signature, ZIP16KiB
alignment and ELF16KiB LOAD alignment pass. SHA256:
`49ad6a9043aa7306fb57090b4ae15c016f946d695a42c7979a230bbb822c2e4e`.
The physical phone is disconnected, so installation and two-thumb play remain
unverified. No release/publication or server deployment was performed.

Local artifacts and full receipts are under
`.agent/tasks/MOBILE-HUD-COMPACT-20261029/`: `evidence.md`, `evidence.json`,
`raw/native/01-iphone-gameplay-layout.png`, `comparison-standalone.html`, and
`android/Omoba-0.45.0-compact-hud-9223165-android-arm64.apk`. Temporary verification
worktrees, copied key, scratch configuration and package intermediates were
removed; shared build caches were preserved. The feature checkout is retained
for review and device iteration.
