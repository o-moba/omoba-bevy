# Hub Classic hero design foundation — 2026-10-30

Base: dev a5562f1. The owner requested one recognizable canonical hero design per
class, derivative cosmetics and a coherent Hub → Studio → game contributor route.

The existing game-contract exporter now also emits a versioned `hero-designs.json`
from a reviewed game-owned source. Every one of the 17 current class IDs has an
explicit unconfigured row: no library model or base art is guessed. The companion
pins its game-contract revision, input hashes and immutable content revision. A
strict authoring validator rejects generic `any`, unknown/mismatched classes,
unconfigured bases, wrong modes, stale design/contract revisions and altered base
hashes. New definitions extend the existing `omoba.character.v1` schema without
changing `ekza.package.v1`, the core animation contract or v1 rendition semantics.

The eight new policy/binding tests, seven game-contract tests and nineteen asset
catalogue tests pass. Export and `--check` pass; repeated export is deterministic.
The core game-contract file remains byte-identical to dev. No native Rust code
changed, so no Cargo/Bevy build was run. Local task evidence is in
`.agent/tasks/HUB-HERO-DESIGN-20261030/`.

This is an authoring/source foundation. It grants no entitlement or game approval.
Native Classic admission, C3 package playback, licensed base/art assignments,
Hub publication/selection, migration of older approvals and two-client visual
acceptance remain separate gates. No secret, migration, service, deployment,
published release or existing approval was changed. See [hero designs](../hero-designs.md).
