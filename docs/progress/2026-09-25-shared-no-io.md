# 2026-09-25 — Shared model free of I/O (roadmap step 13, slices 13a-13e)

## Goal
Slices 13a-13e from [plans/steps-11-13.md](../plans/steps-11-13.md), "Step
13", together with the cheap slice of O30 (no roster lookup relative to the
working directory, the roster source printed at startup) and the load-time
validation of O5 from [ARCHITECTURE_REPORT.md](../ARCHITECTURE_REPORT.md).
After this PR `shared` reads no environment variable and no file at runtime,
embeds nothing from `client/`, and depends on `serde` and `serde_json` only
(`cargo tree -p shared` lists no `ekza-bevy-sdk`). 13f (a registry owned by
the server runtime) is optional and not done.

No wire change: `GOLDEN_JOIN`, `GOLDEN_SNAPSHOT` and the other golden strings
are unchanged, and so are the snapshot byte pins.

## 13a: sprite presentation to the client
- New `client/src/sprite_roster.rs`: `SpriteSheetKind`,
  `SpriteAnimationPlayback`, `SpriteAnimationDefinition`,
  `SpriteAnimationSet`, `SpriteCharacterDefinition`, the embedded
  `client/assets/sprites/manifest.json`, `sprite_character_roster`,
  `sprite_character_definition`, `sprite_character_render_definition` and
  the render fallback resolver (moved verbatim; the `include_str!` path is
  now `../assets/sprites/manifest.json`, the same file).
- `shared` keeps `SPRITE_CHARACTER_IDS`, `DEFAULT_SPRITE_CHARACTER_ID` and
  `normalize_sprite_character_id`, now a search over the id list. The result
  is identical for every input because the manifest lists exactly these ids
  in this order: the client test
  `sprite_roster::tests::sprite_manifest_ids_equal_the_shared_frozen_list`
  pins that (it is the manifest half of the old shared test), and the shared
  test `sprite_ids_normalize_to_the_frozen_list_with_safe_fallback` keeps the
  normalization half.
- Call sites: `sprite.rs` (imports), `team.rs`, `career.rs`, `minimap.rs`,
  `social.rs` read `crate::sprite_roster::…`. The server's two
  normalization sites (`session.rs`, `prematch.rs`) are unchanged.

## 13b: asset root, avatar roster and registry to passport
- New `passport/src/assets.rs` (`client_asset_root`, moved verbatim with the
  Android branch; the development fallback is now passport's
  `CARGO_MANIFEST_DIR` parent, the same workspace `client/assets`).
- New `passport/src/avatars.rs`: `AvatarDefinition`, the manifest schema,
  the embedded fallback (same file bytes), `avatar_roster`,
  `avatar_definition`, `normalize_avatar_slug`, `register_store_avatar`,
  `store_avatars` and the `STORE_AVATARS` global, same names and bodies.
  Phase 1 of the plan: the global stays process-wide because the admission
  threads register off the main thread.
- Pure path rewrite (`shared::x` → `omoba_passport::avatars::x` /
  `omoba_passport::assets::client_asset_root`, inside passport
  `crate::avatars::x`). Rewritten references per crate: client 57 (23
  files, including the `use` lines in `team.rs` and
  `frontend/collection.rs`), server 24 (10 files), passport 10 (6 files,
  including `bin/passport-import.rs` and `tests/*_e2e.rs`), harness 3.
- The harness reads `client/assets/avatars/manifest.json` as a
  `serde_json::Value` (`harness::roster_avatar_slugs`,
  `longest_roster_avatar_slug`), so it stays black-box and links no roster
  loader; `framed_snapshots.rs` and `udp_datagrams.rs` use it.
- Join admission is untouched apart from the paths; the existing
  `passport_admission` tests pass unchanged.

## 13c: roster source at the binary boundary
- `RosterSource { manifest_override, asset_root }::from_env()` reads
  `OMOBA_AVATAR_MANIFEST` and `client_asset_root()`. Server `runtime::run`
  and client `main` call it once, pass it to `init_avatar_roster`, and print
  `LoadedRoster::summary()`, for example
  `Avatar roster: 15 avatars from /…/client/assets/avatars/manifest.json`
  or `… from embedded manifest`. The client reuses the source's asset root
  for `AssetPlugin`. `avatar_roster()` still initialises lazily from the
  environment when nothing called `init_avatar_roster` (unit tests, tools).
- `load_roster(candidates, read)` is pure apart from the reader and returns
  the avatars, the origin and the warnings; `init_avatar_roster` prints the
  warnings. Tests call it with an in-memory map, no environment mutation.
- Candidates are now the override, then `<asset root>/avatars/manifest.json`,
  then the embedded copy. The three working-directory candidates
  (`client/assets/…`, `assets/…`, `../client/assets/…`) are removed. Checked
  first: every launcher sets `OMOBA_ASSET_DIR` or runs a checkout binary
  (`package_native.py` launch scripts, `beta_launcher.py`, the `capture_*`
  scripts, `combat_test.py`, `smoke_native_package.py`,
  `verify_beta_match.py` with an isolated working directory),
  `check_passport_admission.py` sets `OMOBA_AVATAR_MANIFEST`, the Makefile
  uses `cargo run` or those scripts, and the iOS bundle places `assets/`
  next to the executable. In each case the asset root already found the
  manifest before the working-directory candidates were reached, so nothing
  relied on them. On Android the manifest sits inside the APK, where neither
  the asset-root path nor the working-directory paths are readable files,
  so the embedded copy is used, as before.
