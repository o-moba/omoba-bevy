# Community sculptures: game runtime

The free pilot has one code-owned slot, `opposite-dragon`, beside the Wendigo
pit. The Wendigo and dragon remain gameplay objectives. This feature adds no
collider, target, combat component, navigation obstacle or network packet field.

Registry owns publication, project support, placement approval, scheduling and
revocation. OMOBA reads `/v1/projects/omoba/placements/active` from its selected
connection profile's Registry. It never trusts a player's model URL or payment
claim. The related Web `/sponsor` and Studio review flow are separate feature PRs.
A future payment provider must confirm transactions before granting paid
eligibility; this pilot has no operational crypto checkout.

## Authorization and failure behavior

The feed contains at most one approved `map_prop` placement for the fixed slot,
with exact asset/revision identity, SHA-256, byte count and UTC start/expiry.
The Registry endpoint's contract pins the `map-prop-glb-v1` desktop profile.
All URLs must equal that Registry's
`/v1/studio/assets/<sha256>/rendition`, without query parameters or redirects.
HTTPS is required except explicit loopback HTTP for local development.

A background read runs at most once every 30 seconds. Each request has a 3-second
connect and 12-second total timeout, and manifests are limited to 32 KiB. The
remaining lifetime is calculated from Registry `serverNow` and anchored to the
monotonic instant before the request starts. The local wall clock cannot extend
it. Network failure can retain the previous approved model for at most 90 seconds
from that successful request start, and never beyond placement expiry. A healthy
empty or malformed feed removes it immediately. Revocation therefore appears
within one refresh cycle under healthy conditions and is bounded by the stale
limit during an outage. A monotonic Registry-time watermark survives expiry and empty revocation feeds;
replayed or regressing snapshots cannot restart a grant. A backwards Registry
clock fails closed until it catches up. Every process startup starts with no authorization.

Downloads run outside Bevy's update thread, have an 8 MiB hard limit, and require
an exact size and SHA-256 match. Cached bytes are rechecked before loading, while
authorization is never restored from disk. A completed download is ignored if the
current authorization changed or expired. Models use immutable hash filenames in
the existing profile-isolated `ekza-store/map-props` cache. A changed placement
replaces the previous scene root; expiry/revocation despawns its descendants and
drops scene/animation handles. No downloaded file enters the application bundle.

## Clickable creator information

Click or tap the actual rendered sculpture to read the approved title, credit and
optional description. The ray tests the nearest visible geometry, so empty ground
or an occluding tree cannot activate a hidden sculpture. The card uses the common
modal stack, close control, Escape/back signal and wheel/PageUp/PageDown/touch
scrolling. The creator text is literal: no HTML, Markdown, clickable URLs or
tracking. Every opening, closing and touch-drag gesture remains blocked from
movement, combat and camera input, including the release/closing frame.

`description` is immutable with the placement approval. Omission means an empty
string; explicit null and non-strings are invalid. The limit is 1,000 Unicode
scalar values, with LF line breaks allowed and other C0/C1/DEL controls rejected.
Accepted text is not trimmed or normalized. Rust strings reject unpaired
surrogates during JSON parsing. A card is bound to the exact authorized placement;
expiry, stale/invalid authorization, revocation, visual-mode removal or replacement
closes it. A late download or replay cannot recreate its authorization.

The GLB's permitted rotation animation continues behind the card. GLB is the
only model format for this profile; it does not imply arbitrary GLB compatibility.
The budgets below remain identical for animated and static sculptures.

## Conservative model profile

V1 accepts one GLB 2.0 scene with embedded BIN data, at most 64 nodes, 16 meshes,
16 solid PBR materials, 100,000 vertices and 50,000 triangles including instances.
Real position/index/keyframe bytes are checked before Bevy loads the model;
accessor bounds are not trusted. The first pilot uses material or vertex colors:
textures, images, skins, morph targets, cameras, lights and glTF extensions are not
accepted. Nodes use finite TRS transforms, not matrices, with a valid acyclic tree.

One optional animation may rotate nodes with LINEAR/STEP interpolation, at most
64 channels, 1,024 keys per channel and a 30-second duration. Translation/scale
animation is excluded so it cannot escape the slot. A conservative hierarchy
radius encloses every permitted rotation pose; the root is fitted inside a fixed
four-metre envelope and grounded from actual static vertices. Model size does not
follow the player's hero-size preference.

The original lucky cat is `assets-src/community/lucky-cat/lucky-cat.glb`, beside
its editable source and provenance. Its `LuckyPawWave` node is driven by the
embedded `Wave` rotation clip. This source/test fixture is not packaged with the
client. The normal client and opt-in local art scene require an active Registry
placement and download its approved bytes just like any other sculpture.

## Focused local verification

Use the existing shared Cargo target cache, not a worktree-specific target:

```sh
export CARGO_TARGET_DIR=/Users/wotori/git/ekza/omoba-bevy/target
cargo test -p omoba-passport sponsorship --lib
cargo test -p shared sponsorship --lib
cargo test -p client --lib sponsorship
cargo check -p client
```

The passport tests run a real loopback HTTP server and exercise manifest parsing,
verified atomic installation, cached-byte validation, redirect/hash rejection,
expiry, stale data, revocation, late downloads and malformed models. They do not
prove hosted deployment or payment processing.

For a single 1280 × 720 English native capture, run a local Registry integration
fixture that serves an approved cat via the normal endpoints, then start the
client with its ordinary environment override and isolated settings directory:

```sh
GAME_SERVER_ADDR=127.0.0.1:41009 \
OMOBA_REGISTRY_URL=http://127.0.0.1:8769 \
OMOBA_CLIENT_CONFIG_DIR=/tmp/omoba-sculpture-qa \
OMOBA_ASSET_DIR="$PWD/client/assets" \
OMOBA_LANGUAGE=en OMOBA_PLAYER_VISUAL_MODE=models3d \
OMOBA_SPONSOR_QA_DIR="$PWD/.agent/tasks/community-sculptures/raw/native" \
"$CARGO_TARGET_DIR/debug/client"
```

The opt-in entry starts an isolated art app with the actual Verdant map and
production placement plugin. It opens no UDP gameplay session, positions the
camera near the fixed slot and waits for the ordinary runtime to load an authorized
cat with the fixture’s 1,000-scalar multiline description. It records five
screenshots (cat, wave, card, wave behind the card, and final scrolled text),
nonblack readbacks, the placement/hash identity, slot position and paw motion.
Low-level pointer and keyboard events exercise actual triangle selection, the
visible close control and Escape through production input/modal systems.
`result.json` requires full-text scroll reachability, animation while reading and
zero input-policy leaks; it exits nonzero on timeout or a failed assertion. It does not
fabricate an active placement or bypass the downloader. The QA label identifies
local evidence. Other viewports and physical iOS/Android devices are not covered.

On this Mac, direct terminal launch initially produced black GPU readbacks; those
runs were rejected. The accepted local capture used a temporary `.app` wrapper
launched through the native app UI, with the same binary, endpoint overrides and
isolated art entry. The task evidence retains its launcher, logs, two rendered
frames and measured result. No machine settings were changed. If a capture stays
black, open the native application window before accepting evidence; `pass: true`
requires five nonblack readbacks and animation motion, plus visual inspection.
