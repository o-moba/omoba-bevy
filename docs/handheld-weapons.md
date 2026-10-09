# Handheld weapons

A handheld is a cosmetic asset, independent of hero class, VRM appearance and
combat inventory. Warrior defaults to Forge Sword; Adventurer to the dagger;
Ranger to Verdant Bow; Wildspark to the repeater or launcher matching its
accepted weapon mode; Riftshot to the repeater; Dawnweaver to Dawn Scepter.
Explicitly equipped local/SDK skins and empty hands override those defaults. Combat Test's **Hero / Enemy
→ Handheld appearance** selects the sword, hammer, scepter, an installed Ekza
item, class default or empty hands. These choices are stored in sandbox presets,
validated by the server and replicated to both local and remote actors. Any
class can select any installed handheld in Combat Test; changing a prop never
changes damage, reach, projectiles or the skill recipe.

## Current product route

The October 1–2 typed lifecycle supersedes the historical operator-import pilot
below. Studio supports weapon upload/publication and separate OMOBA review.
The SDK reads `/v2/assets?kind=weapon&project=omoba&platform=desktop&profile=handheld-glb-v1`;
Collection → Weapons and draft equipment selection download verified approved
files into the writable runtime cache. Peers install the same immutable IDs;
the server independently checks eligibility. A per-model packaged manifest edit
or game rebuild is not required for this route.

The hosted free-weapon route and two desktop clients have recorded pilot
acceptance, not a fresh production availability check in this document. Mobile
runtime integration exists; physical iPhone/Android visual and performance
acceptance is still separate. Paid ownership/checkout remains future work.

Artists must currently export valid embedded grip metadata: Studio has a generic
model preview, but no grip editor or avatar-hand placement preview. The export
contract below is current. Priorities for making this authorable without an
engineer are in the [creator roadmap](plans/creator-loop-and-class-builder.md).
See [typed lifecycle and runtime ownership](progress/2026-10-01-asset-lifecycle.md).

## Attachment

`client/src/held_weapons.rs` attaches Scene0 under the actual hand entity in the
model instance. The existing VRM/skin-index binding supplies `rightHand` or
`leftHand`; it never searches bone names or another player's model. The palm
frame is cached per rig from its rest-pose wrist, fingers and forearm. Fingers
are optional: rigs without them use an anatomical fallback. Rigs without the
chosen hand show no prop rather than one at world origin.

The prop follows the skeletal hierarchy during idle, run, attack and death.
Unequip, avatar replacement, actor removal and switching to Sprite2d remove the
attachment. Model normalization scales the prop together with the character.
No IK, new collider, root motion or per-frame transform copying is required.
Adventurer uses a right-hand thrust for contact attacks. Wildspark uses a burst-fire motion for the repeater and a heavy shot for launcher rounds; its named rotor and recoil slide are animated only after accepted actions. W/R temporarily select the launcher for class-default equipment. The default repeater uses `weapons/wildspark-repeater.glb`; Riftshot and explicit legacy repeater skins retain `wild-repeater.glb`. Pausing the sandbox freezes these parts and muzzle flashes use the real hand socket. The new bow currently uses the existing ranged avatar
motion path: dedicated bow-draw animation, string deformation and two-handed IK
remain separate work. No visual prop changes the accepted attack profile.

## One contract for shipped and Ekza assets

Shipped catalog: `client/assets/weapons/manifest.json`. Each entry has stable
`id`, display `name`, packaged `model` path and `grip`. The same grip is embedded
inside the model's hashed GLB JSON:

```json
"asset": {
  "version": "2.0",
  "extras": {
    "ekza_handheld_v1": {
      "bone": "rightHand",
      "offset": [0, 0, 0],
      "rotation_degrees": [0, 0, 0],
      "scale": 1
    }
  }
}
```

Author origin at the handle's grip point, +Y along the blade/shaft, +Z towards
fingertips in the palm frame. Offsets are metres in that canonical frame,
rotations are XYZ degrees. Grip bounds are ±0.3 m, ±360°, scale 0.25–2.
Models must be self-contained static GLB (≤8 MiB), one Scene0, ≤64 nodes and
≤16 meshes. Skins, animation channels and unsupported required extensions are
rejected. The runtime's humanoid bone adapter belongs to the avatar; the weapon
itself does not require a humanoid skeleton.

