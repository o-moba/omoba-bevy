# Crate hygiene — 2026-09-24

Roadmap step 4 (`docs/ARCHITECTURE.md`).

- **`career-store` crate.** `career_store.rs`, `matchmaking.rs`, their test
  files and `migrations/postgres` moved out of the server package into
  `career-store` (`omoba-career-store`). It depends on `shared`, `sqlx` and
  `tokio`'s timer only. The server binary and `migrate-career` import it; the
  account API links it instead of the whole `server` package, which pulled in
  Bevy and the Ekza SDK for one type. `server/src/lib.rs` is gone: the server
  is a binary crate again.
- **Removed:** the `skills` crate (no dependents; `MAX_SKILL_RANK = 5` against
  the shared cap of 3), `shared::PlayerAbilitySnapshot` (no uses), 64 local
  `#[allow(clippy::too_many_arguments / type_complexity)]` attributes that the
  workspace lint policy already allows, and references to
  `docs/network-client-session.md`, which never existed.
- **Open:** the shared model still embeds `client/assets` manifests and reads
  `OMOBA_ASSET_DIR` / `OMOBA_AVATAR_MANIFEST`; the leaked store-avatar
  registry lives there too. Moving roster loading out of `shared` is tracked
  as its own step.

Verification: `make check` and `cargo test -p omoba-career-store -p omoba-account-api`.
