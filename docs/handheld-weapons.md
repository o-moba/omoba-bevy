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
or game rebuild is not required for this route. Weapons in the player's own
Ekza library are listed first, see [Owned items](#owned-items).

The hosted free-weapon route and two desktop clients have recorded pilot
acceptance, not a fresh production availability check in this document. Mobile
runtime integration exists; physical iPhone/Android visual and performance
acceptance is still separate. Paid ownership/checkout remains future work.

Studio's weapon preparation (ekza-registry `90e7850`) accepts a static GLB
without grip metadata: the artist sets hand, position, XYZ rotation and uniform
scale on an exact-file avatar-hand preview, and Studio writes only
`asset.extras.ekza_handheld_v1` while preserving the binary chunk. A GLB that
already embeds the grip can still be uploaded unchanged. Which Studio revision
is hosted is a deployment fact; check it before relying on the editor there.
The export contract below is current. Priorities for making this authorable
without an engineer are in the [creator roadmap](plans/creator-loop-and-class-builder.md).
See [typed lifecycle and runtime ownership](progress/2026-10-01-asset-lifecycle.md).

## Owned items

Collection → Weapons and the draft equipment picker list **My Ekza library**
first: the weapons the connected Ekza account created, saved in Studio or
claimed for free on `app.omoba.io/market`. They come from the `assets` array of
`GET /v1/account/library`, through the SDK's
`AccountSession::assets(AssetKind::Weapon, weapon_store::selector())`. Only
free, approved, unambiguous `omoba / desktop / handheld-glb-v1` renditions
qualify, and each one is the same `StoreAsset` the public catalogue would give.

The order is:

1. My Ekza library, by name.
2. Included: class default, empty hands and the packaged props.
3. Ekza catalogue: the public `/v2/assets` list, minus every asset already in
   the library.

An asset is listed once, by asset id. A library revision newer than the cached
catalogue replaces the catalogue's entry instead of adding a second one.
Signed out, the library group is empty and the catalogue group looks the same
as before this change. Each Ekza entry shows its author and licence as
published: Collection prints them under every tile, the draft under the chosen
item. The registry's `license.attribution` line (the CC-BY credit) is shown for
the chosen item: in full in Collection, at most two lines in the draft. The
pinned SDK's `StoreAsset` has no attribution field, so
`passport/src/attribution.rs` reads it from the same documents: the library's
`assets` entries, and one more `/v2/assets` read after each successful
catalogue refresh. The catalogue's lines are saved as
`weapon-attributions.json` next to the offline catalogue. Studio avatars get
theirs from the `/v2/avatars` read (`avatar-attributions.json`) and show it in
the Collection detail under the licence.

- **Refresh.** While Collection or the draft is open, the account library is
  re-read about every 15 s (`poll_account`), so a claim made on the web appears
  without a restart. The screen is rebuilt only when the list changes.
  - An outage keeps the last list.
  - Revocation or sign-out empties it.
  - A library weapon that the cached catalogue does not list yet is still shown
    and installable. It also triggers a catalogue refresh (at most every 30 s).
- **Download.** Choosing an owned weapon that is not installed starts the usual
  verified download (approved sha256, size cap, handheld validator). The class
  default is shown until the download is done.
- **Empty or disconnected.** The group shows the Ekza account button and status
  line (with the approval code while pairing). It also shows a hint: "Get free
  items at app.omoba.io/market" when disconnected, "No weapons in your Ekza
  library yet" when connected with an empty library. The full draft layout
  relies on the avatar library's Connect Ekza button; the compact Weapons tab
  has its own.
- **Not an entitlement.** The library only decides what is listed. The server
  still admits a free weapon from its own registry read, whoever owns it. Paid
  items will need a server-verified entitlement from the same Ekza ledger (the
  Ekza umbrella `adr/0001-web2-first-asset-lifecycle.md`).
  The game links with the read-only `library` scope. Claiming happens on the
  web under `library-claim`, and both places show `account.username`.

The code is in `passport/src/weapon_store.rs` (`set_library`, the merge in
`State::entries`), `client/src/ekza_account.rs` (published on every library
read) and the shared `handheld_choices` in `client/src/frontend/collection.rs`,
which Collection and `draft.rs` both use. It needs an `ekza-bevy-sdk` revision
with `AccountSession::assets` (the library-assets entry in the SDK changelog);
the root `Cargo.toml` pins `d18ec72`.

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
Adventurer uses a right-hand thrust for contact attacks. Wildspark uses a burst-fire motion for the repeater and a heavy shot for launcher rounds; its named rotor and recoil slide are animated only after accepted actions. W/R temporarily select the launcher for class-default equipment and for the launcher variant of a [Wildspark set](#wildspark-set-repeater-and-launcher-in-one-glb). The default repeater uses `weapons/wildspark-repeater.glb`; Riftshot and explicit legacy repeater skins retain `wild-repeater.glb`. Pausing the sandbox freezes these parts and muzzle flashes use the real hand socket. An explicitly selected SDK item that is still downloading, has failed or is not yet in this peer's catalogue shows the class-default handheld for the current mode (for Wildspark the repeater/launcher pair), never empty hands; the item replaces it once installed. The new bow currently uses the existing ranged avatar
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

## Wildspark set: repeater and launcher in one GLB

A single approved weapon asset can carry both Wildspark guns. It is one
submission, approval and claim, travels as the same `HandheldSelection::Item`
and needs no protocol or server change. The art pipeline and recipe live in
[Wildspark skin sets](wildspark-skin-sets.md); the runtime contract is:

```text
scene root "wildspark-set"
 ├─ WildsparkSetRepeater   identity TRS
 │   ├─ ...receiver meshes
 │   └─ WildsparkRotor      pivot (0, 0.12, 0.25), rest rotation identity, spins about local Z
 └─ WildsparkSetLauncher   scale 0 in the file
     ├─ ...launcher meshes
     └─ WildsparkSlide      pivot (0, 0.13, 0.20), recoils along -Z
```

Coordinates use the canonical grip frame above (origin at the palm, +Y up the
frame, +Z towards the muzzle); both variants share one `ekza_handheld_v1` grip.
The usual handheld-glb-v1 limits apply to the whole file. The authored scale 0
keeps the launcher hidden in viewers that do not know this convention, which
also covers older game clients: they show the repeater in both modes. An
optional hint moves the muzzle flash:

```json
"omoba_wildspark_set_v1": {
  "repeater": "WildsparkSetRepeater",
  "launcher": "WildsparkSetLauncher",
  "muzzle": { "repeater": [0, 0.12, 0.735], "launcher": [0, 0.13, 0.81] }
}
```

It sits next to `ekza_handheld_v1` in `asset.extras`. The game identifies the
set by the two node names alone. Without the hint, or for a malformed tip (not
three finite numbers within ±2 m), the baseline tip of the shown gun is used;
the hint never rejects a model.

`client/src/held_weapons.rs` applies the set the same way for local and remote
actors, from replicated class, weapon mode and accepted actions:

- When a held model has both `WildsparkSetRepeater` and `WildsparkSetLauncher`
  glTF nodes, the variant for the owner's current mode is shown at scale 1 and
  the other is `Visibility::Hidden`. Q switches it in place without reloading
  the model; an accepted W/R draws the launcher for 1.1 s, like the class
  default. Owners of any other class always see the repeater variant.
- `WildsparkRotor` and `WildsparkSlide` move on any held model that has them,
  packaged or SDK, relative to the rest transform read when the scene spawned.
  Inside a set, a part belongs to the variant above it and moves only while
  that variant is shown: the rotor in repeater mode, the slide in rockets mode
  and during a W/R draw. The hidden variant keeps its last pose. Parts outside
  a set move in every mode. Root recoil and the Q lowering apply to the same
  models. An SDK model without these nodes stays static.
- SDK models without a hint flash at the baseline tip of the shown mode;
  packaged props other than the class-default Wildspark pair emit no muzzle
  flash.

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
