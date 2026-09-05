//! In-match HUD: progression, local HP/mana, target summary, objective hint, and key hints.

use bevy::prelude::*;

use crate::combat::{CombatStats, TargetState};
use crate::input_bindings::{help_key_display, skill_keys_display, upgrade_key_display};
use crate::net::{
    GameState, GameStateSnapshot, NetworkHeroClass, NetworkStructure, PlayerProgression,
    StructureKind, TargetId, TargetKind, TeamBuffKind, TeamBuffState,
};
use crate::player::Player;
use crate::team::{Team, TeamSelection};
use shared::HeroClass;

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

/// Active boss team-buff indicator (hidden while no buff is active).
#[derive(Component)]
struct MatchHudBuffText;

const BUFF_TEXT_COLOR: Color = Color::srgba(1.0, 0.82, 0.35, 1.0);

/// Container for the HP/Mana bars; hidden until the match is running.
#[derive(Component)]
struct HudBarsRoot;

#[derive(Component)]
struct HpBarFill;

#[derive(Component)]
struct ManaBarFill;

const HUD_LEFT: f32 = 16.0;
/// Below the minimap (minimap top margin + outer size ~ 236px).
const HUD_TOP: f32 = 248.0;

const BAR_TRACK_WIDTH: f32 = 200.0;
const BAR_HEIGHT: f32 = 14.0;
const BAR_TRACK_COLOR: Color = Color::srgba(0.10, 0.11, 0.14, 0.92);
const MANA_BAR_COLOR: Color = Color::srgb(0.30, 0.55, 0.95);

/// HP bar tints green/amber/red so low health reads at a glance.
fn hp_bar_color(ratio: f32) -> Color {
    if ratio > 0.5 {
        Color::srgb(0.30, 0.78, 0.34)
    } else if ratio > 0.25 {
        Color::srgb(0.90, 0.74, 0.20)
    } else {
        Color::srgb(0.86, 0.26, 0.22)
    }
}

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
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(5.0),
                    ..default()
                },
                Visibility::Hidden,
                HudBarsRoot,
                Name::new("MatchHudBars"),
            ))
            .with_children(|bars| {
                spawn_stat_bar(bars, "HP", hp_bar_color(1.0), HpBarFill);
                spawn_stat_bar(bars, "Mana", MANA_BAR_COLOR, ManaBarFill);
            });
            col.spawn((
                Text::new(""),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(BUFF_TEXT_COLOR),
                MatchHudBuffText,
                Name::new("MatchHudBuffText"),
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

fn spawn_stat_bar<F: Component>(
    col: &mut ChildSpawnerCommands,
    label: &str,
    fill_color: Color,
    fill_marker: F,
) {
    col.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            ..default()
        },
        Name::new(format!("MatchHudBar-{label}")),
    ))
    .with_children(|row| {
        row.spawn((
            Text::new(label),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgba(0.85, 0.87, 0.92, 1.0)),
            Node {
                width: Val::Px(44.0),
                ..default()
            },
        ));
        row.spawn((
            Node {
                width: Val::Px(BAR_TRACK_WIDTH),
                height: Val::Px(BAR_HEIGHT),
                ..default()
            },
            BackgroundColor(BAR_TRACK_COLOR),
        ))
        .with_children(|track| {
            track.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(fill_color),
                fill_marker,
            ));
        });
    });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_match_hud(
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    player: Query<(&CombatStats, &PlayerProgression, Option<&NetworkHeroClass>), With<Player>>,
    local_team: Query<&Team, With<Player>>,
    enemy_bases: Query<
        (
            &CombatStats,
            &Team,
            &StructureKind,
            Option<&crate::net::NetworkStructureProtected>,
        ),
        With<NetworkStructure>,
    >,
    target_state: Res<TargetState>,
    mut prog: Query<
        &mut Text,
        (
            With<MatchHudProgressionText>,
            Without<MatchHudStatusText>,
            Without<MatchHudBuffText>,
        ),
    >,
    mut status: Query<
        &mut Text,
        (
            With<MatchHudStatusText>,
            Without<MatchHudProgressionText>,
            Without<MatchHudBuffText>,
        ),
    >,
    mut buff_text: Query<
        &mut Text,
        (
            With<MatchHudBuffText>,
            Without<MatchHudProgressionText>,
            Without<MatchHudStatusText>,
        ),
    >,
    mut bars_root: Query<&mut Visibility, With<HudBarsRoot>>,
    mut hp_fill: Query<(&mut Node, &mut BackgroundColor), (With<HpBarFill>, Without<ManaBarFill>)>,
    mut mana_fill: Query<
        (&mut Node, &mut BackgroundColor),
        (With<ManaBarFill>, Without<HpBarFill>),
    >,
) {
    let Ok(mut prog_text) = prog.single_mut() else {
        return;
    };
    let Ok(mut status_text) = status.single_mut() else {
        return;
    };

    let Some((stats, progression, replicated_class)) = player.iter().next() else {
        prog_text.0 = "Level --   XP --/--   Skill points --".into();
        status_text.0.clear();
        if let Ok(mut v) = bars_root.single_mut() {
            *v = Visibility::Hidden;
        }
        return;
    };
    let hero_class = replicated_class
        .map(|class| class.0)
        .unwrap_or(team_selection.hero_class);

    let running = game_state
        .as_ref()
        .is_some_and(|g| matches!(g.state, GameState::Running));

    update_stat_bars(
        running,
        *stats,
        &mut bars_root,
        &mut hp_fill,
        &mut mana_fill,
    );

    // Boss team-buff indicator: only the LOCAL team's active buffs, with
    // remaining seconds; empty (invisible) when nothing is active.
    if let Ok(mut buff) = buff_text.single_mut() {
        let next = if running {
            let buffs = game_state
                .as_ref()
                .map(|snapshot| snapshot.team_buffs.as_slice())
                .unwrap_or(&[]);
            local_team
                .single()
                .map(|team| team_buff_hud_text(buffs, *team))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if buff.0 != next {
            // Log transitions (not the per-second countdown) for evidence runs.
            if next.is_empty() {
                info!("[hud] team buff indicator cleared");
            } else if buff.0.is_empty() || buff.0.lines().count() != next.lines().count() {
                info!("[hud] team buff indicator: {}", next.replace('\n', " | "));
            }
            buff.0 = next;
        }
    }

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
            "Press {} for controls help.\nClass: {}   Skills: {} - cast on target.",
            help_key_display(),
            hero_class.display_name(),
            skill_keys_display()
        );
        return;
    }

    let objective_line = enemy_base_objective_line(&local_team, &enemy_bases);
    status_text.0 = running_status_text(
        *stats,
        hero_class,
        target_state.selected_target,
        &objective_line,
    );
}

