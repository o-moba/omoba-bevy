# Release policy

This is the canonical policy for OMOBA releases, adopted for the first community beta, 0.43.0. Maintainers and coding agents must follow it when preparing, merging or distributing a release.

## Branches and versions

- `main` is the integration branch for the next version. It is not an installer and is not an instruction to deploy automatically.
- Use short-lived `feature/<topic>`, `fix/<topic>` or `refactor/<topic>` branches and focused pull requests. Merge after the applicable checks pass; remove merged branches. Do not create permanent `develop` or release branches without a concrete support need.
- The canonical application version is `[workspace.package].version` in `Cargo.toml`. Client and server artifacts for a coordinated beta use that version. SDK packages have their own versions and are pinned to an immutable revision.
- Use a patch increment for compatible fixes (0.43.0 → 0.43.1), a minor increment for a new feature/balance/protocol iteration (0.43.x → 0.44.0), and reserve 1.0.0 for an explicitly accepted stable release. During 0.x, a minor may break compatibility; say so in the notes.
- Create an annotated `v<version>` tag on the verified commit in `main`. Never move, replace or reuse a published tag or overwrite its published binaries. A corrected binary needs a new version/tag. Before publication, a draft can be repaired, but record candidate provenance and reverify.
- GitHub's **Pre-release** flag identifies community betas: the first is `v0.43.0`, titled `OMOBA 0.43.0 — Community Beta 1`. A beta need not encode a suffix in the application version. Keep the prerelease flag until stable release acceptance is explicit.
- iOS build numbers increase and the bundle ID stays stable. Android versionCode increases and the signing certificate stays stable; never generate a replacement key to repair a release. Keep signing material out of Git and public artifacts.

## Required release order

1. Freeze scope and source. Finish the changelog, tester instructions and known issues. Review contract changes in [release compatibility](release-compatibility.md): protocol, catalogue, geometry and gameplay are shared client/server obligations, independent of the engine version.
2. Run `make check` and applicable CI. Gameplay changes also require the headless harness and a live multiplayer check; storage changes require disposable PostgreSQL tests. For a mobile release, compile the mobile targets. Record physical-device verification separately from desktop previews and compile checks.
3. Build candidates from the frozen source. Record source commit, application version, compatibility manifest and artifact SHA-256. Never edit that checkout during a build. Keep packages as a draft until the endpoint is ready.
4. Stage the candidate server and verify its contract. Under the owner's deployment authorization, update lobby and workers together; preserve the previous release and durable data. Do not interrupt active human matches. A server deployment is not implied by merging a PR.
5. Probe the live lobby and allocated worker with the candidate manifest; test a deliberate mismatch, authenticated admission, gameplay and reconnection. Nonzero compatibility checks block distribution. Define supported older clients explicitly; a protocol change requires a coordinated upgrade.
6. Create/push the tag and build the draft GitHub release with the existing release workflow. Confirm all platform jobs, source commit, manifest, checksums and stable Android signing. CI never publishes a draft automatically.
7. Publish the prerelease only after the server and package gates pass and distribution is authorized. Upload iOS to TestFlight under explicit upload authorization; Apple processing, tester assignment and external review are distinct from a successful upload.
8. Record actual deployed server path/checksum, tag/commit, CI runs, download/TestFlight status, known limitations and rollback instructions. Clean temporary worktrees only after preserving evidence; keep the latest useful package and matching crash symbols.

If the tag has been frozen before server staging, keep the resulting release a draft until steps 4–5 pass. Do not issue test instructions for a known incompatible endpoint.

## Release record and rollback

Every release needs a short `docs/progress/` delivery record and local raw evidence in `.agent/tasks/<TASK_ID>/`. A record must distinguish **built**, **uploaded**, **processed/available**, **installed** and **tested**. Passing tests does not certify absence of bugs or server capacity.

A GitHub release includes player-facing changes, installation instructions, minimum supported server/client version, known issues, checksums and a bug-report link. Publish neither credentials nor development provisioning profiles. Keep exact source and licenses available with distributed packages.

Rollback is a versioned operation. Retain the previous server and state, but check its contract against clients already distributed before activating it. An incompatible rollback needs a matching client or separate endpoint; never disguise it as an automatic reconnect problem. Do not drop databases or erase match history to make a rollback pass.

## Working with testers

Send testers the release page, not a moving branch. Ask for version/build, device/OS, online/offline mode, server endpoint, reproduction steps, expected/actual behavior and an optional recording. Use [GitHub issues](https://github.com/o-moba/omoba-bevy/issues/new/choose); remove personal data and keys from logs. Triage blockers (crash, lost match, cannot connect) before visual polish. Announce breaking updates and any required reinstall/account recovery before asking users to upgrade.

The current small Beta host is limited to two rooms. This is an operating cap, not a measured capacity guarantee. Increase capacity only after observing memory, simulation latency and concurrent-room behavior.
