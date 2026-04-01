# Changelog

## Unreleased

### Added

- Jungle neutral camps: three server-simulated camp types (Skirmisher, Bruiser, Spitter) with distinct HP, damage, attack range, and kill rewards; placement mirrors client jungle layout (off-lane).
- Server-authoritative neutral AI (idle, proximity/damage aggro, chase, attack, leash reset, respawn), snapshot sync, and `TargetKind::Neutral` for player casts.
- Client rendering for neutrals (sphere mesh) and HP bars consistent with other units; TAB and middle-click target selection includes neutrals.
