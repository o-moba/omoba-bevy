# Creator asset catalogue

OMOBA exports a static, game-owned catalogue for the website's creator gallery.
It describes all 17 classes, their 68 skills, current basic attacks, semantic
motion references and authored cosmetic requirements. The executable game and
combat protocol do not consume this document in this iteration.

The original [Studio proposal (PR #78)](https://github.com/o-moba/omoba-bevy/pull/78) remains the
roadmap for Registry publication, requirement-targeted review and playable sets.
This export implements discovery and preparation of a contribution brief. It does
not add a Registry endpoint or imply that publishing a model approves it for a
particular requirement.

## Export and check

Python 3.10 or newer is sufficient; there are no third-party Python dependencies.
From an explicit game checkout:

```sh
python3 scripts/export_asset_catalog.py --output /tmp/omoba-asset-catalog
python3 scripts/export_asset_catalog.py --output /tmp/omoba-asset-catalog --check
python3 -m unittest discover -s scripts -p test_asset_catalog.py -v
```

The directory contains `catalog.json` and content-addressed `files/<sha256>.glb`
or `.png` files. A consumer can copy the entire directory to its public
`asset-catalog/` directory. The default local URL prefix is `/asset-catalog`;
`--public-prefix /another/path` changes it. No remote URLs are fetched. Existing
unrelated files are retained; corrupt files under a content hash fail validation.
`--check` verifies the exact expected JSON and bytes without changing any files.

Optional still previews can be rendered with the repository's Blender helper:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background \
  --python scripts/render_asset_catalog_previews.py -- \
  --output /tmp/omoba-asset-previews
python3 scripts/export_asset_catalog.py --output /tmp/omoba-asset-catalog \
  --preview-dir /tmp/omoba-asset-previews
```

Preview inputs must be PNG files named `<model-sha256>.png`. The exporter checks
their header and bounded dimensions, hashes their bytes, and copies them under
their own content hash. `baseline.preview` is explicitly `null` when no image is
provided. Preview bytes participate in `inputsSha256` and catalogue revision.
These are static, studio-lit model renders. They do not reproduce the native
camera, held pose, animation timing, VFX, team markers or combat conditions.

## Contract

The top-level discriminator is `schema: "omoba.asset-catalog.v1"`, with
`schemaVersion: 1` and `projectId: "omoba"`. `revision` is `sha256-` followed by
the SHA-256 of canonical, key-sorted compact JSON before adding the revision.
Canonical keys are ASCII, strings preserve Unicode, and numbers are finite and
within JavaScript's safe range. Integer-valued floats become integers; fractional
metadata uses ordinary decimal notation and has magnitude at least `0.000001`.
Browser importers can reproduce this hash with recursively sorted keys and
`JSON.stringify`, and should reject semantic changes under an unchanged revision.
`source.commit` is the checkout's HEAD; `source.inputsSha256` hashes a sorted map
of every consumed source path and byte hash, including the authored bindings,
exporter itself, license notices and optional previews. Uncommitted source changes
are therefore identified by their content digest even though HEAD is unchanged.
`source.gameplayRevision` comes from the reusable skill catalogue. Schema,
gameplay and content revisions are separate concepts.

| Collection | Meaning |
| --- | --- |
| `classes` | Stable class ID, name, description, role, ordered `skills: [{slot,id}]`, basic attack and requirement links. Basic attack IDs currently equal class IDs. |
| `skills` | Stable skill ID, name, description, requirement links and optional icon sprite descriptor. Q/W/E/R placement belongs to the class, not skill identity. |
| `requirements` | Stable game-scoped ID, role, brief, derived `classIds`, exact `uses` edges, support status, technical profile and baseline. |

`uses` contains exactly one `skillId` or `basicAttackId`, with optional semantic
`phase` and `state`. IDs are lowercase and restricted to letters, digits,
underscore, hyphen and period. Unknown/duplicate IDs, dangling or ambiguous edges,
missing source files, unsafe paths, source-schema changes and missing motion or
model references fail closed.

Roles are `held_weapon`, `projectile`, `world_prop`, `engine_effect` and
`animation_reference`. Support states are deliberately narrow:

- `supported`: a bundled static handheld has the existing `handheld-glb-v1`
  technical profile. An artist can use the existing Studio weapon workflow.
  Exact requirement approval, cosmetic-set release and automatic game binding
  are **not** implied.
- `planned`: a prop/projectile usage is documented, but contribution validation
  profiles and game binding are future work.
- `engine_owned`: a procedural effect or shared semantic motion is reference
  material; this is not a replaceable model or executable upload slot.

A model baseline includes `sourcePath`, `sha256`, `bytes`, local `url`, license,
attribution and nullable `preview`. A procedural/reference baseline can also
carry a source hash and metadata in `details`, but has no download URL. Such a
hash identifies the documented source file, not an imaginary generated GLB.
Animation `details` includes semantic motion ID, source clip, duration, looping
and nullable contact time; raw retargeted pose arrays are not exported.

Skill icons reuse the four licensed source atlases. Each descriptor records the
atlas's local URL/hash/size, license/credit, pixel rectangle `x/y/width/height` and
`atlasWidth/atlasHeight`. Consumers should use these sprite coordinates, not
display the entire atlas as one skill image. No icon generation is performed.

## Maintaining the authored layer

Canonical names and skill definitions come from
`shared/assets/catalog/heroes.json` and `skills.json`. The first five classes have
20 inline abilities; the other 12 reference 48 reusable skills. Presentation and
motion metadata come from `skills.skillfx`, `combat_visuals.json` and the shared
humanoid motion resource.

`shared/assets/catalog/asset-requirements.json` supplies the semantic facts that
cannot be inferred safely from a filename: handheld mode uses, baseline role,
artist brief and extension boundary. Its `iconAtlases` assigns each sprite cell
to a semantic skill ID independently of current Q/W/E/R placement. `sourceEvidence`
pins the audited handheld default mapping, symbolic prop loader, native icon
mapper and skill-ID ordering by hash. Any change to these sources
requires reviewing the authored bindings before updating those pins. Do not
blindly refresh hashes to silence validation. This explicit review boundary
avoids creating a partial Rust parser in the exporter.

Every current `body.model` and secondary `aux` model must have an authored
requirement linked to its skill. Existing unused GLBs are not automatically
advertised. Generated IDs `skill.<skillId>.presentation`,
`basic.<classId>.presentation` and `motion.<motionId>` are reserved and cannot be
overridden by an authored requirement.

Wildspark illustrates why semantic bindings matter: Q has two handhelds;
rocket-mode basic attacks normally use the procedural `tumbler`, despite a
configured GLB fallback; Last Spark uses a different flying rocket; Snapline's
three traps reuse one model. Shockline and explosion effects remain engine-owned.
The exporter asserts the current tumbler precedence so a presentation change
cannot silently preserve an obsolete brief. Explicitly selected handhelds still
remain selected in both weapon modes; coordinated two-gun skins are future work.

Only the six current class-default handheld GLBs are publicly copied, with the
CC-BY-4.0 grant from `client/assets/weapons/LICENSE.md`. The seven-item weapon
manifest also contains a hammer that is not currently a class default. Icons
retain CC-BY-4.0 attribution and motion references retain Quaternius CC0 credit.
World-prop notices currently lack an unambiguous asset-specific redistribution
grant, so their source hashes are references and their GLB bytes are withheld.
No new license is assigned. Avatar files are outside this export.

## Next integration slices

The website can pin a contribution brief to `projectId`, catalogue `revision` and
`requirementId`, link to the supported Studio surface, and let the creator carry
that context into an existing upload. Completing the loop requires Registry to
store an exact requirement-to-rendition association and provide owner review.
After that, SDK/runtime work can admit a complete reviewed cosmetic set, keep
asset identities pinned for a match, and fall back to packaged defaults.
Projectile/prop profiles and constrained motion capabilities need their own
validation and native-game proof. These later stages do not require changing
the catalogue's discovery purpose or moving gameplay authority into uploads.