- O5: an entry whose slug looks protected (`ekza-…`) or that carries a
  passport boundary must satisfy the rule `register_store_avatar` applies
  (valid boundary, slug equal to `protected_slug` of it); otherwise it is
  skipped with a warning. Before, a manifest entry `ekza-<hash>` without a
  boundary was admitted as a free cosmetic. The committed manifest and the
  manifests `passport-import` and `check_passport_admission.py` write
  already follow the rule. Pinned by
  `avatars::tests::roster_entries_must_follow_the_store_avatar_rule`.
  The arena-sync half of O5 (checking the chain slug before `fs::write`) is
  not part of this step.

## 13d: `CharacterChoice` owned by shared
- `shared::wire::CharacterChoice` is now a shared enum with the SDK's
  derives, variants, `#[serde(rename_all = "snake_case")]`, `Default`
  (`Ipfs`), `ALL`, `as_str` and `slug` (no `other` variant: an unknown
  character still fails the packet, as before). `prematch.rs` uses it.
- The client re-exports it as `crate::team::CharacterChoice` and converts
  with `world::sdk_character` where it calls the SDK model catalogue
  (`handles_for`, `label_for`; the Android stub takes the SDK type too).
  `model_scale_key` only needs `slug()` and needs no conversion.
- `CharacterChoice` is listed in `shared/src/protocol/wire_enums.rs`
  (`Strict [Ipfs, Toka, Wang, Cube, Paco]`, added on `main` by #47, which
  had excluded it as an SDK type); the stale `SpriteSheetKind` and
  `SpriteAnimationPlayback` entries left `NOT_UDP_WIRE` with 13a.
- New tests: shared `character_choice_wire_ids_are_the_snake_case_slugs`,
  client `world::tests::sdk_character_keeps_every_variant_and_wire_id`
  (same variants in the same order, same `slug`, `as_str` and JSON).

## 13e: the SDK leaves shared
- `Entitlements::grant_verified_avatar(expected, consumed)` became
  `omoba_passport::entitlements::grant_verified_avatar(&mut entitlements,
  expected, consumed)`: it validates the consumed ticket against the exact
  expected avatar and rendition and then calls the new
  `Entitlements::grant_verified_avatar_id`, which still checks the id is a
  canonical avatar identity and keeps the bound. Nothing outside the tests
  called the grant.
- `validate_avatar_id` (with the base58 and UUID checks) is copied into
  `shared/src/social.rs` for the reaction catalog's `VerifiedAvatar` packs.
- `ekza-bevy-sdk` is removed from `shared/Cargo.toml`; career-store,
  account-api and the harness no longer compile it through shared.

## Merge with `main` (#44-#47)
- `client/src/career.rs`, `client/src/passport.rs`: #46 added
  `avatar_portrait_path` and `thumbnail_asset_path_in` (store portraits);
  kept, with `shared::AvatarDefinition`/`avatar_definition`/`avatar_roster`
  rewritten to `omoba_passport::avatars::…` (also in the new portrait test).
- `harness/src/server.rs`: both sides kept (#44's stale-binary warning and
  log helpers, this branch's `roster_avatar_slugs`).
- `shared/Cargo.toml`: the SDK stays removed.
- `docs/ARCHITECTURE.md` crate map: this branch's `shared` and `passport`
  roles with #44's dependency columns.
- `client/src/net/apply.rs`: #46's new test built a `DraftPlayer` with
  `ekza_bevy_sdk::EkzaCharacter::Ipfs`; now `shared::wire::CharacterChoice::Ipfs`.

## Tests
- shared 99 → 92 (13a −2, 13b −5, 13e −1 moved to passport, 13d +1).
- client lib 567 → 571 (13a +3, 13d +1).
- passport lib 18 → 26 (avatar roster, registry, loader and O5 tests,
  the moved grant test).
- server 297 (+3 ignored), harness 22 unit + 24 black-box, Python script
  tests 124 (1 skipped): unchanged.

## Gate
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: clean.
- `cargo clippy -p client --lib --no-deps --no-default-features -- -D warnings`: clean.
- `cargo test --workspace --locked --exclude harness`: all passed.
- `cargo build -p server && cargo test --locked -p harness -- --test-threads=1`: all passed.
- `python3 -m unittest discover -s scripts -p 'test_*.py'`: 124 OK (1 skipped).
- `cargo tree -p shared | grep -c ekza`: 0 (`shared` → serde, serde_json).
- Server startup from the checkout prints
  `Avatar roster: 15 avatars from …/client/assets/avatars/manifest.json`;
  with `OMOBA_ASSET_DIR=/nonexistent`,
  `Avatar roster: 15 avatars from embedded manifest`.
