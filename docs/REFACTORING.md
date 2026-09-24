# Refactoring tracker

This is the hand-over page for the architecture refactoring programme. Anyone
(a person or an agent) picking the work up should be able to continue from
this file alone: what is done, what is next, how each step is delivered, and
where the detailed plans live. Update it in the same pull request as the
step it describes.

Roadmap source: the "Roadmap" section of [ARCHITECTURE.md](ARCHITECTURE.md).
Per-step notes: [progress/](progress/) (one dated note per merged step).
Release notes: `## [Unreleased]` in the root `CHANGELOG.md`.

## Process rules (agreed with the maintainer)

1. Every refactoring step lands through a pull request into `main`; never push
   refactoring commits to `main` directly.
2. Open the PR, then merge it immediately (the maintainer reviews on `main`);
   `merge` or `squash`, whichever gives one readable commit per step.
3. Before opening a PR the full gate must be green locally:
   `make check` (= `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --no-deps -- -D warnings`,
   `cargo test --workspace --locked --exclude harness`, Python script tests)
   plus the black-box harness against the freshly built server:
   `cargo build -p server && cargo test --locked -p harness -- --test-threads=1`
   (about five minutes; it is the only check that exercises a real match end
   to end, so never skip it for a server or protocol change). Reference
   counts at the time of writing: server 278 (+3 ignored), shared 78, client
   lib 543, harness 22 unit + 24 black-box.
4. One step per branch, named `refactor/<topic>` (docs-only: `docs/<topic>`),
   cut from the current `origin/main`.
5. No wire-visible change unless the step says so explicitly; `shared/` stays
   untouched in server-only and client-only steps. Snapshot bytes are pinned by
   `server/src/tests/player_view.rs` and the golden JSON tests in
   `shared/src/protocol/wire.rs`.
6. After merging, look at the CI run on `main` (Actions tab) and treat a red
   run as work now, even if the local gate was green.
7. Each PR updates: the roadmap line in `ARCHITECTURE.md`, a `CHANGELOG.md`
   entry under `## [Unreleased]`, a `docs/progress/<date>-<topic>.md` note,
   and the table below.
8. Keep diffs mechanical where they are large (renames, moves); behaviour
   changes go in their own small PR with a test that pins them.

## Status

| Step | Scope | State | PRs |
| --- | --- | --- | --- |
| 1 | CI workflow, pinned toolchain (1.94.1), `make check` | done | (direct, before the PR rule) |
| 2 | One wire protocol in `shared::protocol::wire`, golden JSON tests, harness on shared types | done | #18 |
| 3 | Balance constants (`shared::hero_balance`) and facing convention (`shared::math`) | done | #19, #20 |
| 4 | Crate hygiene: `career-store` crate, `skills` crate removed | done (shared I/O isolation open, see step 13) | #21 |
| 5 | Server `GameWorld` + `TickCtx`, `main.rs` split into runtime/sim/snapshot/entities | done | #22 |
| 8 | Client `net.rs` split into `client/src/net/` modules | done (session events are step 15) | #23 |
| 9 | Client UI kit pilot: theme, gesture, `UiAction`, widgets, `TestId`; pause menu + practice + career migrated | pilot done, rest is step 9b | #24 |
| 6a | Server one tick loop, one mana regen, one projectile path, ECS mirror removed | done | #25 |
| 6b | `PlayerState` as a view (`owner_view`/`public_view`), `HeroTimers`, `Hero`/`HeroEconomy` structs | done | #26, #27 |
| 6c | `StatModifiers` + `hero_stats.rs`; non-owner redaction of private economy on the wire | in progress (branch `refactor/server-hero-stats`) | |
| 7 | Server `MatchRules` policy object; career, transport and clock behind traits | next | |
| 10 | Client domain module, combat/player split, render backends behind `run_if`, plugin groups, QA behind a cargo feature | pending | |
| 11 | One debug tooling family shared by Combat Test, practice and offline | pending | |
| 12 | Data-driven hero and item catalogs with validation tests | pending | |
| 13 | Roster/asset loading and SDK types out of the shared model | pending | |
| 14 | Server per-variant packet handlers, explicit imports instead of crate-root globs | pending | |
| 15 | Client session events and staged snapshot application | pending | |
| 9b | UI kit follow-ups: scroll unification, modal registry, frontend/social/supporter/sandbox screens, responsive layout, `TestId` in QA | pending (order in [ui-kit.md](ui-kit.md)) | |

Suggested order after 6c: 7, 14, 10, 15, 11, 12, 13, 9b (server first while
its structure is fresh, then the client). Each row is one to four PRs.

