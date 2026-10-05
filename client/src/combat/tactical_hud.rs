//! Screen-space combat identity, allied vitals and global identity-only kill notices.
// i18n-strict
use crate::{
    camera::MainCamera,
    combat::CombatStats,
    mobile_controls::MobileControls,
    net::{ClientSession, GameStateSnapshot, NetworkPlayerId, PlayerProgression},
    player::Player,
    team::{AvatarThumbnails, Team},
};
use bevy::prelude::*;
use shared::live_score::{LiveScorePlayer, LiveScoreboard};
use std::collections::HashMap;

#[derive(Component)]
pub(super) struct AlliedPortrait(u64);

#[derive(Default, Resource)]
pub(super) struct TacticalHud {
    plates: HashMap<Entity, [Entity; 4]>,
    allies: Option<Entity>,
    feed_root: Option<Entity>,
    allies_key: String,
    feed_key: String,
    feed: KillFeed,
}

#[derive(Default)]
struct KillFeed {
    round: Option<(u64, u64)>,
    last_id: u64,
    entries: Vec<(LiveScorePlayer, LiveScorePlayer, f64)>,
}
impl KillFeed {
    fn ingest(&mut self, round: (u64, u64), board: &LiveScoreboard, now: f64) {
        if self.round != Some(round) {
            self.round = Some(round);
            self.entries.clear();
            self.last_id = board.kills.iter().map(|e| e.event_id).max().unwrap_or(0);
            return;
        }
        for kill in &board.kills {
            if kill.event_id <= self.last_id {
                continue;
            }
            self.last_id = kill.event_id;
            let killer = board.players.iter().find(|p| p.player_id == kill.killer_id);
            let victim = board.players.iter().find(|p| p.player_id == kill.victim_id);
            if let (Some(killer), Some(victim)) = (killer, victim) {
                self.entries
                    .push((killer.clone(), victim.clone(), now + 6.0));
            }
        }
        self.entries.retain(|(_, _, until)| *until > now);
        if self.entries.len() > 3 {
            self.entries.drain(..self.entries.len() - 3);
        }
    }
}

fn team_color(team: Team) -> Color {
    match team {
        Team::Blue => Color::srgb(0.2, 0.65, 1.0),
        Team::Green => Color::srgb(0.25, 0.95, 0.55),
    }
}
fn label(text: String, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(text),
        crate::ui::theme::text(size),
        TextColor(color),
    )
}
fn ratio(value: f32, max: f32) -> f32 {
    if value.is_finite() && max.is_finite() && max > 0.0 {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}
fn portrait(
    commands: &mut Commands,
    parent: Entity,
    p: &LiveScorePlayer,
    thumbs: Option<&AvatarThumbnails>,
    size: f32,
) {
    let mut entity = commands.spawn((
        Node {
            width: Val::Px(size),
            height: Val::Px(size),
            flex_shrink: 0.0,
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BorderColor::all(team_color(p.team.into())),
        BackgroundColor(Color::BLACK),
        ChildOf(parent),
    ));
    if let Some(image) = p
        .avatar
        .as_ref()
        .and_then(|slug| thumbs.and_then(|t| t.0.get(slug)))
    {
        entity.insert(ImageNode::new(image.clone()));
    } else {
        entity.with_child(label(
            p.nickname.chars().take(1).collect(),
            14.0,
            Color::WHITE,
        ));
    }
}

fn overhead_plate_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Px(104.0),
        height: Val::Px(36.0),
        ..default()
    }
}

fn overhead_fill_node(fraction: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        width: Val::Percent(fraction * 100.0),
        ..default()
    }
}

fn spawn_overhead_plate(commands: &mut Commands) -> [Entity; 4] {
    let root = commands
        .spawn((
            overhead_plate_node(),
            BackgroundColor(Color::srgba(0.015, 0.03, 0.05, 0.88)),
            ZIndex(10),
            Name::new("HeroOverheadPlate"),
        ))
        .id();
    // Independent rows prevent a long or tall nickname from shrinking the bars.
    let text = commands
        .spawn((
            label(String::new(), 11.0, Color::WHITE),
            TextLayout::new_with_no_wrap(),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(2.0),
                top: Val::Px(2.0),
                width: Val::Px(100.0),
                height: Val::Px(14.0),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let fills = [
        (18.0, 8.0, Color::srgb(0.3, 1.0, 0.3), "HeroOverheadHp"),
        (28.0, 4.0, Color::srgb(0.12, 0.55, 1.0), "HeroOverheadMana"),
    ]
    .map(|(top, height, color, name)| {
        let track = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(2.0),
                    top: Val::Px(top),
                    width: Val::Px(100.0),
                    height: Val::Px(height),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.08, 0.1, 0.14)),
                Name::new(name),
                ChildOf(root),
            ))
            .id();
        commands
            .spawn((
                overhead_fill_node(1.0),
                BackgroundColor(color),
                ChildOf(track),
            ))
            .id()
    });
    [root, text, fills[0], fills[1]]
}

