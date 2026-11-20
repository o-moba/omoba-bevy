# UI-2 / UI-3 — exact-source RU Mobile acceptance, 2026-11-10

## Verdict

**UI-2 Draft/Lobby and UI-3 compatibility PASS in the inspected offscreen
synthetic RU Mobile preview at 852×393.** Physical phones, Vulkan Android,
keep-awake, background/resume and haptics are **not** covered; C3 remains open.

The raw JSON files retain their original `captured_pending_visual_inspection`
status. The PASS/FAIL verdicts below were made by visually inspecting the saved
PNG files and checking their recorded text/glyph geometry; historical captures
were not rewritten.

## Accepted states

| State | Exact source | Binary SHA-256 | PNG SHA-256 | Visual and geometry result |
|---|---|---|---|---|
| Compatibility / UI-3 | `19c2616d5150f4a1256eb85a8cb7cf2ea2009478` | `7261729a6805e0efd8df069b920b3f92d74916508af60b9c2445958b78316399` | `4e1243a1ec05bcd3cb581b5f77f07414f15441cd57ad58e45400e8b938b744d2` | **PASS.** The complete Russian compatibility error is readable. The offline-practice hint stays above and clear of bottom navigation. |
| Widest Draft names / UI-2 | `5f3149816ca7f074c5cab205881585e5a0c4feec` | `60f7214fc79dc2ce2e31130068df367ae90a9131a241a5e3bf0d169c80fc96c7` | `ac05b304b7e6115578dc4cfc266bb4ab7191986d926cc0081d6c31f331a8c112` | **PASS.** All five nameplates show `WWW…`, with readable class/role/status text, no ellipsis-only label and no adjacent-text or panel overflow. All 20 stage labels fit their nodes. |
| Widest Lobby names / UI-2 | `5f3149816ca7f074c5cab205881585e5a0c4feec` | `60f7214fc79dc2ce2e31130068df367ae90a9131a241a5e3bf0d169c80fc96c7` | `c4f873832a160ce198248bbf2776143077aa297e5de77712a3313f3a2369c807` | **PASS.** All five podium names show `WWWWW…`; all five roster rows retain a readable W-prefix and ellipsis inside the panel. Leader status and other labels remain within their nodes. |

Accepted files are in [`accepted/`](accepted/). Both binaries are ARM64 Mach-O
RU debug clients downloaded from the run-specific `macos-qa-client` artifact:

- `19c2616d`: [Release 42795705376](https://github.com/o-moba/omoba-bevy/actions/runs/42795705376).
- `5f314981`: [Release 42802254459](https://github.com/o-moba/omoba-bevy/actions/runs/42802254459), SUCCESS on macOS, Windows, Linux and Android. [CI 42802254350](https://github.com/o-moba/omoba-bevy/actions/runs/42802254350) is SUCCESS on all seven jobs.

The build JSON files bind each capture to its source SHA, workflow run, artifact,
build command, architecture and binary hash. Only the affected Draft/Lobby states
were repeated after `5f314981`; Home compatibility was unchanged by that commit.

## Rejected predecessor retained

The earlier `19c2616d` build exposed a real UI-2 defect:

| State | PNG SHA-256 | Rejection |
|---|---|---|
| Draft | `7ad6ea4e6dd4d38005bdf55eb7668665c461f126d3e1370a235bbebe955ba7ae` | All 20 stage labels collapsed to ellipsis-only text. |
| Lobby | `637fddf647eb8e8ac230594732c320789d6b5ea43a952a5b1cf9412e4ca710c4` | All podium labels collapsed to `…`; roster nicknames had no fitting and overflowed the right panel. |

`FittedLabel` had been measuring against a content-sized node whose width shrank
on each elision pass. The fix keeps the available nameplate width stable and adds
no-wrap, clipping, actual-glyph fitting and QA geometry to roster nicknames. The
capture helper now rejects ellipsis-only labels and shaped layouts wider/taller
than their node by more than 1 px. A separate negative run against the old binary
confirmed that the new gate rejects `PrematchStageText-1-0` when it is only `…`.
Original rejected images and capture JSON remain in [`rejected/`](rejected/).

## Recompute and limits

From the repository root:

```bash
python3 docs/evidence/ui-mobile-states-2026-11-10/verify.py
python3 -S docs/evidence/ui-mobile-states-2026-11-10/verify.py
```

Both runs pass: three accepted captures, 41 label geometries and two rejected
captures are checked. The verifier recomputes PNG hashes, source/binary links,
viewport/profile/locale, label bounds and non-empty widest-name prefixes. It does
not replace the recorded visual inspection.

Scope is the production offscreen renderer with real UI-kit actions and synthetic
RU session/views/settings. It is not a live match/account, a desktop/window test,
a physical phone or proof of keep-awake/haptics. The later documentation-only
commit must receive its own complete CI and Release runs; these code-commit runs
are not transferred to it.
