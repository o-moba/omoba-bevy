# Creator catalogue bridge — 2026-10-10

This implements a local first iteration of [PR #78](https://github.com/o-moba/omoba-bevy/pull/78).
The public marketing site lives in `wotori-studio/landing-v2`, app `apps/omoba`;
the full gallery belongs to `o-moba/omoba-web`. Registry/Studio remains the asset
publication and approval owner. No service, database, secret, deployment or
gameplay protocol is changed.

## Delivered source boundaries

- OMOBA: canonical export of classes, skills, basic attacks, presentation/motion
  references and authored briefs. Stable requirement IDs are independent of QWER
  placements. Snapshot and licensed files are hashed; renderer creates references
  from original bundled handheld GLBs without changing those models.
- Portal: public `/creators`, snapshot importer/validation and version-pinned
  contribution preparation. The gallery works without account login or a game
  server. It does not create an approval record or equip a skin.
- Marketing site: creator entry with a configurable catalogue destination;
  when no destination is configured, it offers the design proposal and Studio
  without advertising an undeployed gallery as live.

Existing Studio accepts avatar/weapon uploads and game submissions by project.
Its request does not accept requirement IDs. A prepared brief is manual context,
not a claimed requirement-targeted submission. Existing single-item weapon
selection also keeps one selected item through both Wildspark modes.

World-prop redistribution is intentionally withheld where only an ambiguous
"project license" notice exists. Handheld downloads use the explicit scoped
CC-BY-4.0 grant; all references retain attribution. Static Blender previews show
shape/materials only, not native animation, grip or VFX acceptance.

## Next iterations and acceptance gates

1. **Registry catalogue revision and exact binding.** Import a bounded validated
   snapshot through existing project ownership. Store immutable revision history
   and selected revision; failed imports cannot replace the previous selection.
   Add a requirement association to the existing rendition submission. Bind
   project/catalogue/requirement to the exact asset revision, rendition SHA-256,
   profile and platform. Reject stale/unknown references and foreign-owner writes;
   preserve idempotent retry and decision history. This is the next useful bridge.
2. **Studio targeted creator loop.** Show the same requirements in Games and
   retain selected context through login, upload and validation. Reuse existing
   model and weapon-grip viewers. Review baseline and candidate in the specified
   role. Test fresh upload, rejection, revision and resubmission through UI.
3. **Approved cosmetic sets.** Add explicit complete-set selection for Wildspark
   to Registry, SDK and OMOBA while preserving single-item and empty-hand choices.
   Pin exact approved members for a match and use the full packaged default if a
   required member is unavailable. Prove mode switching on two independent clients.
4. **Projectile/prop and animation contracts.** Define bounded semantic anchors,
   pivots and motion capabilities before allowing those asset kinds. Preserve
   server-owned geometry, visibility, damage and timing. Resolve baseline licensing
   before redistributing additional files. Browser viewing is not native proof.

These are separate reviewable slices across existing repositories, rather than
a new upload service or second approval system. A full SDK package is unnecessary
for static discovery: versioned JSON plus consumer validation is the first SDK
boundary. A lazy interactive viewer can reuse `ekza-avatar-renderer` after its
production dependencies are separately approved.

## Verification

The exporter unit suite passed 18 tests, including deterministic output, unknown
and duplicate IDs, missing source files, path safety, source-evidence drift,
license boundaries and preservation of skill icons through Q/W reassignment.
Repeated export plus `--check` passed. Node independently reproduced the content
revision using the documented canonicalization. Fresh review also ran the portal's
45 catalogue tests and verified 16 media files (six GLBs, six preview PNGs, four
atlases) against the exported descriptors. A modified brief under an unchanged
revision is rejected. All seven generated handheld stills were visually inspected;
only the six current default models are exported.

The snapshot contains 17 classes, 68 skills and 148 requirements. Its final
revision is generated after the implementation commit so `source.commit` resolves
to the exported source. Consumer build/browser evidence is owned by the portal's
`docs/creator-catalog.md`; the marketing entry passed its production build,
10 component/policy checks and 13 HTTP regressions.

Local proof lives at `.agent/tasks/ASSET-CATALOG-20261010/`. UI scope is English
at one desktop browser viewport. No native Rust build was needed because runtime
code is unchanged. There is no physical-phone, hosted-publication, deployment
or native-equipping claim.

## Wildspark release integration

The Wildspark reference branch changes its default repeater and basic rocket body. The exporter now distinguishes the Wildspark override from Riftshot’s original repeater, pins `held_weapons.rs` as source evidence, and checks the active combat rocket model. Updated authored requirements record the original CC-BY-4.0 prop provenance while keeping unsupported projectile/prop contributions unavailable. Static previews include the class-default override. Regression checks cover separate class bindings and procedural-form precedence drift.
