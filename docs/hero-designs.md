# Classic hero design policy: authoring foundation

OMOBA Classic has one canonical recognizable hero design per class. A skin derives
from that approved base; a compatible humanoid skeleton alone does not make an
arbitrary avatar a Classic skin. Gameplay, kits, hitboxes, timing and authoritative
movement stay game-owned. Workshop and other explicitly supported modes keep their
own admission policy.

## Current implementation boundary

The game-owned exporter now publishes `hero-designs.json` beside the existing
`game-contract.json`. This is the first identity/authoring foundation, not a new
native admission path, approved skin catalogue or deployment. Every committed class
is **unconfigured**: the game team has not assigned a verified base/art contract.
Consumers must show that block instead of guessing a model from the roster.

The existing v1 avatar path is preserved. The animation contract advertises
`ekza.game-contract.v1`, `omoba.character.v1`, `ekza.package.v1` and the intended
`desktop/humanoid-glb-v2` rendition. Exporting that package does not establish
native C3 parsing/admission/playback. Classic identity checks and C3 still need
independent server/SDK/client implementation and two-client acceptance.

## Game-owned authoring input and immutable output

Edit `shared/assets/catalog/hero-designs.source.json` only after game-owner review.
It contains `schema: omoba.hero-designs.v1`, `gameId: omoba`, `modeScope: classic`
and exactly one ordered row for each class in `heroes.json`:

```json
{
  "classId": "wildspark",
  "status": "unconfigured",
  "heroDesignId": null,
  "baseRevision": null,
  "baseSha256": null,
  "designContractRevision": null
}
```

A configured row must supply all four values. `heroDesignId` is a stable reviewed
identity, unique across classes. `baseRevision` and `designContractRevision` are
immutable `sha256-<64 lowercase hex>` identifiers. `baseSha256` hashes the exact
licensed base model bytes. A revision identifies the complete authoring base or
design contract; it is not a mutable slug or URL. Assignment requires an accessible
licensed authoring kit and a reviewed design brief (silhouette, proportions, visual
landmarks, rig/sockets and rendering budgets). Syntactically valid hashes do not
prove those art/licensing facts: game-owner review is required before configuring
and publishing a trusted policy.

The output adds `gameContractRevision`, `source` (`commit`, exporter and canonical
input hash) and `revision`. The latter is `sha256-` plus SHA-256 of canonical JSON
with only `revision` omitted, using the existing game-contract canonicalization.
The game contract's bytes/revision remain independent and unchanged by the new
companion. Changing either the game-contract revision or hero policy creates a new
companion revision. Unrelated commits preserve valid previous provenance.

```sh
python3 scripts/export_game_contract.py --output shared/assets/catalog
python3 scripts/export_game_contract.py --output shared/assets/catalog --check
python3 -m unittest discover -s scripts -p test_hero_designs.py -v
```

The JSON shapes extend the existing profile schema's `$defs/heroDesignPolicy`,
`$defs/heroDesignRecord` and `$defs/classicBinding`. They do not change the existing
package schema or insert an unrecognized field into legacy manifests. Strict
cross-reference validation lives in `scripts/hero_designs.py` for authoring tools;
consumers must implement the same fixture-backed rules at their own boundary.

## Classic binding

A candidate contribution supplies the following context separately from existing
package bytes until an additive consumer protocol carries it:

- `gameId`, `modeScope`, `classId`;
- `heroDesignId`, `baseRevision`, `baseSha256`, `designContractRevision`;
- exact `gameContractRevision` and `heroDesignsRevision`.

`validate_classic_binding` compares every field with the **trusted selected**
game-owned policy and a concrete target. `any`, unknown/mismatched classes,
unconfigured assignments, wrong modes/hashes/revisions, unknown fields and stale
contracts fail closed. A caller must never use an uploader's self-hashed policy as
its authority. Integrity and game-owner selection are different checks.

Passing this necessary authoring check grants no publication, game approval,
entitlement or runtime availability. The full consumer gate additionally binds
exact asset/revision/rendition hash and size, approved class/mode, selected game
release, current ownership and client capabilities. It must be enforced by the
authoritative server as well as the SDK/client; old clients must not receive v2
packages through the v1 selector. No new untrusted fields are added to Join packets
by this foundation.

## Migration and next gate

Earlier general-avatar/class independence is superseded for **Classic**. Existing
asset identities, approvals and purchases remain historical records; this change
does not silently revoke or relabel them. A game-owner migration must select
re-review/binding, a supported legacy mode or withdrawal with user communication.
Running matches retain their pinned selection under an explicit revocation policy.

Next: assign one licensed canonical base and art contract; expose this policy in
Hub; retain binding through Studio and candidate review; implement native
admission/C3 and a deterministic fallback; prove identical approved custom motion
and skin identity on two independently cached clients. Until then Hub must not
label any class or contribution Classic-ready merely from this export.
