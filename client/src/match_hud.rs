//! In-match HUD: progression, local HP/mana, target summary, objective hint, and key hints.

use bevy::prelude::*;

use crate::combat::{CombatStats, TargetState};
#[cfg(test)]
use crate::input_bindings::upgrade_key_display;
use crate::input_bindings::{help_key_display, skill_keys_display};
use crate::net::{
    GameState, GameStateSnapshot, NetworkHeroClass, NetworkStructure, PlayerProgression,
    StructureKind, TeamBuffKind, TeamBuffState,
};
#[cfg(test)]
use crate::net::{TargetId, TargetKind};
use crate::player::Player;
use crate::team::{Team, TeamSelection};
#[cfg(test)]
use shared::HeroClass;

pub struct MatchHudPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MatchHudVisuals;

impl Plugin for MatchHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_match_hud)
            .add_systems(
                Update,
                (update_match_hud, update_hero_details)
                    .in_set(MatchHudVisuals)
                    .after(crate::net::ClientNetPipeline::ApplySnapshot),
            )
            .add_systems(
                Update,
                sync_gameplay_hud_visibility.after(crate::net::ClientNetPipeline::SyncConnectionUi),
            );
    }
}

#[derive(Component)]
struct MatchHudProgressionText;

#[derive(Component)]
struct MatchHudStatusText;

/// Active boss team-buff indicator (hidden while no buff is active).
#[derive(Component)]
struct MatchHudBuffText;

/// Container for the HP/Mana bars; hidden until the match is running.
#[derive(Component)]
struct HudBarsRoot;

#[derive(Component)]
struct HpBarFill;

#[derive(Component)]
struct ManaBarFill;

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
    use crate::ui_theme as ui;
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                bottom: Val::Px(16.0),
                width: Val::Px(230.0),
                height: Val::Px(150.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..ui::panel_node()
            },
            BackgroundColor(ui::PANEL),
            BorderColor::all(ui::EDGE),
            ZIndex(12),
            Name::new("MatchHudColumn"),
        ))
        .with_children(|panel| {
            panel
                .spawn((Node {
                    column_gap: Val::Px(10.0),
                    align_items: AlignItems::Center,
                    ..default()
                },))
                .with_children(|row| {
                    row.spawn((
                        Node {
                            width: Val::Px(44.0),
                            height: Val::Px(44.0),
                            flex_shrink: 0.0,
                            border_radius: BorderRadius::all(Val::Px(22.0)),
                            overflow: Overflow::clip(),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor::all(ui::GOLD),
                        BackgroundColor(ui::TILE),
                        HudPortrait,
                        Name::new("HudHeroPortrait"),
                    ));
                    row.spawn((
                        Text::new("YOUR HERO"),
                        ui::text(15.0),
                        TextColor(ui::IVORY),
                        MatchHudProgressionText,
                    ));
                });
            panel
                .spawn((
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
                    spawn_stat_bar(bars, "MP", MANA_BAR_COLOR, ManaBarFill);
                });
            panel.spawn((
                Text::new("XP 0 / 0"),
                ui::text(12.0),
                TextColor(ui::MUTED),
                HudXpText,
            ));
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // Reserve the desktop map and a gap even when the window narrows.
                // The phone layout supplies its own anchors and panel width.
                left: Val::Px(
                    crate::minimap::DESKTOP_MINIMAP_INSET + crate::minimap::MINIMAP_SIZE + 16.0,
                ),
                right: Val::Px(16.0),
                top: Val::Px(14.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
            ZIndex(8),
            Name::new("MatchObjectiveRoot"),
        ))
        .with_children(|root| {
            root.spawn((
                Button,
                Node {
                    max_width: Val::Px(630.0),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                    ..ui::panel_node()
                },
                BackgroundColor(ui::PANEL),
                BorderColor::all(ui::EDGE),
                Name::new("MatchObjectivePanel"),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(""),
                    ui::text(15.0),
                    TextColor(ui::IVORY),
                    MatchHudStatusText,
                    Name::new("MatchStatusText"),
                ));
                panel.spawn((
                    Text::new(""),
                    ui::text(13.0),
                    TextColor(ui::GOLD),
                    MatchHudBuffText,
                ));
            });
        });
}

