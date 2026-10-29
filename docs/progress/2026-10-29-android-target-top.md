# Android target panel anchoring — 2026-10-29

The owner reported that the selected target sat below the top center on their
OPPO PKB110. The phone has a2374×1080 landscape framebuffer and480dpi override;
a791×360 native preview reproduces the unwanted48px downward shift. Existing
`crowded_top` geometry gave the full-width allies row priority below796px.

`hud_layout.rs` now reserves the top safe-area center for the selected target.
When allied portraits no longer fit beside it, their actual134px row moves below
and clear of the service column. Target width shrinks symmetrically only where
the minimap/score need space; `edge_hud` already resizes its name/HP column.
Brush/action notices follow the secondary lane and avoid enemy portraits and
the left kill feed. Desktop geometry and input behavior are unchanged.

The focused proof is under `.agent/tasks/ANDROID-TARGET-TOP-20261029/`: one English
791×360 native-desktop profile, one affected selected-target state before/after,
geometry regressions at the former795/796 threshold and narrower supported
widths, and independent review. These captures use synthetic local actors and
are not physical Android or live multiplayer evidence.

The private Android candidate will use cacheB, existing Beta signer/package and
increased build450011. Source/hash, installed version and any physical-device
limits are recorded in the task receipts. No public release, production changes,
server/protocol edits, dependencies or CI changes are part of this correction.

A separate pre-existing limitation remains on extremely narrow568px profiles:
a full-width transient action-feedback toast can cross upper combat artwork.
This correction does not certify tiny-phone toast comfort or a full device matrix;
the focused visual profile is the owner's approximately791×360 OPPO layout.

Checks on the repaired source pass2416 Rust tests (1522 client tests), with88
opt-in ignores, both Clippy configurations and final formatting. The local
`make check` Python leg again reports the known adjacent CJK-font handoff drift:
612 tests,9 skips and exactly1 installed-manifest failure. Scripts, fonts and
token assets are unchanged. No asset rewrite or bypass is used.