/// Screen-pixel-sized plates remain readable at every camera distance.
pub(super) fn update_overhead(
    mut commands: Commands,
    mut state: ResMut<TacticalHud>,
    mobile: Option<Res<MobileControls>>,
    session: Res<ClientSession>,
    game: Res<GameStateSnapshot>,
    scale: Option<Res<UiScale>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    anchors: Query<(&super::bars::CombatBarAnchor, &Transform)>,
    owners: Query<(
        &CombatStats,
        &NetworkPlayerId,
        Option<&PlayerProgression>,
        &Team,
        Has<Player>,
        Option<&crate::net::NetworkHeroClass>,
    )>,
) {
    state.plates.retain(|owner, ids| {
        if owners.get(*owner).is_ok() {
            true
        } else {
            commands.entity(ids[0]).despawn();
            false
        }
    });
    let enabled = mobile.as_ref().is_some_and(|m| m.enabled)
        && session.join_confirmed()
        && matches!(game.state, crate::net::GameState::Running);
    for (anchor, pose) in &anchors {
        let Ok((stats, id, progress, team, local, class)) = owners.get(anchor.target) else {
            continue;
        };
        let position = if enabled && stats.is_alive() {
            camera
                .single()
                .ok()
                .and_then(|(camera, transform)| {
                    camera.world_to_viewport(transform, pose.translation).ok()
                })
                .map(|p| crate::hud_layout::world_to_ui(p, scale.as_deref()))
        } else {
            None
        };
        let ids = *state
            .plates
            .entry(anchor.target)
            .or_insert_with(|| spawn_overhead_plate(&mut commands));
        let mut node = overhead_plate_node();
        if let Some(p) = position {
            node.left = Val::Px(p.x - 52.0);
            node.top = Val::Px(p.y - 36.0);
        } else {
            node.display = Display::None;
        }
        commands.entity(ids[0]).insert(node);
        let name = game
            .scoreboard
            .as_ref()
            .and_then(|b| b.players.iter().find(|p| p.player_id == id.0))
            .map(|p| p.nickname.chars().take(13).collect::<String>())
            .unwrap_or_else(|| {
                crate::i18n::data::hero_name(class.map_or(shared::HeroClass::Warrior, |c| c.0))
                    .chars()
                    .take(13)
                    .collect()
            });
        commands.entity(ids[1]).insert(label(
            format!("{}  {}", progress.map_or(1, |p| p.level), name),
            11.0,
            Color::WHITE,
        ));
        for (entity, value) in [
            (ids[2], ratio(stats.hp, stats.max_hp)),
            (ids[3], ratio(stats.mana, stats.max_mana)),
        ] {
            commands.entity(entity).insert(overhead_fill_node(value));
        }
        commands.entity(ids[2]).insert(BackgroundColor(if local {
            Color::srgb(0.3, 1.0, 0.3)
        } else {
            team_color(*team)
        }));
    }
}

