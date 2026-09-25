//! Client-only 2D sprite presentation roster: the sprite sheet schema and the
//! embedded `assets/sprites/manifest.json`. Shared code keeps only the frozen
//! wire ids ([`shared::SPRITE_CHARACTER_IDS`]) and their normalization.

use serde::Deserialize;
use shared::DEFAULT_SPRITE_CHARACTER_ID;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpriteSheetKind {
    Actions,
    #[default]
    #[serde(other)]
    Locomotion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpriteAnimationPlayback {
    Once,
    HoldLast,
    #[default]
    #[serde(other)]
    Loop,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpriteAnimationDefinition {
    pub start: usize,
    pub count: usize,
    pub fps: f32,
    #[serde(default)]
    pub sheet: SpriteSheetKind,
    /// Pinned by the manifest contract test; playback itself follows the
    /// animation state (`sprite.rs`), so only the tests read it.
    #[serde(default)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub playback: SpriteAnimationPlayback,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpriteAnimationSet {
    pub idle: SpriteAnimationDefinition,
    pub run: SpriteAnimationDefinition,
    #[serde(default)]
    pub attack: Option<SpriteAnimationDefinition>,
    #[serde(default)]
    pub cast: Option<SpriteAnimationDefinition>,
    #[serde(default)]
    pub hit: Option<SpriteAnimationDefinition>,
    #[serde(default)]
    pub death: Option<SpriteAnimationDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpriteCharacterDefinition {
    pub id: String,
    /// Draft identities may retain their wire id/portrait while rendering an
    /// explicitly declared, complete character. Never follow fallback chains.
    #[serde(default)]
    pub render_fallback: Option<String>,
    pub display_name: String,
    #[allow(dead_code)] // Required manifest metadata; read by the asset tooling, not at runtime.
    pub theme: String,
    #[allow(dead_code)] // Required manifest metadata; read by the asset tooling, not at runtime.
    pub palette: Vec<String>,
    pub sheet: String,
    #[serde(default)]
    pub action_sheet: Option<String>,
    #[allow(dead_code)] // Required manifest metadata; read by the asset tooling, not at runtime.
    pub license: String,
    #[allow(dead_code)] // Required manifest metadata; read by the asset tooling, not at runtime.
    pub provenance: String,
    pub frame_size: [u32; 2],
    pub columns: u32,
    pub rows: u32,
    #[serde(default)]
    pub action_columns: Option<u32>,
    #[serde(default)]
    pub action_rows: Option<u32>,
    pub pivot: [f32; 2],
    pub world_height: f32,
    pub animations: SpriteAnimationSet,
}

#[derive(Debug, Deserialize)]
struct SpriteManifest {
    schema_version: u32,
    characters: Vec<SpriteCharacterDefinition>,
}

const SPRITE_MANIFEST_JSON: &str = include_str!("../assets/sprites/manifest.json");

/// The sprite manifest is embedded so every client renders the same immutable
/// roster even when launched elsewhere. The server never reads it: wire ids are
/// normalized against [`shared::SPRITE_CHARACTER_IDS`], and a test below pins
/// the manifest ids to that list.
pub fn sprite_character_roster() -> &'static [SpriteCharacterDefinition] {
    static ROSTER: OnceLock<Vec<SpriteCharacterDefinition>> = OnceLock::new();
    ROSTER.get_or_init(
        || match serde_json::from_str::<SpriteManifest>(SPRITE_MANIFEST_JSON) {
            Ok(manifest) if matches!(manifest.schema_version, 1 | 2) => manifest.characters,
            Ok(manifest) => {
                eprintln!(
                    "sprite manifest schema {} is unsupported; sprite roster disabled",
                    manifest.schema_version
                );
                Vec::new()
            }
            Err(error) => {
                eprintln!("sprite manifest is invalid ({error}); sprite roster disabled");
                Vec::new()
            }
        },
    )
}

pub fn sprite_character_definition(id: &str) -> Option<&'static SpriteCharacterDefinition> {
    sprite_character_roster()
        .iter()
        .find(|character| character.id == id)
}

fn resolve_sprite_render_definition<'a>(
    roster: &'a [SpriteCharacterDefinition],
    raw: Option<&str>,
) -> Option<&'a SpriteCharacterDefinition> {
    let default = roster
        .iter()
        .find(|entry| entry.id == DEFAULT_SPRITE_CHARACTER_ID && entry.render_fallback.is_none())?;
    let requested = raw
        .map(str::trim)
        .and_then(|id| roster.iter().find(|entry| entry.id == id))
        .unwrap_or(default);
    match requested.render_fallback.as_deref() {
        None => Some(requested),
        Some(target) => Some(
            roster
                .iter()
                .find(|entry| entry.id == target && entry.render_fallback.is_none())
                .unwrap_or(default),
        ),
    }
}