The [profile document](profiles/handheld-glb-v1.json) describes
`omoba / desktop / handheld-glb-v1`. The existing **Ekza SDK** handles the
catalog query and bounded cached download; no new SDK dependency or fork is
needed. Omoba adds the static attachment validator.

## Historical operator-import pilot

The following CLI/sidecar route is retained for controlled local imports and its
original verification. It is not the current artist-to-game onboarding route:

```sh
cargo run -p omoba-passport --bin weapon-import -- https://registry.ekza.io
```

The importer queries `/v2/avatars` using that exact profile, requires the
project's explicit approval and free access, verifies declared size and SHA-256
through `AssetCache`, then validates the GLB and its grip. The current SDK calls
its generic rendition/catalog DTOs “avatars”; handheld files are distinguished
by profile, never passed through the humanoid avatar validator or avatar picker.
IDs pin source identity plus content hash. Verified files go into
`weapons/imported/<sha256>.glb`; metadata goes into `weapons/ekza-manifest.json`.
The shipped manifest is never overwritten. Import failure preserves the previous
manifest. Copy the same assets to consumers and restart both server and client:

```sh
OMOBA_WEAPON_MANIFEST=/absolute/assets/weapons/ekza-manifest.json
OMOBA_ASSET_DIR=/absolute/assets
```

At startup, imported files are checked again against their hash and embedded
grip; invalid entries remain unavailable. Clients cannot supply model URLs in
combat packets. This original pilot verified desktop packaging only. The current dynamic
asset-source path described above supplies runtime installs; physical mobile
acceptance remains pending.

## Historical pilot scope and remaining work

The local SDK integration is testable with an HTTP catalog fixture and real
GLB downloads. At the original pilot stage, the profile was not registered in the hosted
Registry. The subsequent October 1–2 typed rollout registered it and published
the Forge Hammer; that does not mean every shipped prop is published or that
Ekza Space itself supports handhelds.
The `weapon-build --source FILE --output-dir DIRECTORY` adapter implements the
existing registry RenditionWorker result contract and reuses the Rust validator.
It preserves bytes; it does not generate a humanoid skeleton or animation. The
registry's current `none` builder accepts VRM only, so register the supplied
`omoba-handheld-v1` builder when enabling this profile.
Studio publication and separate game approval remain necessary for each
public rendition. See the newer lifecycle evidence for the hosted pilot;
current production availability must be checked separately.

This pilot includes free built-ins and approved free imports. Paid weapon
ownership and storefront checkout remain future work; the current game already
has collection/draft equipment UI. The importer deliberately excludes `owned`
entries. Connect that ownership
check to `HandheldSelection::Item` before adding paid props. Do not repurpose
combat gold/stat items as cosmetic ownership.

Focused native check (English 1280×720, one class, two avatars):

```sh
python3 scripts/capture_standard_skills.py --handhelds --client-bin /path/to/client --server-bin /path/to/server --assets client/assets --output /tmp/handheld-pilot
```

The harness captures idle/run, accepted basic attacks with three props, an
Agnes→Orion avatar replacement and empty hands, with a second independently
bound remote Warrior. JSON records actual scene load status, owner, hand and
attachment positions. This is a scripted desktop check, not manual multiplayer
or physical mobile certification.

To exercise the entire builder → catalog → SDK download → equip path:

```sh
python3 scripts/capture_handheld_sdk.py --client-bin /path/to/client --server-bin /path/to/server --import-bin /path/to/weapon-import --builder-bin /path/to/weapon-build --assets client/assets --output /tmp/handheld-sdk-pilot
```

This creates a separate asset directory and a loopback catalog with one approved
free rendition. The HTTP server shuts down before the native match starts; the
game uses the verified installed file. Assertions check actual avatar GLTF
paths and runtime binding, separate owners, attachment transforms, scene loads,
accepted attack sequences, animations and removal. The closest supported
gameplay zoom (55%) is used to inspect the props.

The pilot also fixes a sandbox avatar replacement race: reconstruction now
happens in the same update as the appearance snapshot. The legacy spawn fallback
can no longer put the old model back at a team spawn between packets.
See [verification results](progress/2026-10-01-handheld-pilot.md).
