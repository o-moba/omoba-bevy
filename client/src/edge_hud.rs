//! Compact match chrome, selected-target health and the live two-team scoreboard.
use crate::{
    combat::{CombatStats, TargetState},
    input_context::InputContextSet,
    mobile_controls::MobileControls,
    net::{
        ClientSession, GameState, GameStateSnapshot, NetworkHeroClass, NetworkMinionId,
        NetworkNeutralId, NetworkPlayerId, NetworkStructureId, TargetKind,
    },
    player::Player,
    ui_theme as ui,
};
use bevy::{
    input::{
        mouse::{MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    prelude::*,
    window::PrimaryWindow,
};
use shared::{
    live_score::{LiveScorePlayer, LiveScoreboard},
    map::Team,
};

pub(crate) struct EdgeHudPlugin;
#[derive(Resource, Default)]
pub(crate) struct ScoreboardState {
    pub open: bool,
    namespace: Option<(u64, u64)>,
}
#[derive(Component)]
struct EdgePart;
#[derive(Component)]
struct ScoreLabel;
#[derive(Component)]
struct KdaLabel;
#[derive(Component)]
struct TargetLabel;
#[derive(Component)]
struct TargetValue;
#[derive(Component)]
struct TargetFill;
#[derive(Component)]
struct ScoreRows(Team);
#[derive(Component)]
struct ScoreScroll;
#[derive(Component)]
struct ScoreDetail;

impl Plugin for EdgeHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScoreboardState>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                actions
                    .in_set(InputContextSet::Modal)
                    .before(crate::help_overlay::HelpOverlaySet::Input)
                    .before(crate::shop::ShopModalSet),
            )
            .add_systems(
                Update,
                (update, render_rows, scroll_rows)
                    .chain()
                    .after(InputContextSet::Resolve),
            )
            .add_systems(PostUpdate, layout.before(bevy::ui::UiSystems::Layout));
    }
}
fn button_node(width: f32) -> Node {
    Node {
        width: Val::Px(width),
        height: Val::Px(44.0),
        flex_shrink: 0.0,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}
fn text(parent: &mut ChildSpawnerCommands, value: &str, size: f32, color: Color) {
    parent.spawn((Text::new(value), ui::text(size), TextColor(color)));
}
fn setup(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(166.0),
                top: Val::Px(16.0),
                column_gap: Val::Px(6.0),
                ..default()
            },
            EdgePart,
            ZIndex(14),
            Name::new("MatchScoreStrip"),
        ))
        .with_children(|row| {
            row.spawn((
                Button,
                button_node(68.0),
                BackgroundColor(ui::PANEL),
                BorderColor::all(ui::EDGE),
                Name::new("MatchScoreButton"),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new("— : —"),
                    ui::text(16.0),
                    TextColor(ui::GOLD),
                    ScoreLabel,
                ));
            });
            row.spawn((
                Node {
                    width: Val::Px(76.0),
                    height: Val::Px(44.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(ui::PANEL),
            ))
            .with_children(|p| {
                text(p, "K / D / A", 12.0, ui::MUTED);
                p.spawn((
                    Text::new("—/—/—"),
                    ui::text(14.0),
                    TextColor(ui::IVORY),
                    KdaLabel,
                    Name::new("MatchKdaText"),
                ));
            });
        });
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                top: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..button_node(44.0)
            },
            BackgroundColor(ui::PANEL),
            BorderColor::all(ui::EDGE),
            ZIndex(14),
            EdgePart,
            Name::new("MatchMenuButton"),
        ))
        .with_children(|p| {
            for _ in 0..3 {
                p.spawn((
                    Node {
                        width: Val::Px(20.0),
                        height: Val::Px(2.0),
                        ..default()
                    },
                    BackgroundColor(ui::GOLD),
                ));
            }
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                width: Val::Px(224.0),
                height: Val::Px(34.0),
                padding: UiRect::all(Val::Px(2.0)),
                flex_direction: FlexDirection::Column,
                ..ui::panel_node()
            },
            BackgroundColor(ui::PANEL),
            BorderColor::all(ui::EDGE),
            EdgePart,
            ZIndex(14),
            Name::new("TargetHealthRoot"),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                ui::text(12.0),
                TextColor(ui::IVORY),
                TargetLabel,
                Name::new("TargetHealthName"),
                Node {
                    height: Val::Px(14.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ));
            p.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(14.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.12, 0.04, 0.04)),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.70, 0.16, 0.12)),
                    TargetFill,
                    Name::new("TargetHealthFill"),
                ));
                bar.spawn((
                    Text::new(""),
                    ui::text(12.0),
                    TextColor(ui::IVORY),
                    TargetValue,
                    ZIndex(1),
                    Name::new("TargetHealthValue"),
                ));
            });
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.015, 0.02, 0.72)),
            ZIndex(90),
            Name::new("ScoreboardRoot"),
        ))
        .with_children(|overlay| {
            overlay.spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                Name::new("ScoreboardBackdrop"),
            ));
            overlay
                .spawn((
                    Node {
                        width: Val::Px(760.0),
                        max_width: Val::Percent(94.0),
                        max_height: Val::Percent(92.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..ui::panel_node()
                    },
                    BackgroundColor(ui::PANEL.with_alpha(1.0)),
                    BorderColor::all(ui::EDGE),
                    ZIndex(1),
                    Name::new("ScoreboardPanel"),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        },))
                        .with_children(|row| {
                            text(row, "MATCH SCORE", 20.0, ui::GOLD);
                            row.spawn((
                                Button,
                                button_node(64.0),
                                BackgroundColor(ui::TILE),
                                BorderColor::all(ui::EDGE),
                                Name::new("ScoreboardCloseButton"),
                            ))
                            .with_children(|p| text(p, "Close", 14.0, ui::IVORY));
                        });
                    panel
                        .spawn((Node {
                            column_gap: Val::Px(12.0),
                            ..default()
                        },))
                        .with_children(|tables| {
                            for team in [Team::Green, Team::Blue] {
                                tables
                                    .spawn((Node {
                                        width: Val::Percent(50.0),
                                        min_width: Val::Px(0.0),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    },))
                                    .with_children(|col| {
                                        text(
                                            col,
                                            if team == Team::Green {
                                                "GREEN TEAM"
                                            } else {
                                                "BLUE TEAM"
                                            },
                                            14.0,
                                            if team == Team::Green {
                                                ui::JADE
                                            } else {
                                                Color::srgb(0.50, 0.72, 1.0)
                                            },
                                        );
                                        spawn_score_row(
                                            col, "PLAYER", "K/D/A", "GOLD ↑", "LV", false, true,
                                        );
                                        col.spawn((
                                            Node {
                                                height: Val::Px(190.0),
                                                flex_direction: FlexDirection::Column,
                                                overflow: Overflow::scroll_y(),
                                                ..default()
                                            },
                                            ScrollPosition::default(),
                                            ScoreRows(team),
                                            ScoreScroll,
                                            Name::new(if team == Team::Green {
                                                "ScoreboardGreenRows"
                                            } else {
                                                "ScoreboardBlueRows"
                                            }),
                                        ));
                                    });
                            }
                        });
                    panel.spawn((
                        Text::new(""),
                        ui::text(12.0),
                        TextColor(ui::MUTED),
                        ScoreDetail,
                        Name::new("ScoreboardDetail"),
                    ));
                });
        });
}
fn actions(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    mut state: ResMut<ScoreboardState>,
    mut pause: ResMut<crate::pause_menu::PauseMenuState>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    shop: Option<Res<crate::shop::ShopState>>,
    social: Option<Res<crate::social::SocialClient>>,
    career: Option<Res<crate::career::CareerClient>>,
    supporter: Option<Res<crate::supporter::SupporterUiState>>,
    screen: Option<Res<State<crate::frontend::AppScreen>>>,
    entry: Option<Res<crate::mobile_ui::ServerEntry>>,
    mobile: Option<Res<MobileControls>>,
    buttons: Query<(&Name, &Interaction), (With<Button>, Changed<Interaction>)>,
) {
    let identity = (game.meta.server_epoch, game.meta.match_id);
    let allowed = session.join_confirmed() && matches!(game.state, GameState::Running);
    if state.namespace != Some(identity) || !allowed {
        state.open = false;
        state.namespace = Some(identity);
    }
    let other_modal = pause.open
        || help.0
        || shop.as_ref().is_some_and(|s| s.open)
        || social.as_ref().is_some_and(|s| s.blocks_gameplay())
        || career.as_ref().is_some_and(|s| s.modal_open())
        || supporter.as_ref().is_some_and(|s| s.open)
        || screen.as_ref().is_some_and(|s| s.get().is_menu())
        || entry.as_ref().is_some_and(|s| s.open)
        || mobile
            .as_ref()
            .is_some_and(|s| s.enabled && (!s.landscape || !s.focused));
    if other_modal {
        state.open = false;
    }
    let can_open = allowed && !other_modal;
    if state.open && keys.just_pressed(KeyCode::Escape) {
        state.open = false;
        keys.clear_just_pressed(KeyCode::Escape);
    }
    if can_open && keys.just_pressed(KeyCode::Tab) {
        state.open = !state.open;
        keys.clear_just_pressed(KeyCode::Tab);
    }
    for (name, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match name.as_str() {
            "ScoreboardCloseButton" | "ScoreboardBackdrop" => state.open = false,
            "MatchScoreButton" if can_open => state.open = !state.open,
            "MatchMenuButton" if can_open => {
                state.open = false;
                pause.open = true;
                pause.in_settings = false;
            }
            _ => {}
        }
    }
}
fn scores(board: Option<&LiveScoreboard>, local: Option<u64>) -> (String, String) {
    let Some(board) = board else {
        return ("— : —".into(), "—/—/—".into());
    };
    let sum = |team| {
        board
            .players
            .iter()
            .filter(|p| p.team == team)
            .fold(0u32, |n, p| n.saturating_add(p.kills))
    };
    let kda = board
        .players
        .iter()
        .find(|p| Some(p.player_id) == local)
        .map_or_else(
            || "—/—/—".into(),
            |p| format!("{}/{}/{}", p.kills, p.deaths, p.assists),
        );
    (format!("{} : {}", sum(Team::Green), sum(Team::Blue)), kda)
}
fn update(
    state: Res<ScoreboardState>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    context: Res<crate::input_context::GameplayInputContext>,
    target: Res<TargetState>,
    local: Query<&NetworkPlayerId, With<Player>>,
    local_team: Query<&crate::team::Team, With<Player>>,
    targets: Query<(
        &CombatStats,
        Option<&NetworkPlayerId>,
        Option<&NetworkMinionId>,
        Option<&NetworkStructureId>,
        Option<&NetworkNeutralId>,
        Option<&NetworkHeroClass>,
    )>,
    mut labels: Query<(
        &mut Text,
        Option<&ScoreLabel>,
        Option<&KdaLabel>,
        Option<&TargetLabel>,
        Option<&TargetValue>,
        Option<&ScoreDetail>,
    )>,
    mut nodes: Query<(&Name, &mut Node)>,
) {
    let (score, kda) = scores(game.scoreboard.as_ref(), local.single().ok().map(|id| id.0));
    let details = target
        .selected_entity
        .zip(target.selected_target)
        .and_then(|(entity, id)| {
            let (stats, player, minion, structure, neutral, class) = targets.get(entity).ok()?;
            let matches = match id.kind {
                TargetKind::Player => player.map(|p| p.0) == Some(id.id),
                TargetKind::Minion => minion.map(|p| p.0) == Some(id.id),
                TargetKind::Structure => structure.map(|p| p.0) == Some(id.id),
                TargetKind::Neutral => neutral.map(|p| p.0) == Some(id.id),
            };
            if !matches || !stats.is_alive() || !stats.max_hp.is_finite() || stats.max_hp <= 0.0 {
                return None;
            }
            let name = match id.kind {
                TargetKind::Player => game
                    .scoreboard
                    .as_ref()
                    .and_then(|s| s.players.iter().find(|p| p.player_id == id.id))
                    .map(|p| p.nickname.clone())
                    .unwrap_or_else(|| class.map_or("Hero", |c| c.0.display_name()).into()),
                TargetKind::Minion => "Minion".into(),
                TargetKind::Structure => "Tower / base".into(),
                TargetKind::Neutral => "Neutral monster".into(),
            };
            Some((name, stats.hp.max(0.0), stats.max_hp))
        });
    for (mut text, score_label, kda_label, target_label, value, detail) in &mut labels {
        if score_label.is_some() {
            text.0.clone_from(&score);
        } else if kda_label.is_some() {
            text.0.clone_from(&kda);
        } else if target_label.is_some() {
            text.0 = details
                .as_ref()
                .map_or_else(String::new, |(n, _, _)| short_name(n, 24));
        } else if value.is_some() {
            text.0 = details
                .as_ref()
                .map_or_else(String::new, |(_, hp, max)| format!("{hp:.0} / {max:.0}"));
        } else if detail.is_some() {
            text.0="Gold ↑ is total income this round; starting gold excluded. Clear one lane to unlock the enemy base.".into();
            let buffs = local_team
                .single()
                .map(|team| crate::match_hud::team_buff_hud_text(&game.team_buffs, *team))
                .unwrap_or_default();
            if !buffs.is_empty() {
                text.0.push('\n');
                text.0.push_str(&buffs);
            }
        }
    }
    let resting =
        session.join_confirmed() && matches!(game.state, GameState::Running) && !context.modal_open;
    for (name, mut node) in &mut nodes {
        let show = match name.as_str() {
            "MatchScoreStrip" | "MatchMenuButton" => Some(resting),
            "TargetHealthRoot" => Some(resting && details.is_some()),
            "ScoreboardRoot" => Some(state.open),
            _ => None,
        };
        if let Some(show) = show {
            node.display = if show { Display::Flex } else { Display::None };
        }
        if name.as_str() == "TargetHealthFill" {
            node.width = Val::Percent(
                details
                    .as_ref()
                    .map_or(0.0, |(_, hp, max)| (hp / max).clamp(0.0, 1.0) * 100.0),
            );
        }
    }
}
fn short_name(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
fn spawn_score_row(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    kda: &str,
    gold: &str,
    level: &str,
    local: bool,
    header: bool,
) {
    parent
        .spawn((
            Node {
                height: Val::Px(29.0),
                min_height: Val::Px(29.0),
                column_gap: Val::Px(4.0),
                padding: UiRect::horizontal(Val::Px(3.0)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(if local {
                Color::srgb(0.055, 0.20, 0.16)
            } else {
                Color::NONE
            }),
        ))
        .with_children(|row| {
            for (value, width, grow) in [
                (name, 0.0, 1.0),
                (kda, 61.0, 0.0),
                (gold, 52.0, 0.0),
                (level, 22.0, 0.0),
            ] {
                row.spawn((
                    Text::new(value),
                    ui::text(12.0),
                    TextColor(if header {
                        ui::MUTED
                    } else if local {
                        ui::JADE
                    } else {
                        ui::IVORY
                    }),
                    Node {
                        width: if grow > 0.0 {
                            Val::Auto
                        } else {
                            Val::Px(width)
                        },
                        min_width: Val::Px(0.0),
                        flex_grow: grow,
                        flex_shrink: if grow > 0.0 { 1.0 } else { 0.0 },
                        overflow: Overflow::clip(),
                        ..default()
                    },
                ));
            }
        });
}
fn render_rows(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    local: Query<&NetworkPlayerId, With<Player>>,
    rows: Query<(Entity, &ScoreRows)>,
    mut previous: Local<Option<(Option<LiveScoreboard>, Option<u64>)>>,
) {
    let id = local.single().ok().map(|p| p.0);
    let next = (game.scoreboard.clone(), id);
    if previous.as_ref() == Some(&next) {
        return;
    }
    *previous = Some(next);
    for (entity, team) in &rows {
        commands.entity(entity).despawn_related::<Children>();
        commands.entity(entity).with_children(|parent| {
            let Some(board) = game.scoreboard.as_ref() else {
                text(parent, "Waiting for live score…", 12.0, ui::MUTED);
                return;
            };
            let mut players: Vec<&LiveScorePlayer> =
                board.players.iter().filter(|p| p.team == team.0).collect();
            players.sort_by_key(|p| (std::cmp::Reverse(p.kills), p.player_id));
            if players.is_empty() {
                text(parent, "No players", 12.0, ui::MUTED);
            }
            for p in players {
                let name = if p.connected {
                    short_name(&p.nickname, 12)
                } else {
                    format!("{} · off", short_name(&p.nickname, 7))
                };
                spawn_score_row(
                    parent,
                    &name,
                    &format!("{}/{}/{}", p.kills, p.deaths, p.assists),
                    &p.earned_gold.to_string(),
                    &p.level.to_string(),
                    Some(p.player_id) == id,
                    false,
                );
            }
        });
    }
}
fn layout(
    windows: Query<&Window, With<PrimaryWindow>>,
    mobile: Res<MobileControls>,
    mut nodes: Query<(&Name, &mut Node)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let phone = mobile.enabled;
    let right = if phone { mobile.safe.right } else { 16.0 };
    let top = if phone { mobile.safe.top } else { 16.0 };
    for (name, mut node) in &mut nodes {
        match name.as_str() {
            "MatchScoreStrip" => {
                node.right = Val::Px(right + 150.0);
                node.top = Val::Px(top);
            }
            "MatchMenuButton" => {
                node.right = Val::Px(right);
                node.top = Val::Px(top);
            }
            "TargetHealthRoot" => {
                let width = if phone { 160.0 } else { 224.0 };
                node.left = Val::Px((window.width() - width) * 0.5);
                node.top = Val::Px(
                    top + if phone && window.width() < 800.0 {
                        48.0
                    } else {
                        0.0
                    },
                );
                node.width = Val::Px(width);
            }
            "ScoreboardRoot" => {
                node.padding = if phone {
                    UiRect {
                        left: Val::Px(mobile.safe.left),
                        right: Val::Px(mobile.safe.right),
                        top: Val::Px(mobile.safe.top),
                        bottom: Val::Px(mobile.safe.bottom),
                    }
                } else {
                    UiRect::ZERO
                };
            }
            "ScoreboardPanel" => {
                node.max_width = Val::Percent(if phone { 100.0 } else { 94.0 });
                node.max_height = Val::Percent(if phone { 100.0 } else { 92.0 });
                node.width = Val::Px(if phone { 620.0 } else { 760.0 });
                node.padding = UiRect::all(Val::Px(if phone { 12.0 } else { 16.0 }));
            }
            "ScoreboardGreenRows" | "ScoreboardBlueRows" => {
                node.height = Val::Px(if phone {
                    if window.height() <= 340.0 {
                        135.0
                    } else {
                        145.0
                    }
                } else {
                    190.0
                });
            }
            _ => {}
        }
    }
}
fn scroll_rows(
    state: Res<ScoreboardState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut wheel: MessageReader<MouseWheel>,
    mut touches: MessageReader<TouchInput>,
    mut held: Local<Option<(u64, Vec2)>>,
    mut rows: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<ScoreScroll>>,
) {
    if !state.open {
        wheel.clear();
        touches.clear();
        *held = None;
        return;
    }
    let pointer = windows.single().ok().and_then(Window::cursor_position);
    let mut delta = 0.0;
    let mut point = pointer;
    for event in wheel.read() {
        delta -= event.y
            * if event.unit == MouseScrollUnit::Line {
                28.0
            } else {
                1.0
            };
    }
    for event in touches.read() {
        match event.phase {
            TouchPhase::Started => *held = Some((event.id, event.position)),
            TouchPhase::Moved => {
                if let Some((id, previous)) = held.as_mut() {
                    if *id == event.id {
                        delta += previous.y - event.position.y;
                        point = Some(event.position);
                        *previous = event.position;
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if held.is_some_and(|(id, _)| id == event.id) {
                    *held = None;
                }
            }
        }
    }
    let Some(point) = point else {
        return;
    };
    for (node, transform, mut scroll) in &mut rows {
        let center = transform.translation * node.inverse_scale_factor();
        let size = node.size() * node.inverse_scale_factor();
        if Rect::from_center_size(center, size).contains(point) {
            let max =
                (node.content_size().y - node.size().y).max(0.0) * node.inverse_scale_factor();
            scroll.0.y = (scroll.0.y + delta).clamp(0.0, max);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(ClientSession::admitted_for_test())
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::pause_menu::PauseMenuState>()
            .init_resource::<crate::help_overlay::HelpOverlayVisible>()
            .init_resource::<MobileControls>()
            .init_resource::<TargetState>()
            .add_message::<MouseWheel>()
            .add_message::<TouchInput>()
            .add_plugins((crate::input_context::InputContextPlugin, EdgeHudPlugin))
            .add_systems(
                Update,
                crate::pause_menu::toggle_pause_menu
                    .in_set(InputContextSet::Modal)
                    .after(crate::shop::ShopModalSet),
            );
        app
    }
    fn named(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .query::<(Entity, &Name)>()
            .iter(app.world())
            .find(|(_, n)| n.as_str() == name)
            .unwrap()
            .0
    }
    #[test]
    fn score_button_blocks_same_frame_escape_restores_and_round_change_closes() {
        let mut app = app();
        app.update();
        let score = named(&mut app, "MatchScoreButton");
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::Pressed);
        app.update();
        assert!(app.world().resource::<ScoreboardState>().open);
        assert!(
            !app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<ScoreboardState>().open);
        assert!(
            !app.world()
                .resource::<crate::pause_menu::PauseMenuState>()
                .open
        );
        assert!(
            app.world()
                .resource::<crate::input_context::GameplayInputContext>()
                .gameplay_allowed()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .get_mut::<Interaction>(score)
            .unwrap()
            .clone_from(&Interaction::Pressed);
        app.update();
        assert!(app.world().resource::<ScoreboardState>().open);
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .match_id += 1;
        app.update();
        assert!(!app.world().resource::<ScoreboardState>().open);
    }
    #[test]
    fn scoreboard_cannot_reopen_over_chat_or_frontend_from_key_or_button() {
        for frontend in [false, true] {
            let mut app = app();
            if frontend {
                app.insert_resource(State::new(crate::frontend::AppScreen::Home));
            } else {
                let mut social = crate::social::SocialClient::default();
                social.chat_open = true;
                app.insert_resource(social);
            }
            app.update();
            let score = named(&mut app, "MatchScoreButton");
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Tab);
            app.update();
            assert!(!app.world().resource::<ScoreboardState>().open);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            *app.world_mut().get_mut::<Interaction>(score).unwrap() = Interaction::Pressed;
            app.update();
            assert!(!app.world().resource::<ScoreboardState>().open);
            if frontend {
                app.insert_resource(State::new(crate::frontend::AppScreen::InMatch));
            } else {
                app.world_mut()
                    .resource_mut::<crate::social::SocialClient>()
                    .chat_open = false;
            }
            *app.world_mut().get_mut::<Interaction>(score).unwrap() = Interaction::None;
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Tab);
            app.update();
            assert!(app.world().resource::<ScoreboardState>().open);
        }
    }
    #[test]
    fn target_health_follows_each_kind_and_clears_invalid_or_dead_locks() {
        let mut app = app();
        app.update();
        let root = named(&mut app, "TargetHealthRoot");
        let fill = named(&mut app, "TargetHealthFill");
        let value = named(&mut app, "TargetHealthValue");
        for kind in [
            TargetKind::Player,
            TargetKind::Minion,
            TargetKind::Structure,
            TargetKind::Neutral,
        ] {
            let mut entity = app.world_mut().spawn(CombatStats {
                hp: 100.0,
                max_hp: 300.0,
                mana: 0.0,
                max_mana: 0.0,
            });
            match kind {
                TargetKind::Player => {
                    entity.insert(NetworkPlayerId(42));
                }
                TargetKind::Minion => {
                    entity.insert(NetworkMinionId(42));
                }
                TargetKind::Structure => {
                    entity.insert(NetworkStructureId(42));
                }
                TargetKind::Neutral => {
                    entity.insert(NetworkNeutralId(42));
                }
            }
            let entity = entity.id();
            {
                let mut target = app.world_mut().resource_mut::<TargetState>();
                target.selected_entity = Some(entity);
                target.selected_target = Some(crate::net::TargetId { kind, id: 42 });
            }
            app.update();
            assert_eq!(app.world().get::<Text>(value).unwrap().0, "100 / 300");
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::Flex
            );
            let Val::Percent(percent) = app.world().get::<Node>(fill).unwrap().width else {
                panic!("health fill must be proportional")
            };
            assert!((percent - 100.0 / 3.0).abs() < 0.01);
            app.world_mut().get_mut::<CombatStats>(entity).unwrap().hp = 63.0;
            app.update();
            assert_eq!(app.world().get::<Text>(value).unwrap().0, "63 / 300");
            app.world_mut()
                .resource_mut::<TargetState>()
                .selected_target
                .as_mut()
                .unwrap()
                .id = 43;
            app.update();
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::None
            );
            app.world_mut()
                .resource_mut::<TargetState>()
                .selected_target
                .as_mut()
                .unwrap()
                .id = 42;
            app.world_mut().get_mut::<CombatStats>(entity).unwrap().hp = 0.0;
            app.update();
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                Display::None
            );
            app.world_mut().despawn(entity);
        }
    }
    #[test]
    fn live_score_totals_use_both_teams_and_distinguish_missing_data() {
        let player = |id, team, kills| LiveScorePlayer {
            player_id: id,
            nickname: "Player".into(),
            team,
            hero_class: shared::HeroClass::Warrior,
            kills,
            deaths: 2,
            assists: 3,
            earned_gold: 57,
            level: 4,
            connected: true,
        };
        let board = LiveScoreboard {
            players: vec![
                player(1, Team::Green, 4),
                player(2, Team::Green, 5),
                player(3, Team::Blue, 6),
            ],
        };
        assert_eq!(
            scores(Some(&board), Some(2)),
            ("9 : 6".into(), "5/2/3".into())
        );
        assert_eq!(scores(None, Some(2)), ("— : —".into(), "—/—/—".into()));
        assert_eq!(
            scores(Some(&LiveScoreboard::default()), None),
            ("0 : 0".into(), "—/—/—".into())
        );
    }
}
