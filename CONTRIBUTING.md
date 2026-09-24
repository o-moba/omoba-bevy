# Contributing to Open Moba

Help us build the [mission](MISSION.md) through code, playtesting, documentation, art, accessibility or useful tools. Start with the current [features and limitations](docs/features.md) and existing issues; discuss large changes before doing substantial implementation work.

## Find your part of the world

| If you enjoy… | A concrete place to start |
| --- | --- |
| Drawing, modeling or animating | Build a character, prop, projectile or effect with documented source files and credits. Read [combat cosmetics](docs/combat-cosmetics.md) and [map customization](docs/map-customization.md). |
| Designing games and maps | Try [bot practice](docs/bot-practice-and-social.md), then propose a specific change to routes, camps, towers or [balance](docs/balance-tuning.md) with before/after observations. |
| Making music and sound | Add a cue or improve a mix using the [audio authoring guide](docs/game-audio.md). |
| Playing and testing | Play one match and submit a reproducible [bug report](docs/bug-report-template.md), especially for phone controls, readability and accessibility. |
| Translating or explaining | Improve an instruction, an error message or a player's first few minutes; include the language and device you tested. |
| Building games and tools | Explore the [Ekza Bevy SDK](https://github.com/ekza-space/ekza-bevy-sdk), the [Stellar TypeScript SDK](https://github.com/ekza-space/ekza-stellar-sdk), and this game's [Passport reference integration](docs/progress/2026-09-11-avatar-passport-roundtrip.md). Report missing contracts or contribute a focused example. Check each SDK's own licensing status. |

Start with a small [issue](https://github.com/o-moba/omoba-bevy/issues) or pull
request: what you want to create, who it helps and how someone can try it. A
sketch, source model, test recording or clear reproduction can be as useful as
code. You do not need a wallet or purchased avatar to contribute or play the
built-in roster. Bring a collaborator and credit everyone involved.

## Rights and licenses

By intentionally submitting a contribution for inclusion, you offer the rights you hold in it under the license applying to those files in [LICENSING.md](LICENSING.md): AGPL-3.0-only for original server source, MPL-2.0 for other original source, and CC-BY-4.0 for original standalone documentation and the identified Verdant visual assets. Existing asset-specific terms, including CC0, continue to apply. State the license and provenance of any newly introduced material clearly in your submission.

You retain your copyright. This project does not require copyright assignment or a blanket permission to relicense your contribution under proprietary terms. Confirm that you created the contribution or have sufficient permission to submit it, including any necessary employer permission. Disclose third-party material, its source, license and changes; preserve its notices. A submission cannot grant rights its author does not hold, and this policy does not retroactively secure permissions from past contributors.

AI-assisted work is welcome when you have reviewed it, verified the result and can explain its provenance and intended behavior. Do not submit material copied from proprietary games, unlicensed repositories or private data. Mark generated art and retain the available tool/source records without claiming rights the project does not have. Keep user avatars, likenesses and runtime downloads out of the repository unless their actual permission covers that contribution.

## Review expectations

Keep changes focused and explain the user-visible result. Include relevant verification and known limits; a mobile preview is not a physical-device test. Follow the existing Rust formatting and Conventional Commit style. Run `make check` before pushing: it is the same gate CI runs on every push and pull request (`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace --locked`, and the Python tooling tests). The toolchain is pinned in `rust-toolchain.toml`; bump it deliberately in its own change. For gameplay changes also run `make verify-gameplay` (the headless harness) or a multiplayer check. Documentation-only changes need link/content review rather than a full game build.

Wire types live only in `shared/src/protocol/wire.rs`; never copy them into the server, client or harness. Add a field there once (with a `serde(default)` when it is additive) and extend the golden test in that file.

Maintain API and license boundaries: shared reusable logic belongs in the MPL crates; moving AGPL-only server code into another directory does not relicense it. Do not add dependencies or change existing asset licenses silently.

Be respectful of players and contributors. The maintainers decide what enters the official repository and explain significant direction changes. Others may fork or disagree under the licenses; contributing does not require surrendering that freedom or ownership of your work.
