# Configurable map objects and structure tuning

Frozen: 2026-09-12. Base: e3e30fb. Task: MAP-CUSTOMIZATION-2026-09-12.

## User intent

Commit/push fresh updates. Make the map customizable through reusable prop archetypes and model substitutions. Expose high-level gameplay-object placement and tuning, especially tower positions, counts, health and threat, so later playability iterations do not require scattered code edits.

## Acceptance criteria

- AC1: A versioned, validated server map-object definition supports stable structure IDs/keys, variable lane tower count and placement, and HP/range/damage/cooldown profiles with per-instance overrides. Embedded defaults preserve the existing eight structures and numbers. An explicit server startup file override fails with useful errors when invalid and remains pinned across rematches.
- AC2: Variable tower configurations preserve coherent siege progression: defending lane tiers unlock in order, the base requires a configured lane to be cleared, and zero-tower configurations are defined. Actual targeting, navigation discs, damage protection, replication and round reconstruction use configured live structures. Validate placement against the supported arena and reject overlap/invalid IDs/unsafe numeric input.
- AC3: The shared supported arena geometry has one canonical definition consumed by server and client; snapshots identify compatible geometry so clients cannot silently join a different terrain/collision version. This iteration tunes objects on Verdant's existing authored terrain; arbitrary terrain rescaling, relocating bases off their authored pads and editing collision polygons require a separate versioned geometry export.
- AC4: A client map-presentation registry maps real repeated authored props to stable archetypes and instances, with configurable packaged model, color and safe transform variations, and reusable live tower/base visuals. Overrides affect actual rendered instances, preserve owner teardown and F4/mode behavior, bound caches, and fall back safely for invalid/missing assets. Solid prop changes preserve the authoritative collision footprint; terrain/roads are not silently relocated by cosmetic data. Both 3D and 2D have explicit usable presentation contracts.
- AC5: Fresh meaningful Rust tests and UDP verification cover custom tower count/positions/stats, invalid configurations, tier gates and reset persistence. Native desktop and touch-preview captures verify real configured objects plus repeated-prop replacement against server state and drawable loaded geometry; retain raw results and screenshots. Physical phone and human balance testing are reported separately.
- AC6: Update version, changelog/features, contributor map-tuning guide and session note; preserve unrelated cinematic files; commit and non-force push verified work to main with AC evidence.

## Assumptions and limits

Server simulation owns gameplay settings; client presentation cannot change HP, reach or collision. Map tuning takes effect on server restart; no live mid-match edits or remote arbitrary asset downloads. Configured tower positions are constrained to supported lane corridors and valid clearance. Base anchors, walkable terrain, roads and static collision remain the shipped Verdant geometry. Existing repeated scene roots already carry asset_id/role metadata and should be reused rather than duplicating or regenerating artwork. No production dependency, deployment or secrets changes.

## Verification plan

Use the existing native build cache and locked dependencies. Run focused tests then a fresh workspace pass, strict Clippy and formatting. Exercise an alternate arena profile through real server startup and a UDP observer. Capture configured structures and model-swapped prop geometry with the running client in desktop and phone-preview modes; include 2D compatibility checks. Publish compact proof, exact binary/config/image hashes, limitations and an actionable contributor example.
