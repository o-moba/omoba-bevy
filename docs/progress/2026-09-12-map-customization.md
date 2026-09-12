# Configurable map objects and reusable props — 2026-09-12

Version: `0.19.0-rc.5`. Task: `MAP-CUSTOMIZATION-2026-09-12`.

## Authoritative objects

The server loads one validated map profile before accepting players. Stable
structure IDs/keys, lane placement and reusable stat profiles replace the
hardcoded eight-object builder. The embedded default preserves previous object
IDs, positions and combat values. `examples/maps/two-tier.json` demonstrates ten
structures with two mid-lane tiers and independent HP overrides.

The resolved profile is retained across rematches. Lane tier gates derive from
progress toward the enemy, not JSON ordering. Outer defending towers must fall
before inner towers; the base requires one nonempty configured lane to clear.
No configured lane towers means an initially vulnerable base. Damage, minion
selection and live collision use actual authoritative structures.

Shared Verdant geometry supplies layout coordinates and roads to client/server.
Snapshots identify the geometry and active map profile; the client rejects
incompatible geometry before applying it. Unsupported production bounds no
longer silently select an empty static navigation map. Base pads, terrain,
static forest collision and road shape retain the authored geometry contract.

## Presentation architecture

Real Verdant GLB roots already have reusable asset metadata and stable names.
The map registry binds those instances, allowing archetype and individual
appearance overrides. Local GLB replacement, palette/tint and safe decorative
transform changes are applied through one presentation layer. Live structures
retain their server-owned roots and receive the same visual profile mechanism.

A solid prop retains its authored transform and physical footprint. Replacement
models explicitly declare collision preservation and pass horizontal size/pivot
checks; incompatible or missing models retain the authored drawable fallback.
Nonblocking decoration can move and vary visually. Terrain and walkable bridge
geometry are excluded from cosmetic prop mutation. Mesh/material caches are
bounded and existing F4 foliage ownership is preserved.

The 2D renderer uses registry sprite keys/tints with its existing atlases, and
its real structure proxies remain linked to authoritative IDs. GLB replacements
do not manufacture new 2D art. Five packaged prop models are exact original
library copies with source paths and SHA-256 provenance.

The [contributor guide](../map-customization.md) documents startup, schema,
placement and profile examples, collision boundaries and testing requirements.

## Verification

Fresh final verification passed **481 Rust tests** with zero failures or ignored
tests, strict workspace Clippy with warnings denied, and formatting. Thirty
positive/negative capture-verifier checks also passed.

Five actual native scenarios produced nineteen verified frames: default and
two-tier maps, desktop and phone-size preview, and both 3D/2D renderers. The
independent UDP observer confirms configured IDs, positions, max HP and siege
protection at the exact captured server tick. The 3D check loads all 942 authored
props; repeated A→B→A→B swaps retain their owner and bounded caches. The 2D check
includes 3025 loaded terrain tiles; manual inspection caught and corrected a
QA-only camera clipping issue before final captures.

[Seven unedited native screenshots and provenance](2026-09-12-map/captures.json)
are published alongside the note. These use scripted overview/detail cameras,
not normal gameplay zoom. [Task evidence](../../.agent/tasks/MAP-CUSTOMIZATION-2026-09-12/evidence.md)
maps AC1–AC6 to raw results and records discovered/fixed issues.

## Limits

This is an object-tuning and presentation extension on the supported Verdant
arena, not a general terrain editor or live map authoring service. Changing
roads, pads or static obstacle positions requires a matching geometry/collision
export and revision. Gameplay changes take effect on server restart; there is
no mid-match gameplay hot reload, remote asset download or new dependency.
Native mobile UI preview remains separate from physical Android/iOS testing.
The alternate tower profile demonstrates mechanics rather than human balance.
