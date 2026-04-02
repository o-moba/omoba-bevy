//! In-match HUD: progression, local HP/mana, target summary, objective hint, and key hints.

use bevy::prelude::*;

use crate::combat::{CombatStats, LocalCastCooldown, TargetState};
use crate::input_bindings::{
    help_key_display, skill_keys_display, skill_slots_bracket_line, upgrade_key_display,
};
use crate::net::{
    GameState, GameStateSnapshot, NetworkStructure, PlayerProgression, StructureKind, TargetId,
    TargetKind,
};
use crate::player::Player;
use crate::team::Team;

pub struct MatchHudPlugin;

impl Plugin for MatchHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_match_hud)
            .add_systems(Update, update_match_hud);
    }
}

#[derive(Component)]
struct MatchHudProgressionText;

#[derive(Component)]
struct MatchHudStatusText;

const HUD_LEFT: f32 = 16.0;
/// Below the minimap (minimap top margin + outer size ~ 236px).
const HUD_TOP: f32 = 248.0;

fn setup_match_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(HUD_LEFT),
                top: Val::Px(HUD_TOP),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                max_width: Val::Px(420.0),
                ..default()
            },
            ZIndex(8),
            Name::new("MatchHudColumn"),
        ))
        .with_children(|col| {
            col.spawn((
                Text::new("Level --   XP --/--   Skill points --"),
                TextFont {
                    font_size: 22.0,
                    ..default()
                },
                TextColor::WHITE,
                MatchHudProgressionText,
            ));
            col.spawn((
                Text::new(""),
                TextFont {
                    font_size: 17.0,
                    ..default()
                },
                TextColor(Color::srgba(0.88, 0.90, 0.94, 1.0)),
                MatchHudStatusText,
            ));
        });
}

fn update_match_hud(
    game_state: Option<Res<GameStateSnapshot>>,
    player: Query<(&CombatStats, &PlayerProgression), With<Player>>,
    local_team: Query<&Team, With<Player>>,
    enemy_bases: Query<(&CombatStats, &Team, &StructureKind), With<NetworkStructure>>,
    cast_cd: Res<LocalCastCooldown>,
    target_state: Res<TargetState>,
    mut prog: Query<&mut Text, (With<MatchHudProgressionText>, Without<MatchHudStatusText>)>,
    mut status: Query<&mut Text, (With<MatchHudStatusText>, Without<MatchHudProgressionText>)>,
) {
    let Ok(mut prog_text) = prog.single_mut() else {
        return;
    };
    let Ok(mut status_text) = status.single_mut() else {
        return;
    };

    let Some((stats, progression)) = player.iter().next() else {
        prog_text.0 = "Level --   XP --/--   Skill points --".into();
        status_text.0.clear();
        return;
    };

    let running = game_state
        .as_ref()
        .is_some_and(|g| matches!(g.state, GameState::Running));

    let up = upgrade_key_display();
    if progression.next_level_xp == 0 {
        prog_text.0 = format!(
            "Level {}   XP MAX   Skill points {}   Upgrade: {} (when available)",
            progression.level.max(1),
            progression.skill_points,
            up
        );
    } else {
        let displayed_xp = progression.xp.min(progression.next_level_xp);
        prog_text.0 = format!(
            "Level {}   XP {}/{}   Skill points {}   Upgrade: {} (when available)",
            progression.level.max(1),
            displayed_xp,
            progression.next_level_xp,
            progression.skill_points,
            up
        );
    }

    if !running {
        status_text.0 = format!(
            "Press {} for controls help.\nSkills: {} — cast on target.",
            help_key_display(),
            skill_keys_display()
        );
        return;
    }

    let objective_line = enemy_base_objective_line(&local_team, &enemy_bases);
    status_text.0 = running_status_text(
        *stats,
        target_state.selected_target,
        &objective_line,
        cast_cd.remaining_secs,
    );
}

fn running_status_text(
    stats: CombatStats,
    selected_target: Option<TargetId>,
    objective_line: &str,
    cooldown_remaining_secs: f32,
) -> String {
    let slots_line = format!(
        "Skill slots: {} (same cast, shared CD)",
        skill_slots_bracket_line()
    );
    let cast_line = if cooldown_remaining_secs > 0.0 {
        format!("Spell cooldown: {:.1}s (shared)", cooldown_remaining_secs)
    } else {
        format!("Spell ready — {} (shared cooldown)", skill_keys_display())
    };
    let target_line = match selected_target {
        Some(t) => {
            let kind = match t.kind {
                TargetKind::Player => "Player",
                TargetKind::Minion => "Minion",
                TargetKind::Structure => "Structure",
                TargetKind::Neutral => "Neutral",
            };
            format!("Target: {kind} #{}", t.id)
        }
        None => {
            "Target: none — Tab (nearest foe), middle-click near foe, Backspace clear".to_string()
        }
    };
    let hp = stats.hp.max(0.0);
    let max_hp = stats.max_hp.max(1.0);
    let mana = stats.mana.max(0.0);
    let max_mana = stats.max_mana.max(1.0);
    format!(
        "HP {:.0}/{:.0}   Mana {:.0}/{:.0}\n\
{target_line}\n\
{objective_line}\n\
{slots_line}\n\
{cast_line}\n\
Keys: {} — cast on target   |   Upgrade {}   |   {} help",
        hp,
        max_hp,
        mana,
        max_mana,
        skill_keys_display(),
        upgrade_key_display(),
        help_key_display()
    )
}

fn enemy_base_objective_line(
    local_team: &Query<&Team, With<Player>>,
    structures: &Query<(&CombatStats, &Team, &StructureKind), With<NetworkStructure>>,
) -> String {
    let Ok(team) = local_team.single() else {
        return "Goal: destroy the enemy base tower.".to_string();
    };
    let mut hp_sum = 0.0f32;
    let mut max_sum = 0.0f32;
    let mut any = false;
    for (stats, st_team, kind) in structures.iter() {
        if *kind == StructureKind::BaseTower && *st_team != *team {
            hp_sum += stats.hp.max(0.0);
            max_sum += stats.max_hp.max(0.0);
            any = true;
        }
    }
    if any && max_sum > 0.0 {
        format!(
            "Goal: destroy enemy base — {:.0} / {:.0} HP remaining",
            hp_sum.min(max_sum),
            max_sum
        )
    } else {
        "Goal: destroy the enemy base tower.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_status_text_shows_resources_and_objective() {
        let text = running_status_text(
            CombatStats {
                hp: 75.0,
                max_hp: 100.0,
                mana: 40.0,
                max_mana: 100.0,
            },
            None,
            "Goal: destroy enemy base — 650 / 650 HP remaining",
            0.0,
        );
        assert!(text.contains("HP 75/100   Mana 40/100"));
        assert!(text.contains("Goal: destroy enemy base"));
        assert!(text.contains("Spell ready"));
    }

    #[test]
    fn running_status_text_shows_target_and_cooldown() {
        let text = running_status_text(
            CombatStats {
                hp: 90.0,
                max_hp: 120.0,
                mana: 12.0,
                max_mana: 60.0,
            },
            Some(TargetId {
                kind: TargetKind::Player,
                id: 42,
            }),
            "Goal: destroy enemy base",
            1.26,
        );
        assert!(text.contains("Target: Player #42"));
        assert!(text.contains("Spell cooldown: 1.3s"));
    }
}
