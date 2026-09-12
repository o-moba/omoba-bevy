# Verification fixes

- Initial server movement fixture remained in Lobby, where movement must be blocked. Set Running and assert positive approach progress as well as collision.
- New strict-schema test exposed serde flattened placement ignoring extra base coordinates. Replace it with a strict explicit decoder; preserve the public JSON schema.
- Structure-footprint broad phase initially reused hero-sized spatial lookup. Query bins over the full requested radius and test an obstacle across a bin boundary.
- First client test run: 265 passed, two new fixture failures. Real glTF scene test lacked reflected component registration; 2D white-color comparison used different equivalent color representations. Fix fixtures while retaining real asset loading.
- Cross-review found actual glTF instances have an intermediate scene root. Traverse bounded ancestors to import real prop metadata, and verify the full environment/foliage scenes (942 eligible props, 236 solids).
- Cross-review found stale structure presentation caches after same-ID kind/team changes and stale 2D cue positions after sprite-height changes. Recreate only the affected 3D child and reconcile 2D cues while retaining authoritative owners.

- Native screenshot inspection caught the old HUD/help instruction to destroy any one tower. Updated desktop/mobile goals and protected-target feedback for complete lane chains; recapture the final client after this wording fix.

- Full-workspace UDP regression caught old convergence fixtures sending bots straight through live towers. Updated both helpers to the production path navigator with authoritative live structure discs; retained the original timing, range and combat assertions.
- Final configuration review found longitudinal offsets could reverse physical tower order while tiers still followed nominal lane progress. Reject reversed effective progression and add a regression.
- The same review found minion structure reach used model-center height, unlike ground-plane unit combat; a valid three-meter lateral offset could become unreachable to melee units. Use horizontal reach while retaining the projectile aim point, and test actual melee damage against offset towers.
- Manual inspection rejected preliminary 2D captures despite structure-only verification: QA placed Camera2d at z=999 with far=1000, clipping terrain bands at negative Z. Production uses z=0. Match production QA placement and require loaded, visible, camera-reachable terrain tiles in the verifier; retain the rejected preliminary capture separately.
- Preserved authored lane sampling includes decorative end spurs intentionally omitted by minion marching. Reject configured towers farther than three meters from the actual walked polyline; mirrored spur regressions preserve default/example and maximum lateral offset support.
