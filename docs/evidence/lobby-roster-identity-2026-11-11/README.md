# Lobby roster identity acceptance — 2026-11-11

## Result

**PASS for the roster-identity review finding in the synthetic RU Mobile
852×393 offscreen preview.** Long nicknames with the same prefix remain elided,
while their distinct public handle tags stay readable beside the kick action.

The accepted PNG shows leader `#1000` and visible rows `#1001` and `#1002`; the
latter two are directly beside `Исключить`. Their common W-prefixes are elided
without overlap or clipping. The QA state contains the exact five-tag set
`#1000`…`#1004` with valid geometry. `#1003` and `#1004` are below the visible
scroll viewport: they are verified in retained state, not claimed to be visible
simultaneously in this PNG. The raw capture correctly retains
`captured_pending_visual_inspection`; this README records the separate visual
verdict.

## Exact-source provenance

- native source and RU debug ARM64 Mach-O binary: `f8f68c3d5ff0c74d3dc15489230c9a2abda59c3e`;
- binary SHA-256: `fb3540d65d774e91c484915a11e5f8183100940e12a41fe9d23d52898224b9a5`;
- binary artifact: `macos-qa-client` from fully successful four-platform
  [Release 43010114221](https://github.com/o-moba/omoba-bevy/actions/runs/43010114221);
- capture helper and `head_at_capture`: `95d38e07f5f91e465869402d331e0d0de6b35187`;
- PNG SHA-256: `9005d8123837015270ac661d22ce1108da5d33af92dcd7d1795e2abe04538fbc`.

Between those commits only the temporary review workflow and Python capture
helper changed. The 19 native source files archived under `native-source/` were
extracted from the binary's source commit and their SHA-256 values match the
source hashes recorded by the capture at its later HEAD. The verifier recomputes
those links rather than treating the binary as a build of the helper commit.

## Reproduce archive checks

```sh
python3 docs/evidence/lobby-roster-identity-2026-11-11/verify.py
python3 -S docs/evidence/lobby-roster-identity-2026-11-11/verify.py
```

The standard-library verifier checks retained hashes, binary/source/runner links,
PNG dimensions, locale/profile, all five exact tags and every captured label's
geometry. It does not automate the visual judgement recorded above.

## Limits

This is a synthetic offscreen UI acceptance, not a physical phone, live account,
WAN, capacity, recovery or RU shipment test. It does not change the original
62 done / 5 partial / 1 open finding count or RU **NO SHIP**. The final
documentation SHA still needs its own CI and Release, and both PRs need
independent human review.