pub(super) fn update_tactical_hud(
    mut commands: Commands,
    mut state: ResMut<TacticalHud>,
    time: Res<Time>,
    game: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    mobile: Option<Res<MobileControls>>,
    thumbs: Option<Res<AvatarThumbnails>>,
    windows: Query<&Window>,
    scale: Option<Res<UiScale>>,
    actors: Query<(&NetworkPlayerId, &CombatStats)>,
    local: Query<&Team, With<Player>>,
    pressed_allies: Query<(&Interaction, &AlliedPortrait), Changed<Interaction>>,
    mut focus: ResMut<crate::camera::AllyCameraFocus>,
    mut navigation: ResMut<crate::minimap::MinimapNavigationState>,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    if context.gameplay_allowed() {
        for (interaction, ally) in &pressed_allies {
            if *interaction == Interaction::Pressed {
                focus.player_id = Some(ally.0);
                navigation.focus_target = None;
            }
        }
    }
    let viewport = windows
        .iter()
        .next()
        .map(|w| crate::hud_layout::ui_viewport(w, scale.as_deref()))
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let layout = crate::hud_layout::HudLayout::resolve(viewport, mobile.as_deref(), false);
    let show = session.join_confirmed() && matches!(game.state, crate::net::GameState::Running);
    let board = game.scoreboard.as_ref();
    if let Some(board) = board {
        state.feed.ingest(
            (game.meta.server_epoch, game.meta.match_id),
            board,
            time.elapsed_secs_f64(),
        );
    }
    let mut allies = Vec::new();
    if show && let (Some(board), Ok(team)) = (board, local.single()) {
        for player in board
            .players
            .iter()
            .filter(|p| p.team == *team && p.player_id != game.your_id)
            .take(4)
        {
            let hp = actors
                .iter()
                .find(|(id, _)| id.0 == player.player_id)
                .map(|(_, s)| ratio(s.hp, s.max_hp));
            allies.push((player, hp));
        }
    }
    // Rebuild only when displayed values change, not on each millisecond of
    // an authoritative countdown snapshot.
    let displayed: Vec<_> = allies
        .iter()
        .map(|(p, hp)| {
            (
                p.player_id,
                &p.nickname,
                &p.avatar,
                p.team,
                *hp,
                p.respawn_remaining_ms.div_ceil(1000),
            )
        })
        .collect();
    let key = format!("{displayed:?}:{:?}:{:?}", layout.allies, focus.player_id);
    if key != state.allies_key {
        if let Some(root) = state.allies.take() {
            commands.entity(root).despawn();
        }
        state.allies_key = key;
        if !allies.is_empty() {
            let root = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(layout.allies.min.x),
                        top: Val::Px(layout.allies.min.y),
                        column_gap: Val::Px(2.0),
                        ..default()
                    },
                    Name::new("AlliedVitals"),
                    ZIndex(12),
                ))
                .id();
            let cell = ((layout.allies.width() - 6.0) / 4.0).clamp(32.0, 44.0);
            for (p, hp) in allies {
                let col = commands
                    .spawn((
                        Node {
                            width: Val::Px(cell),
                            height: Val::Px(44.0),
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(3.0),
                            ..default()
                        },
                        Button,
                        Interaction::None,
                        AlliedPortrait(p.player_id),
                        crate::ui::TestId::new(format!("AllyCamera-{}", p.player_id)),
                        BorderColor::all(if focus.player_id == Some(p.player_id) {
                            crate::ui::tokens::color::GOLD_400
                        } else {
                            Color::NONE
                        }),
                        ChildOf(root),
                    ))
                    .id();
                portrait(
                    &mut commands,
                    col,
                    p,
                    thumbs.as_deref(),
                    (cell - 2.0).min(32.0),
                );
                if p.respawn_remaining_ms > 0 {
                    commands
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                top: Val::Px(0.0),
                                width: Val::Percent(100.0),
                                height: Val::Px(32.0),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                            Pickable::IGNORE,
                            ZIndex(2),
                            ChildOf(col),
                            Name::new("Ally respawn countdown"),
                        ))
                        .with_child((
                            label(
                                p.respawn_remaining_ms.div_ceil(1000).to_string(),
                                19.0,
                                Color::WHITE,
                            ),
                            Pickable::IGNORE,
                        ));
                }
                let bg = commands
                    .spawn((
                        Node {
                            width: Val::Px((cell - 2.0).min(32.0)),
                            height: Val::Px(5.0),
                            ..default()
                        },
                        BackgroundColor(Color::BLACK),
                        ChildOf(col),
                    ))
                    .id();
                if let Some(hp) = hp {
                    commands.spawn((
                        Node {
                            width: Val::Percent(hp * 100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(team_color(p.team.into())),
                        ChildOf(bg),
                    ));
                } else {
                    commands.spawn((label("?".into(), 10.0, Color::WHITE), ChildOf(col)));
                }
            }
            state.allies = Some(root);
        }
    }
    let key = format!("{show}:{:?}:{:?}", state.feed.entries, layout.kill_feed());
    if key != state.feed_key {
        if let Some(root) = state.feed_root.take() {
            commands.entity(root).despawn();
        }
        state.feed_key = key;
        if show && !state.feed.entries.is_empty() {
            let root = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(layout.kill_feed().min.x),
                        top: Val::Px(layout.kill_feed().min.y),
                        width: Val::Px(layout.kill_feed().width()),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(3.0),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Pickable::IGNORE,
                    ZIndex(20),
                    Name::new("KillFeed"),
                ))
                .id();
            let phone = mobile.as_ref().is_some_and(|m| m.enabled);
            let portrait_size = if phone { 20.0 } else { 24.0 };
            let font_size = if phone { 10.0 } else { 11.0 };
            for (killer, victim, _) in state.feed.entries.iter().rev() {
                let row = commands
                    .spawn((
                        Node {
                            column_gap: Val::Px(3.0),
                            max_width: Val::Percent(100.0),
                            align_self: AlignSelf::FlexEnd,
                            align_items: AlignItems::Center,
                            padding: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.01, 0.02, 0.04, 0.85)),
                        ChildOf(root),
                    ))
                    .id();
                portrait(&mut commands, row, killer, thumbs.as_deref(), portrait_size);
                commands.spawn((
                    label(
                        killer.nickname.chars().take(12).collect(),
                        font_size,
                        team_color(killer.team.into()),
                    ),
                    TextLayout::new_with_no_wrap(),
                    Node {
                        min_width: Val::Px(0.0),
                        max_width: Val::Px(if phone { 72.0 } else { 104.0 }),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    ChildOf(row),
                ));
                commands.spawn((
                    crate::ui::widgets::icon_node(
                        crate::ui::kit_assets::Icon::HudKill,
                        16.0,
                        Color::WHITE,
                    ),
                    ChildOf(row),
                ));
                portrait(&mut commands, row, victim, thumbs.as_deref(), portrait_size);
                commands.spawn((
                    label(
                        victim.nickname.chars().take(12).collect(),
                        font_size,
                        team_color(victim.team.into()),
                    ),
                    TextLayout::new_with_no_wrap(),
                    Node {
                        min_width: Val::Px(0.0),
                        max_width: Val::Px(if phone { 72.0 } else { 104.0 }),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    ChildOf(row),
                ));
            }
            state.feed_root = Some(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipad_overhead_layout_keeps_hp_and_mana_visible_under_long_names() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::image::ImagePlugin::default(),
            bevy::text::TextPlugin,
            bevy::transform::TransformPlugin,
            bevy::input::InputPlugin,
            bevy::ui::UiPlugin,
            bevy::camera::visibility::VisibilityPlugin,
            bevy::picking::PickingPlugin,
            bevy::picking::InteractionPlugin,
        ));
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<TextureAtlasLayout>>();
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set(1180.0, 820.0);
        app.world_mut().spawn((window, bevy::window::PrimaryWindow));
        app.world_mut().spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(1180, 820),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        let ids = spawn_overhead_plate(&mut app.world_mut().commands());
        app.world_mut().flush();
        app.world_mut()
            .entity_mut(ids[1])
            .insert(Text::new("20  WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW"));
        app.finish();
        app.cleanup();
        for fraction in [1.0, 0.5, 0.0] {
            for id in [ids[2], ids[3]] {
                app.world_mut()
                    .entity_mut(id)
                    .insert(overhead_fill_node(fraction));
            }
            for _ in 0..4 {
                app.update();
            }
            let world = app.world();
            assert_eq!(
                world.get::<ComputedNode>(ids[0]).unwrap().size(),
                Vec2::new(104.0, 36.0)
            );
            assert_eq!(world.get::<ComputedNode>(ids[1]).unwrap().size().y, 14.0);
            for (id, height) in [(ids[2], 8.0), (ids[3], 4.0)] {
                let track = world.get::<ChildOf>(id).unwrap().parent();
                assert_eq!(
                    world.get::<ComputedNode>(track).unwrap().size(),
                    Vec2::new(100.0, height)
                );
                assert_eq!(
                    world.get::<ComputedNode>(id).unwrap().size(),
                    Vec2::new(100.0 * fraction, height)
                );
            }
        }
    }

    #[test]
    fn kills_are_global_identity_only_deduplicated_expire_and_reset() {
        let player = |id, team| LiveScorePlayer {
            player_id: id,
            nickname: format!("Player{id}"),
            avatar: Some("agnes".into()),
            team,
            hero_class: shared::HeroClass::Wildspark,
            kills: 0,
            deaths: 0,
            assists: 0,
            earned_gold: 0,
            level: 1,
            connected: true,
            respawn_remaining_ms: 0,
        };
        let mut board = LiveScoreboard {
            elapsed_secs: 0,
            kills: vec![],
            players: vec![
                player(1, shared::map::Team::Blue),
                player(2, shared::map::Team::Green),
            ],
        };
        let mut feed = KillFeed::default();
        feed.ingest((1, 1), &board, 0.0);
        board.kills.push(shared::live_score::KillNotice {
            event_id: 5,
            killer_id: 1,
            victim_id: 2,
        });
        feed.ingest((1, 1), &board, 1.0);
        feed.ingest((1, 1), &board, 2.0);
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].0.nickname, "Player1");
        assert_eq!(feed.entries[0].1.avatar.as_deref(), Some("agnes"));
        feed.ingest((1, 1), &board, 7.1);
        assert!(feed.entries.is_empty());
        board.kills.push(shared::live_score::KillNotice {
            event_id: 6,
            killer_id: 2,
            victim_id: 1,
        });
        feed.ingest((1, 1), &board, 8.0);
        assert_eq!(feed.entries.len(), 1);
        feed.ingest((1, 2), &board, 8.1);
        assert!(
            feed.entries.is_empty(),
            "new rounds do not replay retained notices"
        );
    }
}