## How to continue

1. `git fetch origin main && git checkout -b refactor/<topic> origin/main`.
2. Read the roadmap line in `ARCHITECTURE.md`, the plan for the step below and
   the latest `docs/progress/` note that touched the same area.
3. If the step is large, first get a read-only plan (file/line inventory of
   readers and writers, ordered PR-sized slices, risks), then build one slice
   at a time; keep `cargo test -p server` (or `-p client --lib`) green after
   every slice.
4. Run the gate, commit with a message that names the step, push the branch,
   open the PR (body: summary, behaviour notes, test plan), merge it, update
   this table.

## Plans for the open steps

### 6c: StatModifiers and redaction (in progress)
- `server/src/hero_stats.rs`: `StatModifiers { damage_mult, attack_speed_mult, move_speed_mult, armor, resistance, base_max_hp, god_mode, infinite_hp, infinite_resource, no_cooldowns, unlock_all, bypass_vision, grant_xp, respawns }` with `Default` = normal play; the single formulas for combat bonuses, basic damage/cooldown, ability cooldown, skill recovery, move speed, `max_hp`, `max_mana` (moved out of `sandbox.rs`). `sandbox::apply_actor` converts `ActorConfig` into modifiers + loadout once; `sandbox.is_some()` gates become flags; `god_mode`/`speed_mult` fields fold into the modifiers.
- Redaction: `public_view` blanks `gold`, `earned_gold`, `inventory`, `item_bonuses`, `last_purchase`, `basic_attack_request_id`, `utility.last_request_id` (all `#[serde(default)]`, no protocol bump); `build_players_snapshot` uses `owner_view` for the recipient and `public_view` for everyone else; vision tests pin it. Scoreboard gold comes from `LiveScoreboard`, not `PlayerState`.

### 7: MatchRules, career trait, Transport/Clock
- `server/src/match_rules.rs`: one `MatchRules` value built from the mode (practice / development / release) that answers team assignment, bot filling, whether debug commands are accepted, whether career credit applies, respawn policy; replaces the scattered `mode ==` checks in `session.rs`, `bots.rs`, `runtime/dispatch.rs`, `career_runtime.rs`.
- `trait CareerBackend` (already partly there in `career_backend.rs`): make the runtime generic or dyn over it so tests use an in-memory backend; the Postgres one lives in `career-store`.
- `trait Transport { recv, send }` over the UDP socket and `trait Clock { now }` so `ServerRuntime` tests run without sockets and with a virtual clock (the sandbox already has one; unify).

### 14: Per-variant packet handlers
- Split `runtime/dispatch.rs::handle_packet_authorized` into one function per `ClientPacket` variant (`handle_move`, `handle_cast`, `handle_purchase`, …) in `runtime/handlers/`; replace `pub(crate) use x::*` globs in `server/src/main.rs` with explicit imports module by module.

### 10: Client domain and plugin groups
- `client/src/domain/` for client-side game model types now spread across `combat.rs`, `player.rs`, `targeting.rs`; split `combat.rs` (targeting, casting, feedback) and `player.rs` (prediction vs presentation); 2D/3D render backends registered behind `run_if(PlayerVisualMode == …)`; `PluginGroup`s (`NetPlugins`, `GameplayPlugins`, `PresentationPlugins`, `UiPlugins`, `QaPlugins`); QA modules behind a `qa` cargo feature (default on for dev builds).

### 15: Client session events
- `SessionEvent` (Connected, Joined, Rejected, Disconnected, RoundReset) and `SnapshotApplied` messages emitted by `net/session.rs` and `net/apply.rs`; other modules subscribe instead of writing into `ClientSession`; `apply_server_snapshot` split into staged passes (players, structures, minions, neutrals, events).

### 11: One debug tooling family
- One `DebugCommand` family in `shared` covering Combat Test sandbox, practice (`PracticeCommand`) and offline; one tools UI page; the offline simulation implements the same commands so the pause-menu page works in all three modes.

### 12: Data-driven catalogs
- Hero classes/ability kits and items as JSON under `shared/assets/` (or `assets/config/`), loaded once into static tables with validation tests (every class has a kit, every item is recommended exactly once, costs within starter budget); `ItemId`/`HeroClass` stay enums for the wire.

### 13: Shared I/O isolation
- Move avatar/sprite roster manifests and env-var reads out of `shared` into `client` (and the SDK types into a small adapter crate or `passport`); `shared` keeps the model only.

### 9b: UI kit follow-ups
- Order and details in [ui-kit.md](ui-kit.md).
