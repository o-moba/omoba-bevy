# SDK Twenty developer collection

Twenty new Open Source Avatars models, selected from the owner's local archive. This is a persistent **local developer catalog**, consumed by the real Ekza SDK and the ordinary game asset loader. It is not a public Ekza Space publication or an account-library grant.

`selection.json` pins source IDs, author/collection, source URLs, VRM hashes and portrait hashes. The models are deliberately absent from `client/assets/avatars/manifest.json`; no avatar binaries are committed here. Original archive files remain unchanged.

The selection covers EYEWizard, EYESummoner, CosmicBot, BotBunny, SnakeBot, MushroomFairy, Buffedwolf, BluePixie, YetiDude, LadyFawn, DreamFighter, DreamEater, LilShark, Kiba, FireEye, GoatGhost, AstroNacho, EvilPendra, CursedAmy and CoolThief.

## Build and play

Run from the repository root. Requirements: Python 3, existing project build dependencies and `assimp` on PATH. Use the shared Cargo cache, not a worktree-local cache.

```sh
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target"
cargo build -p client -p server -p omoba-passport --bin client --bin server --bin avatar-pack-install
python3 scripts/sdk_avatar_pack.py build \
  --archive /Users/wotori/Yandex.Disk.localized/models/opensourceavatars \
  --output .agent/tasks/SDK-AVATAR20-2026-10-01/pack-v1
python3 scripts/sdk_avatar_pack.py launch \
  --pack .agent/tasks/SDK-AVATAR20-2026-10-01/pack-v1 \
  --client-bin "$CARGO_TARGET_DIR/debug/client" \
  --server-bin "$CARGO_TARGET_DIR/debug/server"
```

The prepared `pack-v1` already exists in the development worktree: skip the build step there. Building refuses to overwrite an existing directory. For another build, choose a new output directory.

Open **Avatars → Studio**, preview and equip an avatar, then play against the local server. The launcher keeps the loopback catalog and server alive until the client exits. Model downloads are lazy; the isolated persistent player cache lives under `<pack>/player/config/ekza-store`. The source archive is no longer needed once the pack is built. Relaunch through this command to restore catalog access and server admission. It does not change the normal account profile or production registry configuration.

## Pipeline and verification

1. Verify pinned source bytes and the existing embedded-permission policy. All selected collection records and VRM metadata declare CC0; embedded violent/commercial flags explicitly allow use. Conflicting candidates are excluded. R3 metadata includes external `otherLicenseUrl` links; these were retained but could not be fetched during this run. Do not describe those external documents as independently verified.
2. Use `ekza_build_rendition.build` to produce `humanoid-glb-v1` renditions with retargeted `idle`, `walk`, `attack`, `cast`, `death` clips. Inspect every output with Assimp.
3. Serve a loopback-only v2 catalog with explicit free/project approval, content hashes, sizes and allowlisted files. Developer IDs are deterministic UUIDs in a separate local namespace.
4. `avatar-pack-install` fetches via the existing SDK, installs into an empty store, rechecks content hashes and validates runtime humanoid bones including both hands.
5. The capture runner exercises independent real-server admission for all 20, unknown-ID rejection and withdrawal on a fresh server, then ordinary native preview and representative gameplay. Withdrawal on an already-running server retains the existing five-minute catalog TTL.

```sh
python3 scripts/capture_sdk_avatar_pack.py \
  --pack .agent/tasks/SDK-AVATAR20-2026-10-01/pack-v1 \
  --output .agent/tasks/SDK-AVATAR20-2026-10-01/verified \
  --client-bin "$CARGO_TARGET_DIR/debug/client" \
  --server-bin "$CARGO_TARGET_DIR/debug/server" \
  --import-bin "$CARGO_TARGET_DIR/debug/avatar-pack-install"
```

Native verification needs an unlocked desktop and a `qa`-enabled client (the default). It targets English at 1280×720 only. It checks 20 previews and three running Warrior avatars with sword attachments; it does not certify every class/skill, finger posing, arbitrary accessories, mobile performance, paid ownership or public publishing. Check the task evidence for actual results, including any pending visual verification.

For SDK/server-only verification on a locked or headless host, append `--data-only`. Its report sets `data_pass: true` only after those checks pass and deliberately leaves overall `pass: false` / `visual_status: pending`.

For publication under a real Studio account and delivery to iOS/Android, follow [the production publication runbook](../../docs/ekza-production-avatars.md). It includes the current public API gap, attribution fields and separate game approval.