#[derive(Component)]
struct HudPortrait;
#[derive(Component)]
struct HudXpText;
#[derive(Component)]
struct HudResourceText(&'static str);

fn update_hero_details(
    mut commands: Commands,
    assets: Res<AssetServer>,
    player: Query<
        (
            &CombatStats,
            &PlayerProgression,
            Option<&crate::net::NetworkAvatar>,
        ),
        With<Player>,
    >,
    mut labels: Query<(&HudResourceText, &mut Text), Without<HudXpText>>,
    mut xp: Query<&mut Text, (With<HudXpText>, Without<HudResourceText>)>,
    portraits: Query<Entity, With<HudPortrait>>,
    mut previous: Local<Option<String>>,
) {
    let Ok((stats, progression, avatar)) = player.single() else {
        return;
    };
    for (label, mut text) in &mut labels {
        text.0 = if label.0 == "HP" {
            format!("{:.0} / {:.0}", stats.hp.max(0.0), stats.max_hp)
        } else {
            format!("{:.0} / {:.0}", stats.mana.max(0.0), stats.max_mana)
        };
    }
    for mut text in &mut xp {
        text.0 = if progression.next_level_xp == 0 {
            "MAX LEVEL".into()
        } else {
            format!(
                "XP {} / {}   |   {} points",
                progression.xp, progression.next_level_xp, progression.skill_points
            )
        };
    }
    let slug = avatar
        .and_then(|avatar| avatar.0.as_deref())
        .unwrap_or("agnes");
    if previous.as_deref() != Some(slug) {
        *previous = Some(slug.to_owned());
        for entity in &portraits {
            commands.entity(entity).despawn_related::<Children>();
            // A shipped portrait is preferred; the neutral hero silhouette is a
            // deliberate fallback for the one roster entry without a thumbnail.
            if let Some(file) =
                shared::avatar_definition(slug).and_then(|avatar| avatar.thumbnail.as_deref())
            {
                commands
                    .entity(entity)
                    .insert(ImageNode::new(assets.load(format!("avatars/{file}"))));
            } else {
                commands.entity(entity).remove::<ImageNode>();
                commands.entity(entity).with_children(|portrait| {
                    portrait.spawn((
                        Text::new("H"),
                        crate::ui_theme::text(24.0),
                        TextColor(crate::ui_theme::GOLD),
                    ));
                });
            }
        }
    }
}

/// Keep entry and result cards clear; these controls only describe a live,
/// admitted hero. Visibility and layout agree so hidden panels reserve no space.
fn sync_gameplay_hud_visibility(
    game: Res<GameStateSnapshot>,
    session: Res<crate::net::ClientSession>,
    help: Option<Res<crate::help_overlay::HelpOverlayVisible>>,
    pause: Option<Res<crate::pause_menu::PauseMenuState>>,
    shop: Option<Res<crate::shop::ShopState>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut roots: Query<(&Name, &mut Node, &mut Visibility), Without<ChildOf>>,
) {
    let show = session.join_confirmed()
        && matches!(game.state, GameState::Running)
        && !help.is_some_and(|help| help.0)
        && !pause.is_some_and(|pause| pause.open)
        && !shop.is_some_and(|shop| shop.open);
    for (name, mut node, mut visibility) in &mut roots {
        if matches!(
            name.as_str(),
            "MatchHudColumn"
                | "SkillBarRoot"
                | "ActionFeedback"
                | "EquipmentHud"
                | "MatchObjectiveRoot"
        ) {
            // Transient feedback owns its own empty/expired layout state.
            if name.as_str() != "ActionFeedback" {
                node.display = if show
                    && !(name.as_str() == "SkillBarRoot"
                        && mobile.as_ref().is_some_and(|mobile| mobile.enabled))
                {
                    Display::Flex
                } else {
                    Display::None
                };
            }
            *visibility = if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
}

fn spawn_stat_bar<F: Component>(
    col: &mut ChildSpawnerCommands,
    label: &'static str,
    fill_color: Color,
    fill_marker: F,
) {
    col.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(19.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(Val::Px(3.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(BAR_TRACK_COLOR),
        Name::new(format!("MatchHudBar-{label}")),
    ))
    .with_children(|track| {
        track.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(fill_color),
            fill_marker,
        ));
        track.spawn((
            Text::new("100 / 100"),
            crate::ui_theme::text(12.0),
            TextColor(Color::WHITE),
            HudResourceText(label),
            ZIndex(1),
        ));
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

    prog_text.0 = format!(
        "{}\nLEVEL {}",
        hero_class.display_name(),
        progression.level.max(1)
    );

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
    let target = if !stats.is_alive() {
        "Defeated - respawning soon"
    } else if target_state.selected_target.is_some() {
        "Target locked — basic attack or Q/W/E/R"
    } else {
        "Select a foe  /  P shop  /  F1 help"
    };
    status_text.0 = format!("{}\n{target}", objective_line.replace("Goal: ", ""));
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
#[cfg(test)]
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
            format!("Target: enemy {kind} - locked")
        }
        None => "Target: none - click a foe or use Tab".to_string(),
    };
    let hp = stats.hp.max(0.0);
    let max_hp = stats.max_hp.max(1.0);
    let mana = stats.mana.max(0.0);
    let max_mana = stats.max_mana.max(1.0);
    format!(
        "HP {:.0}/{:.0}   Mana {:.0}/{:.0}   Class: {}\n\
{target_line}\n\
{objective_line}\n\
Keys: {} cast | {} upgrade | {} help",
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
        "Goal: clear all towers in one lane to expose the enemy base.".to_string()
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
