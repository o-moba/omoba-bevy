//! Presentation-only art mapping. Ability IDs remain owned by shared gameplay data.
use bevy::prelude::*;

pub(crate) const ATLAS_PATH: &str = "ui/skills/skills-atlas.png";
const ABILITIES: [&str; 16] = [
    "shield_bash",
    "battle_rally",
    "heroic_strike",
    "rampage",
    "arc_bolt",
    "mana_surge",
    "frost_lance",
    "pyroblast",
    "quick_shot",
    "field_dressing",
    "piercing_arrow",
    "longshot",
    "smite",
    "renew",
    "divine_favor",
    "guardians_blessing",
];

/// Pixel rectangles support any atlas resolution, including odd-sized source art.
pub(crate) fn icon_rect(ability: &str, size: Vec2) -> Option<Rect> {
    let index = ABILITIES.iter().position(|id| *id == ability)?;
    let cell = size / 4.0;
    let min = Vec2::new((index % 4) as f32, (index / 4) as f32) * cell;
    Some(Rect::from_corners(min, min + cell))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_gameplay_ability_has_a_distinct_art_cell() {
        let mut cells = Vec::new();
        for class in shared::HeroClass::ALL {
            for slot in shared::SkillSlot::ALL {
                let id = shared::ability_for_class_slot(class, slot).id;
                let rect = icon_rect(id, Vec2::splat(1254.0)).expect(id);
                assert!(!cells.contains(&rect));
                assert!(rect.max.cmple(Vec2::splat(1254.0)).all());
                cells.push(rect);
            }
        }
        assert_eq!(cells.len(), 16);
        assert!(icon_rect("community_future_ability", Vec2::ONE).is_none());
    }
}
