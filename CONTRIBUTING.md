# Contributing to Open Moba

Help us build the [mission](MISSION.md) through code, playtesting, documentation, art, accessibility or useful tools. Start with the current [features and limitations](docs/features.md) and existing issues; discuss large changes before doing substantial implementation work.

## Rights and licenses

By intentionally submitting a contribution for inclusion, you offer the rights you hold in it under the license applying to those files in [LICENSING.md](LICENSING.md): AGPL-3.0-only for original server source, MPL-2.0 for other original source, and CC-BY-4.0 for original standalone documentation and the identified Verdant visual assets. Existing asset-specific terms, including CC0, continue to apply. State the license and provenance of any newly introduced material clearly in your submission.

You retain your copyright. This project does not require copyright assignment or a blanket permission to relicense your contribution under proprietary terms. Confirm that you created the contribution or have sufficient permission to submit it, including any necessary employer permission. Disclose third-party material, its source, license and changes; preserve its notices. A submission cannot grant rights its author does not hold, and this policy does not retroactively secure permissions from past contributors.

AI-assisted work is welcome when you have reviewed it, verified the result and can explain its provenance and intended behavior. Do not submit material copied from proprietary games, unlicensed repositories or private data. Mark generated art and retain the available tool/source records without claiming rights the project does not have. Keep user avatars, likenesses and runtime downloads out of the repository unless their actual permission covers that contribution.

## Review expectations

Keep changes focused and explain the user-visible result. Include relevant verification and known limits; a mobile preview is not a physical-device test. Follow the existing Rust formatting and Conventional Commit style. For gameplay changes, run appropriate server/client or multiplayer checks. Documentation-only changes need link/content review rather than a full game build.

Maintain API and license boundaries: shared reusable logic belongs in the MPL crates; moving AGPL-only server code into another directory does not relicense it. Do not add dependencies or change existing asset licenses silently.

Be respectful of players and contributors. The maintainers decide what enters the official repository and explain significant direction changes. Others may fork or disagree under the licenses; contributing does not require surrendering that freedom or ownership of your work.