/// Rendering only. Normalization/admission still preserve the requested stable id.
/// An unknown, self-referencing or recursive fallback safely resolves to the
/// complete default; an invalid default yields None instead of loading a draft.
pub fn sprite_character_render_definition(
    raw: Option<&str>,
) -> Option<&'static SpriteCharacterDefinition> {
    resolve_sprite_render_definition(sprite_character_roster(), raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{SPRITE_CHARACTER_IDS, normalize_sprite_character_id};

    #[test]
    fn sprite_manifest_ids_equal_the_shared_frozen_list() {
        let roster = sprite_character_roster();
        assert_eq!(roster.len(), SPRITE_CHARACTER_IDS.len());
        assert_eq!(
            roster
                .iter()
                .map(|character| character.id.as_str())
                .collect::<Vec<_>>(),
            SPRITE_CHARACTER_IDS
        );
        // Normalization over the shared id list resolves to a manifest entry.
        for id in SPRITE_CHARACTER_IDS {
            assert_eq!(
                sprite_character_definition(normalize_sprite_character_id(Some(id)))
                    .map(|character| character.id.as_str()),
                Some(id)
            );
        }
    }

    #[test]
    fn draft_sprite_preserves_identity_and_only_renders_a_complete_one_hop_target() {
        const DRAFT: &str = "orchard-comet-centaur";
        assert_eq!(normalize_sprite_character_id(Some(DRAFT)), DRAFT);
        assert_eq!(
            sprite_character_roster()
                .iter()
                .position(|entry| entry.id == DRAFT),
            Some(9)
        );
        assert_eq!(
            sprite_character_render_definition(Some(DRAFT)).unwrap().id,
            DEFAULT_SPRITE_CHARACTER_ID
        );
        let mut roster = sprite_character_roster().to_vec();
        for invalid in ["missing", "../mossback-teapot", DRAFT] {
            roster[9].render_fallback = Some(invalid.into());
            assert_eq!(
                resolve_sprite_render_definition(&roster, Some(DRAFT))
                    .unwrap()
                    .id,
                DEFAULT_SPRITE_CHARACTER_ID
            );
        }
        roster[9].render_fallback = Some(roster[1].id.clone());
        roster[1].render_fallback = Some(roster[2].id.clone());
        assert_eq!(
            resolve_sprite_render_definition(&roster, Some(DRAFT))
                .unwrap()
                .id,
            DEFAULT_SPRITE_CHARACTER_ID
        );
        roster[0].render_fallback = Some(DRAFT.into());
        assert!(resolve_sprite_render_definition(&roster, Some(DRAFT)).is_none());
    }

    #[test]
    fn every_new_sprite_has_the_frozen_six_state_contract() {
        let expected = [
            (
                "idle",
                0,
                6.0,
                SpriteSheetKind::Locomotion,
                SpriteAnimationPlayback::Loop,
            ),
            (
                "run",
                8,
                12.0,
                SpriteSheetKind::Locomotion,
                SpriteAnimationPlayback::Loop,
            ),
            (
                "attack",
                0,
                12.0,
                SpriteSheetKind::Actions,
                SpriteAnimationPlayback::Once,
            ),
            (
                "cast",
                8,
                10.0,
                SpriteSheetKind::Actions,
                SpriteAnimationPlayback::Once,
            ),
            (
                "hit",
                16,
                14.0,
                SpriteSheetKind::Actions,
                SpriteAnimationPlayback::Once,
            ),
            (
                "death",
                24,
                8.0,
                SpriteSheetKind::Actions,
                SpriteAnimationPlayback::HoldLast,
            ),
        ];
        for id in &SPRITE_CHARACTER_IDS[5..] {
            let character = sprite_character_definition(id).expect("new roster definition");
            assert_eq!(character.frame_size, [256, 256], "{id}");
            assert_eq!((character.columns, character.rows), (8, 2), "{id}");
            assert_eq!(
                (character.action_columns, character.action_rows),
                (Some(8), Some(4)),
                "{id}"
            );
            assert!(character.action_sheet.is_some(), "{id}");
            let animations = [
                &character.animations.idle,
                &character.animations.run,
                character.animations.attack.as_ref().expect("attack"),
                character.animations.cast.as_ref().expect("cast"),
                character.animations.hit.as_ref().expect("hit"),
                character.animations.death.as_ref().expect("death"),
            ];
            for ((name, start, fps, sheet, playback), animation) in
                expected.into_iter().zip(animations)
            {
                assert_eq!(animation.start, start, "{id} {name}");
                assert_eq!(animation.count, 8, "{id} {name}");
                assert_eq!(animation.fps, fps, "{id} {name}");
                assert_eq!(animation.sheet, sheet, "{id} {name}");
                assert_eq!(animation.playback, playback, "{id} {name}");
            }
        }
    }
}
