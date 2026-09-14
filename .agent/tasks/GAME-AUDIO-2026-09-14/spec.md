# Game music and sound — GAME-AUDIO-2026-09-14

Initialized and frozen before implementation on 2026-09-14. Base: `747b991`.
User asks to commit/push current work and enrich the game with music and effects,
using Creative Commons or generated audio. Existing work is already on main;
unrelated cinematic files must remain untouched. Work in an isolated worktree.

- AC1: Bundle at least one suitable instrumental game-music loop and a coherent
  original/licensed sound palette for distinct combat styles, UI and major local
  match/player events. Track exact provenance and licenses. No unlicensed samples,
  ripped commercial game assets, remote runtime audio fetches or new production deps.
- AC2: Actual native game playback consumes authoritative combat/presentation and
  match state with round baselines, deduplication and bounded voice/rate limits.
  Nearby combat is audible, distant combat culled/attenuated. Repeated snapshots,
  reconnects and missing/late audio assets do not create replay storms or backlog.
  Music loops at a restrained level with smooth volume/state changes. Missing audio
  hardware or assets must not break gameplay. Honor focus and mobile constraints.
- AC3: Persistent master/music/effects/UI volume and mute controls are available in
  the settings UI on desktop and mobile, apply to active playback, clamp invalid
  values, and preserve existing preference migrations and input/modal safety.
- AC4: The sound palette is configured through stable cue IDs and safe packaged
  asset paths so class/skin audio can evolve without changing gameplay authority.
  Do not change networking, ranked progression, NFT ownership or production infra.
- AC5: Current-source client/workspace checks, relevant playback/deduplication/
  preference tests, strict Clippy and formatting pass. Verify actual decoder/sink
  startup and render audio settings in desktop/mobile previews. Record real output,
  asset measurements and an audio preview; distinguish scripted preview, decoded
  playback and subjective listening from physical-device or global-load claims.
- AC6: Update version/changelog/features, add authoring/run documentation and proof
  evidence, commit and push tested changes to main without losing unrelated files.

No physical Android/iOS acceptance, dynamic music composer, voice chat, narration,
audio marketplace, new production dependency or public deployment is implied.
Task runtime/config/cache and downloaded source archives are not Git artifacts.