fn update_stat_bars(
    running: bool,
    stats: CombatStats,
    bars_root: &mut Query<&mut Visibility, With<HudBarsRoot>>,
    hp_fill: &mut Query<(&mut Node, &mut BackgroundColor), (With<HpBarFill>, Without<ManaBarFill>)>,
    mana_fill: &mut Query<
        (&mut Node, &mut BackgroundColor),
        (With<ManaBarFill>, Without<HpBarFill>),
    >,
) {
    if let Ok(mut v) = bars_root.single_mut() {
        *v = if running {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    let hp_ratio = (stats.hp.max(0.0) / stats.max_hp.max(1.0)).clamp(0.0, 1.0);
    let mana_ratio = (stats.mana.max(0.0) / stats.max_mana.max(1.0)).clamp(0.0, 1.0);

    if let Ok((mut node, mut color)) = hp_fill.single_mut() {
        node.width = Val::Percent(hp_ratio * 100.0);
        *color = BackgroundColor(hp_bar_color(hp_ratio));
    }
    if let Ok((mut node, mut color)) = mana_fill.single_mut() {
        node.width = Val::Percent(mana_ratio * 100.0);
        *color = BackgroundColor(MANA_BAR_COLOR);
    }
}

/// One line per active buff of the LOCAL player's team, with the remaining
/// time in whole seconds. Effect numbers mirror `server/src/balance.rs`
/// (`BOTTOM_BOSS_BUFF_*` / `TOP_BOSS_BUFF_*`); an empty string hides the row.
fn team_buff_hud_text(buffs: &[TeamBuffState], local_team: Team) -> String {
    buffs
        .iter()
        .filter(|buff| buff.team == local_team)
        .map(|buff| {
            let secs = buff.remaining_secs.max(0.0).ceil() as u32;
            match buff.kind {
                TeamBuffKind::WendigoFavor => {
                    format!("Wendigo's Favor: +15% ability damage - {secs}s")
                }
                TeamBuffKind::MutatioMight => {
                    format!("Mutatio's Might: +25% ability damage, +2 HP/s - {secs}s")
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Short effect summary for an ability tooltip line (single effect per ability).
fn running_status_text(
    stats: CombatStats,
    hero_class: HeroClass,
    selected_target: Option<TargetId>,
    objective_line: &str,
) -> String {
    let target_line = match selected_target {
        Some(t) => {
            let kind = match t.kind {
                TargetKind::Player => "Player",
                TargetKind::Minion => "Minion",
                TargetKind::Structure => "Structure",
                TargetKind::Neutral => "Neutral",
            };
            format!("Target: enemy {kind} - Q attacks; approach happens automatically")
        }
        None => {
            "Target: none - click/tap a foe to attack, Tab selects, Backspace clears".to_string()
        }
    };
    let hp = stats.hp.max(0.0);
    let max_hp = stats.max_hp.max(1.0);
    let mana = stats.mana.max(0.0);
    let max_mana = stats.max_mana.max(1.0);
    format!(
        "HP {:.0}/{:.0}   Mana {:.0}/{:.0}   Class: {}\n\
{target_line}\n\
{objective_line}\n\
Keys: {} - cast   |   Upgrade {}   |   {} help",
        hp,
        max_hp,
        mana,
        max_mana,
        hero_class.display_name(),
        skill_keys_display(),
        upgrade_key_display(),
        help_key_display()
    )
}

fn enemy_base_objective_line(
    local_team: &Query<&Team, With<Player>>,
    structures: &Query<
        (
            &CombatStats,
            &Team,
            &StructureKind,
            Option<&crate::net::NetworkStructureProtected>,
        ),
        With<NetworkStructure>,
    >,
) -> String {
    let Ok(team) = local_team.single() else {
        return "Goal: destroy the enemy base tower.".to_string();
    };
    let mut hp_sum = 0.0f32;
    let mut max_sum = 0.0f32;
    let mut any = false;
    let mut protected = false;
    for (stats, st_team, kind, protection) in structures.iter() {
        if *kind == StructureKind::BaseTower && *st_team != *team {
            protected |= protection.is_some_and(|protection| protection.0);
            hp_sum += stats.hp.max(0.0);
            max_sum += stats.max_hp.max(0.0);
            any = true;
        }
    }
    if protected {
        "Goal: destroy an enemy lane tower - the enemy base is protected.".to_string()
    } else if any && max_sum > 0.0 {
        format!(
            "Goal: destroy enemy base - {:.0} / {:.0} HP remaining",
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
    fn running_status_text_shows_resources_objective_and_class_kit() {
        let text = running_status_text(
            CombatStats {
                hp: 75.0,
                max_hp: 100.0,
                mana: 40.0,
                max_mana: 100.0,
            },
            HeroClass::Mage,
            None,
            "Goal: destroy enemy base - 650 / 650 HP remaining",
        );
        assert!(text.contains("HP 75/100   Mana 40/100"));
        assert!(text.contains("Class: Mage"));
        assert!(text.contains("Goal: destroy enemy base"));
        assert!(text.contains("Keys: Q / W / E / R") || text.contains("Keys: Q"));
    }

    #[test]
    fn running_status_text_shows_target_and_per_slot_cooldown() {
        let text = running_status_text(
            CombatStats {
                hp: 90.0,
                max_hp: 120.0,
                mana: 12.0,
                max_mana: 60.0,
            },
            HeroClass::Cleric,
            Some(TargetId {
                kind: TargetKind::Player,
                id: 42,
            }),
            "Goal: destroy enemy base",
        );
        assert!(text.contains("Target: enemy Player"));
        assert!(!text.contains("#42"));
    }

    #[test]
    fn team_buff_hud_text_lists_local_team_buffs_with_remaining_time() {
        let buffs = vec![
            TeamBuffState {
                team: Team::Green,
                kind: TeamBuffKind::WendigoFavor,
                remaining_secs: 71.3,
            },
            TeamBuffState {
                team: Team::Blue,
                kind: TeamBuffKind::MutatioMight,
                remaining_secs: 45.0,
            },
        ];

        let green = team_buff_hud_text(&buffs, Team::Green);
        assert!(green.contains("Wendigo's Favor"));
        assert!(green.contains("+15% ability damage"));
        assert!(
            green.contains("72s"),
            "remaining time must round up: {green}"
        );
        assert!(
            !green.contains("Mutatio"),
            "enemy team's buff must not show: {green}"
        );

        let blue = team_buff_hud_text(&buffs, Team::Blue);
        assert!(blue.contains("Mutatio's Might"));
        assert!(blue.contains("+25% ability damage, +2 HP/s"));
        assert!(blue.contains("45s"));
    }

    #[test]
    fn team_buff_hud_text_is_empty_without_active_buffs() {
        assert_eq!(team_buff_hud_text(&[], Team::Green), "");
        let enemy_only = vec![TeamBuffState {
            team: Team::Blue,
            kind: TeamBuffKind::WendigoFavor,
            remaining_secs: 10.0,
        }];
        assert_eq!(team_buff_hud_text(&enemy_only, Team::Green), "");
    }

    #[test]
    fn team_buff_hud_text_stacks_both_buffs_on_separate_lines() {
        let buffs = vec![
            TeamBuffState {
                team: Team::Green,
                kind: TeamBuffKind::WendigoFavor,
                remaining_secs: 30.0,
            },
            TeamBuffState {
                team: Team::Green,
                kind: TeamBuffKind::MutatioMight,
                remaining_secs: 80.0,
            },
        ];
        let text = team_buff_hud_text(&buffs, Team::Green);
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("Wendigo's Favor") && text.contains("Mutatio's Might"));
    }
}
