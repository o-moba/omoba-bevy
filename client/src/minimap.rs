//! Tactical overlay. Detection affects enemy hero markers only; world rendering
//! and network snapshots do not yet implement fog of war.
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::camera::{CameraState, MainCamera};
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::{
    ClientSession, NetworkAvatar, NetworkHeroClass, NetworkMinion, NetworkNeutral,
    NetworkNeutralCampType, NetworkSpriteCharacter, NetworkStructure, NeutralCampType,
    RemotePlayer, StructureKind,
};
use crate::player::{PLAYER_SIZE, Player};
use crate::sprite::{PlayerVisualMode, SpriteVisualAssets};
use crate::team::{AvatarThumbnails, Team};
use crate::ui_theme;

pub(crate) const MINIMAP_SIZE: f32 = 252.0;
pub(crate) const DESKTOP_MINIMAP_INSET: f32 = 16.0;
pub(crate) const MINIMAP_INNER_SIZE: f32 = 232.0;
const HERO_SIGHT: f32 = 32.0;
const MINION_SIGHT: f32 = 22.0;
const TOWER_SIGHT: f32 = 28.0;
const BASE_SIGHT: f32 = 34.0;

pub struct MinimapPlugin;
impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::minimap_route::MinimapRoutePlugin)
            .init_resource::<MinimapUiState>()
            .init_resource::<MinimapNavigationState>()
            .add_systems(Startup, setup_minimap_ui)
            .add_systems(
                Update,
                handle_minimap_navigation_system
                    .in_set(crate::input_context::InputContextSet::Actions)
                    .before(crate::combat::CombatPointerInputSet),
            )
            .add_systems(
                PostUpdate,
                (
                    update_minimap_icons_system,
                    sync_minimap_visibility_for_session,
                    update_camera_footprint.after(bevy::camera::CameraUpdateSystems),
                )
                    .before(bevy::ui::UiSystems::Layout),
            );
    }
}

#[derive(Resource, Default)]
pub struct MinimapNavigationState {
    pub focus_target: Option<Vec3>,
    pub consumed_primary_click: bool,
    /// A one-frame move order, separate from the persistent camera focus.
    pub movement_target: Option<Vec3>,
    /// Only a finger that started on the map may pan it.
    touch_id: Option<u64>,
}
#[derive(Resource, Default)]
struct MinimapUiState {
    container: Option<Entity>,
    player_icons: HashMap<Entity, (Entity, String)>,
    structure_icons: HashMap<Entity, Entity>,
    minion_icons: HashMap<Entity, Entity>,
    camp_icons: [Option<Entity>; 6],
}
#[derive(Component)]
struct MinimapRoot;

/// Persistent anchor marker; life state comes only from current neutral entities.
#[derive(Component, Clone, Copy, Debug)]
struct MinimapCamp {
    index: usize,
    alive: bool,
}
#[derive(Component)]
pub(crate) struct MinimapContainer;
#[derive(Component)]
struct CameraFootprintEdge(usize);

/// Read-only diagnostics for opt-in native QA. These inspect the actual UI
/// entities and computed transforms; they do not fabricate marker fixtures.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MinimapQaScene<'w, 's> {
    state: Res<'w, MinimapUiState>,
    nodes: Query<
        'w,
        's,
        (
            &'static Node,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static InheritedVisibility>,
        ),
    >,
    edges: Query<'w, 's, (Entity, &'static CameraFootprintEdge)>,
    routes: Query<'w, 's, (Entity, &'static crate::minimap_route::RouteSegment)>,
    camps: Query<'w, 's, (Entity, &'static MinimapCamp)>,
    destinations: Query<'w, 's, Entity, With<crate::minimap_route::RouteDestination>>,
    heroes: Query<
        'w,
        's,
        (&'static Team, Option<&'static Player>),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
}

impl MinimapQaScene<'_, '_> {
    pub(crate) fn diagnostics(&self) -> serde_json::Value {
        let rendered_rect = |entity| {
            let (node, computed, transform, visibility) = self.nodes.get(entity).ok()?;
            if node.display == Display::None || visibility.is_some_and(|value| !value.get()) {
                return None;
            }
            container_rect(computed, transform)
        };
        let rect_json = |rect: Rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
        let local_team = self
            .heroes
            .iter()
            .find(|(_, player)| player.is_some())
            .map(|(team, _)| *team);
        let mut local = 0;
        let mut allied = 0;
        let mut enemy = 0;
        let mut markers = Vec::new();
        for (hero, (icon, _)) in &self.state.player_icons {
            let (Some(rect), Ok((team, player))) = (rendered_rect(*icon), self.heroes.get(*hero))
            else {
                continue;
            };
            if player.is_some() {
                local += 1;
            } else if Some(*team) == local_team {
                allied += 1;
            } else {
                enemy += 1;
            }
            markers.push(serde_json::json!({
                "entity": hero.to_bits(), "team": format!("{team:?}"),
                "local": player.is_some(), "rect": rect_json(rect),
            }));
        }
        markers.sort_by_key(|marker| marker["entity"].as_u64());
        let mut camera_edges = Vec::new();
        for (entity, edge) in &self.edges {
            if rendered_rect(entity).is_none() {
                continue;
            }
            let (_, computed, transform, _) = self.nodes.get(entity).unwrap();
            let half = computed.size() * 0.5;
            let scale = computed.inverse_scale_factor();
            let a = transform.transform_point2(Vec2::new(-half.x, 0.0)) * scale;
            let b = transform.transform_point2(Vec2::new(half.x, 0.0)) * scale;
            camera_edges.push(serde_json::json!({
                "edge": edge.0, "a": [a.x, a.y], "b": [b.x, b.y],
            }));
        }
        camera_edges.sort_by_key(|edge| edge["edge"].as_u64());
        serde_json::json!({
            "source": "computed Bevy minimap UI nodes in logical pixels",
            "visibility_policy": "shared radial enemy hero minimap detection; no world or network fog",
            "container_rect": self.state.container.and_then(rendered_rect).map(rect_json),
            "hero_markers": {"local": local, "allied": allied, "enemy": enemy},
            "marker_rects": markers,
            "minion_markers": self.state.minion_icons.values().filter(|icon| rendered_rect(**icon).is_some()).count(),
            "camp_markers": self.camps.iter().filter_map(|(entity, camp)| {
                let rect = rendered_rect(entity)?;
                Some(serde_json::json!({"index": camp.index, "alive": camp.alive, "rect": rect_json(rect)}))
            }).collect::<Vec<_>>(),
            "structure_markers": self.state.structure_icons.values().filter(|icon| rendered_rect(**icon).is_some()).count(),
            "camera_edges": camera_edges,
            "route_segments": self.routes.iter().filter_map(|(entity, index)| {
                rendered_rect(entity)?;
                let (_, computed, transform, _) = self.nodes.get(entity).ok()?;
                let half = computed.size().x * 0.5;
                let scale = computed.inverse_scale_factor();
                let a = transform.transform_point2(Vec2::new(-half, 0.0)) * scale;
                let b = transform.transform_point2(Vec2::new(half, 0.0)) * scale;
                Some(serde_json::json!({"index":index.0,"a":[a.x,a.y],"b":[b.x,b.y]}))
            }).collect::<Vec<_>>(),
            "route_destination": self.destinations.iter().find_map(rendered_rect).map(rect_json),
        })
    }
}

fn setup_minimap_ui(
    mut commands: Commands,
    mut state: ResMut<MinimapUiState>,
    layout: Res<MapLayout>,
) {
    commands
        .spawn((
            // Block world clicks on the decorative frame as well as the map.
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(DESKTOP_MINIMAP_INSET),
                top: Val::Px(DESKTOP_MINIMAP_INSET),
                width: Val::Px(MINIMAP_SIZE),
                height: Val::Px(MINIMAP_SIZE),
                padding: UiRect::all(Val::Px(9.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(ui_theme::PANEL),
            BorderColor::all(ui_theme::EDGE),
            ZIndex(8),
            MinimapRoot,
            Name::new("MinimapRoot"),
        ))
        .with_children(|parent| {
            let mut map = parent.spawn((
                Node {
                    width: Val::Px(MINIMAP_INNER_SIZE),
                    height: Val::Px(MINIMAP_INNER_SIZE),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.055, 0.135, 0.115)),
                MinimapContainer,
                Name::new("MinimapContainer"),
            ));
            state.container = Some(map.id());
            map.with_children(|map| {
                for center in layout.jungle_block_centers() {
                    let point = map_point(*layout, Vec3::new(center.x, 0.0, center.y));
                    map.spawn((
                        marker_node(point, 32.0),
                        BackgroundColor(Color::srgb(0.07, 0.19, 0.145)),
                    ));
                }
                let river = layout.river_polyline();
                spawn_map_line(
                    map,
                    *layout,
                    river[0],
                    river[1],
                    13.0,
                    Color::srgb(0.08, 0.27, 0.29),
                );
                for lane in layout.lane_polylines() {
                    for segment in lane.windows(2) {
                        spawn_map_line(
                            map,
                            *layout,
                            segment[0],
                            segment[1],
                            5.0,
                            Color::srgb(0.40, 0.43, 0.29),
                        );
                    }
                }
                for edge in 0..4 {
                    map.spawn((
                        Node {
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(1.0, 0.85, 0.24)),
                        CameraFootprintEdge(edge),
                        ZIndex(5),
                        Name::new("MinimapCameraEdge"),
                    ));
                }
            });
        });
}

fn spawn_map_line(
    parent: &mut ChildSpawnerCommands,
    layout: MapLayout,
    a: Vec2,
    b: Vec2,
    width: f32,
    color: Color,
) {
    let a = map_point(layout, Vec3::new(a.x, 0.0, a.y));
    let b = map_point(layout, Vec3::new(b.x, 0.0, b.y));
    let (node, transform) = line_node(a, b, width);
    parent.spawn((node, transform, BackgroundColor(color)));
}
pub(crate) fn line_node(a: Vec2, b: Vec2, width: f32) -> (Node, UiTransform) {
    let delta = b - a;
    let middle = (a + b) * 0.5;
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(middle.x - delta.length() * 0.5),
            top: Val::Px(middle.y - width * 0.5),
            width: Val::Px(delta.length()),
            height: Val::Px(width),
            ..default()
        },
        UiTransform::from_rotation(Rot2::radians(delta.y.atan2(delta.x))),
    )
}
fn marker_node(point: Vec2, size: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px((point.x - size * 0.5).clamp(0.0, MINIMAP_INNER_SIZE - size)),
        top: Val::Px((point.y - size * 0.5).clamp(0.0, MINIMAP_INNER_SIZE - size)),
        width: Val::Px(size),
        height: Val::Px(size),
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn handle_minimap_navigation_system(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    containers: Query<(&ComputedNode, &UiGlobalTransform), With<MinimapContainer>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    layout: Res<MapLayout>,
    mut camera: ResMut<CameraState>,
    mut navigation: ResMut<MinimapNavigationState>,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    navigation.consumed_primary_click = false;
    navigation.movement_target = None;
    if !context.gameplay_allowed() {
        navigation.touch_id = None;
        return;
    }
    let (Ok(window), Ok((node, transform))) = (windows.single(), containers.single()) else {
        return;
    };
    let Some(rect) = container_rect(node, transform) else {
        return;
    };
    if let Some(cursor) = window.cursor_position() {
        let target = minimap_cursor_to_world(*layout, rect, cursor);
        if mouse.just_pressed(MouseButton::Right)
            && !keyboard.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        {
            navigation.movement_target = target;
        }
        if mouse.just_pressed(MouseButton::Left) && target.is_some() {
            navigation.consumed_primary_click = true;
        }
        if mouse.pressed(MouseButton::Left) {
            if let Some(target) = target {
                navigation.focus_target = Some(target);
                camera.locked = true;
            }
        }
    }
    if let Some(id) = navigation.touch_id {
        if let Some(touch) = touches.get_pressed(id) {
            if let Some(target) = minimap_cursor_to_world(*layout, rect, touch.position()) {
                navigation.focus_target = Some(target);
                camera.locked = true;
            }
        } else {
            navigation.touch_id = None;
        }
    } else {
        for touch in touches.iter_just_pressed() {
            if let Some(target) = minimap_cursor_to_world(*layout, rect, touch.position()) {
                navigation.consumed_primary_click = true;
                navigation.touch_id = Some(touch.id());
                navigation.focus_target = Some(target);
                camera.locked = true;
                break;
            }
        }
    }
}
fn container_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Option<Rect> {
    // Computed UI positions are physical pixels; input cursors are logical.
    let scale = node.inverse_scale_factor();
    let size = node.size() * transform.to_scale_angle_translation().0.abs() * scale;
    (size.min_element() > 0.0).then(|| Rect::from_center_size(transform.translation * scale, size))
}

fn update_minimap_icons_system(
    mut commands: Commands,
    layout: Res<MapLayout>,
    mut state: ResMut<MinimapUiState>,
    heroes: Query<
        (
            Entity,
            &Transform,
            &Team,
            &CombatStats,
            Option<&Player>,
            Option<&NetworkAvatar>,
            Option<&NetworkSpriteCharacter>,
            Option<&NetworkHeroClass>,
        ),
        Or<(With<Player>, With<RemotePlayer>)>,
    >,
    structures: Query<
        (Entity, &Transform, &Team, &StructureKind, &CombatStats),
        With<NetworkStructure>,
    >,
    minions: Query<(Entity, &Transform, &Team, &CombatStats), With<NetworkMinion>>,
    neutrals: Query<(&Transform, &NetworkNeutralCampType, &CombatStats), With<NetworkNeutral>>,
    thumbnails: Res<AvatarThumbnails>,
    sprites: Res<SpriteVisualAssets>,
    mode: Res<PlayerVisualMode>,
) {
    let Some(container) = state.container else {
        return;
    };
    let local_team = heroes
        .iter()
        .find(|hero| hero.4.is_some())
        .map(|hero| *hero.2);
    let mut observers = Vec::new();
    for (_, transform, team, stats, ..) in &heroes {
        if Some(*team) == local_team && stats.is_alive() {
            observers.push((transform.translation.xz(), HERO_SIGHT));
        }
    }
    for (_, transform, team, stats) in &minions {
        if Some(*team) == local_team && stats.is_alive() {
            observers.push((transform.translation.xz(), MINION_SIGHT));
        }
    }
    for (_, transform, team, kind, stats) in &structures {
        if Some(*team) == local_team && stats.is_alive() {
            observers.push((
                transform.translation.xz(),
                match kind {
                    StructureKind::Tower => TOWER_SIGHT,
                    StructureKind::BaseTower => BASE_SIGHT,
                },
            ));
        }
    }
    let mut seen = HashSet::new();
    for (entity, transform, team, stats, local, avatar, sprite, class) in &heroes {
        if !hero_marker_visible(
            local_team,
            *team,
            stats.is_alive(),
            transform.translation.xz(),
            &observers,
        ) {
            continue;
        }
        seen.insert(entity);
        let is_local = local.is_some();
        let slug = avatar.and_then(|avatar| avatar.0.as_deref());
        let sprite_id = sprite
            .and_then(|sprite| sprite.0.as_deref())
            .unwrap_or(shared::DEFAULT_SPRITE_CHARACTER_ID);
        let thumbnail = slug.and_then(|slug| thumbnails.0.get(slug));
        let key = format!(
            "{mode:?}:{team:?}:{is_local}:{slug:?}:{sprite_id}:{:?}:{:?}",
            class.map(|class| class.0),
            thumbnail.map(Handle::id),
        );
        let size = if is_local { 30.0 } else { 24.0 };
        let mut node = marker_node(map_point(*layout, transform.translation), size);
        node.border = UiRect::all(Val::Px(if is_local { 2.5 } else { 2.0 }));
        node.justify_content = JustifyContent::Center;
        node.align_items = AlignItems::Center;
        if let Some((icon, previous_key)) = state.player_icons.get(&entity) {
            if previous_key == &key {
                commands.entity(*icon).insert(node);
                continue;
            }
            commands.entity(*icon).despawn();
        }
        let mut image = if *mode == PlayerVisualMode::Sprite2d {
            let index = shared::sprite_character_roster()
                .iter()
                .position(|entry| entry.id == sprite_id)
                .unwrap_or(0);
            let (image, layout, index) = sprites.portrait(index);
            Some(ImageNode::from_atlas_image(
                image,
                TextureAtlas { layout, index },
            ))
        } else {
            thumbnail.map(|image| ImageNode::new(image.clone()))
        };
        if let Some(image) = image.as_mut() {
            image.image_mode = NodeImageMode::Stretch;
        }
        let fallback = slug
            .and_then(|slug| slug.chars().next())
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| {
                match class.map(|class| class.0) {
                    Some(shared::HeroClass::Mage) => "M",
                    Some(shared::HeroClass::Cleric) => "C",
                    Some(shared::HeroClass::Ranger) => "R",
                    _ => "W",
                }
                .to_owned()
            });
        let icon = commands
            .spawn((
                node,
                BackgroundColor(if is_local {
                    ui_theme::GOLD
                } else {
                    team_color(*team)
                }),
                BorderColor::all(if is_local {
                    ui_theme::GOLD
                } else {
                    team_color(*team)
                }),
                ZIndex(if is_local { 20 } else { 10 }),
                Name::new(if is_local {
                    "MinimapLocalPlayer"
                } else {
                    "MinimapRemotePlayer"
                }),
                ChildOf(container),
            ))
            .with_children(|parent| {
                let mut portrait = parent.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(if is_local { 2.0 } else { 0.0 })),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(ui_theme::PANEL),
                    BorderColor::all(team_color(*team)),
                    Name::new("MinimapHeroPortrait"),
                ));
                if let Some(image) = image {
                    portrait.insert(image);
                } else {
                    portrait.with_children(|portrait| {
                        portrait.spawn((
                            Text::new(fallback),
                            ui_theme::text(12.0),
                            TextColor(ui_theme::IVORY),
                        ));
                    });
                }
            })
            .id();
        state.player_icons.insert(entity, (icon, key));
    }
    state.player_icons.retain(|entity, (icon, _)| {
        if seen.contains(entity) {
            true
        } else {
            commands.entity(*icon).despawn();
            false
        }
    });
    let mut seen = HashSet::new();
    for (entity, transform, team, kind, stats) in &structures {
        if !stats.is_alive() {
            continue;
        }
        seen.insert(entity);
        let size = match kind {
            StructureKind::Tower => 8.0,
            StructureKind::BaseTower => 12.0,
        };
        sync_dot(
            &mut commands,
            container,
            &mut state.structure_icons,
            entity,
            map_point(*layout, transform.translation),
            size,
            team_color(*team),
            "MinimapStructure",
        );
    }
    despawn_removed_icons(&mut commands, &mut state.structure_icons, &seen);
    let mut seen = HashSet::new();
    for (entity, transform, team, stats) in &minions {
        if !stats.is_alive() {
            continue;
        }
        seen.insert(entity);
        sync_dot(
            &mut commands,
            container,
            &mut state.minion_icons,
            entity,
            map_point(*layout, transform.translation),
            3.5,
            team_color(*team),
            "MinimapMinion",
        );
    }
    despawn_removed_icons(&mut commands, &mut state.minion_icons, &seen);
    let camps = shared::jungle::camp_layout(layout.size().x);
    let mut living = [false; 6];
    for (transform, kind, stats) in &neutrals {
        if !stats.is_alive() || kind.0.is_boss() {
            continue;
        }
        // A roaming mob remains attached to its home marker. Matching types
        // are on opposite sides, much farther apart than the server leash.
        let closest = camps
            .iter()
            .enumerate()
            .filter(|(_, (_, camp_kind))| NeutralCampType::from(*camp_kind) == kind.0)
            .min_by(|(_, (a, _)), (_, (b, _))| {
                transform
                    .translation
                    .xz()
                    .distance_squared(Vec2::from_array(*a))
                    .total_cmp(
                        &transform
                            .translation
                            .xz()
                            .distance_squared(Vec2::from_array(*b)),
                    )
            })
            .map(|(index, _)| index);
        if let Some(index) = closest {
            living[index] = true;
        }
    }
    for (index, (anchor, kind)) in camps.into_iter().enumerate() {
        let alive = living[index];
        let color = match kind {
            shared::jungle::JungleCampKind::Skirmisher => Color::srgb(0.46, 0.96, 0.40),
            shared::jungle::JungleCampKind::Bruiser => Color::srgb(1.0, 0.71, 0.26),
            shared::jungle::JungleCampKind::Spitter => Color::srgb(0.84, 0.52, 1.0),
        };
        let mut node = marker_node(
            map_point(*layout, Vec3::new(anchor[0], 0.0, anchor[1])),
            10.0,
        );
        node.border = UiRect::all(Val::Px(2.0));
        node.border_radius = BorderRadius::all(Val::Px(5.0));
        let components = (
            node,
            BackgroundColor(if alive {
                color
            } else {
                Color::srgb(0.07, 0.10, 0.09)
            }),
            BorderColor::all(if alive {
                Color::srgb(0.10, 0.13, 0.09)
            } else {
                Color::srgb(0.47, 0.50, 0.44)
            }),
            MinimapCamp { index, alive },
        );
        if let Some(icon) = state.camp_icons[index] {
            commands.entity(icon).insert(components);
        } else {
            state.camp_icons[index] = Some(
                commands
                    .spawn((
                        components,
                        ZIndex(2),
                        Name::new(format!("MinimapCamp-{index}")),
                        ChildOf(container),
                    ))
                    .id(),
            );
        }
    }
}
fn hero_marker_visible(
    local_team: Option<Team>,
    team: Team,
    alive: bool,
    position: Vec2,
    observers: &[(Vec2, f32)],
) -> bool {
    alive
        && local_team.is_some()
        && (local_team == Some(team)
            || observers
                .iter()
                .any(|(origin, radius)| position.distance_squared(*origin) <= radius * radius))
}
fn sync_dot(
    commands: &mut Commands,
    container: Entity,
    icons: &mut HashMap<Entity, Entity>,
    world: Entity,
    point: Vec2,
    size: f32,
    color: Color,
    name: &'static str,
) {
    let node = marker_node(point, size);
    if let Some(icon) = icons.get(&world) {
        commands
            .entity(*icon)
            .insert((node, BackgroundColor(color)));
    } else {
        let icon = commands
            .spawn((
                node,
                BackgroundColor(color),
                ZIndex(2),
                Name::new(name),
                ChildOf(container),
            ))
            .id();
        icons.insert(world, icon);
    }
}
fn despawn_removed_icons(
    commands: &mut Commands,
    icons: &mut HashMap<Entity, Entity>,
    seen: &HashSet<Entity>,
) {
    icons.retain(|entity, icon| {
        if seen.contains(entity) {
            true
        } else {
            commands.entity(*icon).despawn();
            false
        }
    });
}
fn sync_minimap_visibility_for_session(
    session: Res<ClientSession>,
    mut roots: Query<&mut Node, With<MinimapRoot>>,
) {
    for mut node in &mut roots {
        node.display = if session.join_confirmed() {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Unclamped projection also serves footprint clipping; clamping its endpoints
/// independently would distort off-map camera edges.
pub(crate) fn map_point(layout: MapLayout, world: Vec3) -> Vec2 {
    let normalized = (world.xz() - layout.min) / layout.size();
    Vec2::new(normalized.y, 1.0 - normalized.x) * MINIMAP_INNER_SIZE
}
fn minimap_cursor_to_world(layout: MapLayout, rect: Rect, cursor: Vec2) -> Option<Vec3> {
    if !rect.contains(cursor) || rect.size().min_element() <= 0.0 {
        return None;
    }
    let normalized = (cursor - rect.min) / rect.size();
    let world = layout.min + Vec2::new(1.0 - normalized.y, normalized.x) * layout.size();
    Some(layout.clamp_position(Vec3::new(world.x, PLAYER_SIZE * 0.5, world.y)))
}
fn team_color(team: Team) -> Color {
    match team {
        Team::Green => Color::srgb(0.30, 0.93, 0.58),
        Team::Blue => Color::srgb(0.35, 0.65, 1.0),
    }
}
fn update_camera_footprint(
    mut commands: Commands,
    cameras: Query<(Entity, &Camera), With<MainCamera>>,
    transforms: bevy::transform::helper::TransformHelper,
    edges: Query<(Entity, &CameraFootprintEdge)>,
    layout: Res<MapLayout>,
    mode: Res<PlayerVisualMode>,
) {
    let corners = cameras.single().ok().and_then(|(entity, camera)| {
        // Bevy UI layout precedes TransformSystems::Propagate. Resolve
        // this camera's current pose directly so its footprint reaches
        // this frame's layout without a scheduling cycle or frame lag.
        let transform = transforms.compute_global_transform(entity).ok()?;
        camera_ground_corners(camera, &transform, *mode)
    });
    for (entity, edge) in &edges {
        let segment = corners.and_then(|corners| {
            clip_map_segment(
                map_point(*layout, corners[edge.0]),
                map_point(*layout, corners[(edge.0 + 1) % 4]),
            )
        });
        if let Some((a, b)) = segment {
            let (node, transform) = line_node(a, b, 1.8);
            commands.entity(entity).insert((node, transform));
        } else {
            commands.entity(entity).insert(Node {
                display: Display::None,
                ..default()
            });
        }
    }
}
fn camera_ground_corners(
    camera: &Camera,
    transform: &GlobalTransform,
    mode: PlayerVisualMode,
) -> Option<[Vec3; 4]> {
    let viewport = camera.logical_viewport_rect()?;
    let pixels = [
        viewport.min,
        Vec2::new(viewport.max.x, viewport.min.y),
        viewport.max,
        Vec2::new(viewport.min.x, viewport.max.y),
    ];
    let mut corners = [Vec3::ZERO; 4];
    for (index, pixel) in pixels.into_iter().enumerate() {
        if mode == PlayerVisualMode::Sprite2d {
            let world = camera.viewport_to_world_2d(transform, pixel).ok()?;
            corners[index] = crate::world2d::render_xy_to_simulation_xz(world, 0.0);
        } else {
            let ray = camera.viewport_to_world(transform, pixel).ok()?;
            corners[index] = ray_ground_point(ray.origin, *ray.direction)?;
        }
    }
    Some(corners)
}
fn ray_ground_point(origin: Vec3, direction: Vec3) -> Option<Vec3> {
    if direction.y >= -0.00001 {
        return None;
    }
    let distance = -origin.y / direction.y;
    let point = origin + direction * distance;
    (distance >= 0.0 && point.is_finite()).then_some(point)
}
/// Liang–Barsky clipping preserves rotated camera edges at the arena border.
fn clip_map_segment(a: Vec2, b: Vec2) -> Option<(Vec2, Vec2)> {
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let delta = b - a;
    let mut enter: f32 = 0.0;
    let mut leave: f32 = 1.0;
    for (p, q) in [
        (-delta.x, a.x),
        (delta.x, MINIMAP_INNER_SIZE - a.x),
        (-delta.y, a.y),
        (delta.y, MINIMAP_INNER_SIZE - a.y),
    ] {
        if p.abs() < 0.00001 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            enter = enter.max(q / p);
        } else {
            leave = leave.min(q / p);
        }
    }
    (enter <= leave).then_some((a + delta * enter, a + delta * leave))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_context::GameplayInputContext;

    #[test]
    fn desktop_map_keeps_click_shield_on_its_upper_left_frame() {
        let mut app = App::new();
        app.init_resource::<MapLayout>()
            .init_resource::<MinimapUiState>()
            .add_systems(Startup, setup_minimap_ui);
        app.update();
        let mut roots = app
            .world_mut()
            .query_filtered::<(&Node, Option<&Button>), With<MinimapRoot>>();
        let (root, shield) = roots.single(app.world()).unwrap();
        assert!(shield.is_some());
        assert_eq!(root.left, Val::Px(16.0));
        assert_eq!(root.top, Val::Px(16.0));
        assert_eq!(root.right, Val::Auto);
        assert_eq!(root.bottom, Val::Auto);
        assert_eq!(root.width, Val::Px(252.0));
        assert_eq!(root.height, Val::Px(252.0));
    }

    fn marker_app() -> App {
        let mut app = App::new();
        app.init_resource::<MapLayout>()
            .init_resource::<MinimapUiState>()
            .init_resource::<AvatarThumbnails>()
            .init_resource::<SpriteVisualAssets>()
            .init_resource::<PlayerVisualMode>()
            .add_systems(Update, update_minimap_icons_system);
        let container = app
            .world_mut()
            .spawn((Node::default(), MinimapContainer))
            .id();
        app.world_mut().resource_mut::<MinimapUiState>().container = Some(container);
        app
    }

    #[test]
    fn six_persistent_camp_markers_follow_authoritative_death_roaming_and_respawn() {
        let mut app = marker_app();
        let layout = *app.world().resource::<MapLayout>();
        let camps = shared::jungle::camp_layout(layout.size().x);
        app.update();
        let icons = app.world().resource::<MinimapUiState>().camp_icons;
        assert!(icons.iter().all(Option::is_some));
        for icon in icons {
            assert!(
                !app.world()
                    .entity(icon.unwrap())
                    .get::<MinimapCamp>()
                    .unwrap()
                    .alive
            );
        }
        let entities = camps.map(|(anchor, kind)| {
            app.world_mut()
                .spawn((
                    NetworkNeutral,
                    NetworkNeutralCampType(kind.into()),
                    Transform::from_xyz(anchor[0], 0.5, anchor[1]),
                    CombatStats {
                        hp: 72.0,
                        max_hp: 72.0,
                        ..default()
                    },
                ))
                .id()
        });
        app.update();
        for icon in icons {
            assert!(
                app.world()
                    .entity(icon.unwrap())
                    .get::<MinimapCamp>()
                    .unwrap()
                    .alive
            );
        }
        app.world_mut()
            .entity_mut(entities[0])
            .get_mut::<Transform>()
            .unwrap()
            .translation
            .x += 10.0;
        app.world_mut()
            .entity_mut(entities[2])
            .get_mut::<CombatStats>()
            .unwrap()
            .hp = 0.0;
        app.world_mut().despawn(entities[4]);
        app.update();
        for (index, icon) in icons.into_iter().enumerate() {
            let marker = app
                .world()
                .entity(icon.unwrap())
                .get::<MinimapCamp>()
                .unwrap();
            assert_eq!(marker.alive, index != 2 && index != 4);
        }
        let (anchor, kind) = camps[4];
        app.world_mut().spawn((
            NetworkNeutral,
            NetworkNeutralCampType(kind.into()),
            Transform::from_xyz(anchor[0], 0.5, anchor[1]),
            CombatStats {
                hp: 72.0,
                ..default()
            },
        ));
        app.update();
        assert_eq!(app.world().resource::<MinimapUiState>().camp_icons, icons);
        assert!(
            app.world()
                .entity(icons[4].unwrap())
                .get::<MinimapCamp>()
                .unwrap()
                .alive
        );
        assert_eq!(
            app.world_mut()
                .query::<&MinimapCamp>()
                .iter(app.world())
                .count(),
            6
        );
    }

    #[test]
    fn minimap_schedule_works_with_bevys_camera_layout_transform_order() {
        let mut app = App::new();
        app.init_resource::<MapLayout>()
            .init_resource::<AvatarThumbnails>()
            .init_resource::<Assets<Image>>()
            .init_resource::<SpriteVisualAssets>()
            .init_resource::<PlayerVisualMode>()
            .init_resource::<ClientSession>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Touches>()
            .init_resource::<CameraState>()
            .init_resource::<GameplayInputContext>()
            .add_plugins(MinimapPlugin)
            // This is the real Bevy UI constraint that a standalone marker
            // system test would miss: UI layout precedes transform propagation.
            .configure_sets(
                PostUpdate,
                (
                    bevy::camera::CameraUpdateSystems,
                    bevy::ui::UiSystems::Layout,
                    bevy::transform::TransformSystems::Propagate,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    (|| {}).in_set(bevy::ui::UiSystems::Layout),
                    (|| {}).in_set(bevy::transform::TransformSystems::Propagate),
                ),
            );
        app.update();
        assert!(app.world().resource::<MinimapUiState>().container.is_some());
    }

    fn spawn_hero(app: &mut App, local: bool, team: Team, position: Vec3, hp: f32) -> Entity {
        let mut entity = app.world_mut().spawn((
            team,
            Transform::from_translation(position),
            CombatStats { hp, ..default() },
            NetworkHeroClass(shared::HeroClass::Ranger),
        ));
        if local {
            entity.insert(Player);
        } else {
            entity.insert(RemotePlayer);
        }
        entity.id()
    }

    fn has_hero_marker(app: &App, hero: Entity) -> bool {
        app.world()
            .resource::<MinimapUiState>()
            .player_icons
            .contains_key(&hero)
    }

    #[test]
    fn actual_marker_system_uses_only_living_allied_observers_of_every_supported_kind() {
        for (kind, sight) in [
            ("hero", HERO_SIGHT),
            ("minion", MINION_SIGHT),
            ("tower", TOWER_SIGHT),
            ("base", BASE_SIGHT),
        ] {
            let mut app = marker_app();
            // A dead local hero still shares its living team's vision, but
            // cannot itself reveal the nearby enemy being tested.
            spawn_hero(&mut app, true, Team::Green, Vec3::ZERO, 0.0);
            let enemy = spawn_hero(
                &mut app,
                false,
                Team::Blue,
                Vec3::new(sight, 0.0, 0.0),
                100.0,
            );
            let mut observer =
                app.world_mut()
                    .spawn((Team::Green, Transform::default(), CombatStats::default()));
            match kind {
                "hero" => {
                    observer.insert(RemotePlayer);
                }
                "minion" => {
                    observer.insert(NetworkMinion);
                }
                "tower" => {
                    observer.insert((NetworkStructure, StructureKind::Tower));
                }
                "base" => {
                    observer.insert((NetworkStructure, StructureKind::BaseTower));
                }
                _ => unreachable!(),
            }
            let observer = observer.id();
            app.update();
            assert!(
                has_hero_marker(&app, enemy),
                "{kind} should reveal at its radius"
            );

            app.world_mut()
                .get_mut::<Transform>(enemy)
                .unwrap()
                .translation
                .x += 0.01;
            app.update();
            assert!(
                !has_hero_marker(&app, enemy),
                "{kind} must respect its own radius"
            );
            app.world_mut()
                .get_mut::<Transform>(enemy)
                .unwrap()
                .translation
                .x = sight;

            app.world_mut().get_mut::<CombatStats>(observer).unwrap().hp = 0.0;
            app.update();
            assert!(!has_hero_marker(&app, enemy), "dead {kind} cannot reveal");

            app.world_mut().get_mut::<CombatStats>(observer).unwrap().hp = 100.0;
            *app.world_mut().get_mut::<Team>(observer).unwrap() = Team::Blue;
            app.update();
            assert!(!has_hero_marker(&app, enemy), "enemy {kind} cannot reveal");

            *app.world_mut().get_mut::<Team>(observer).unwrap() = Team::Green;
            app.update();
            assert!(has_hero_marker(&app, enemy));
            app.world_mut().despawn(observer);
            app.update();
            assert!(
                !has_hero_marker(&app, enemy),
                "removed {kind} leaves no stale vision"
            );
        }
    }

    #[test]
    fn allies_are_shared_out_of_range_but_dead_heroes_and_missing_local_team_have_no_markers() {
        let mut app = marker_app();
        let local = spawn_hero(&mut app, true, Team::Green, Vec3::ZERO, 100.0);
        let ally = spawn_hero(&mut app, false, Team::Green, Vec3::splat(200.0), 100.0);
        let enemy = spawn_hero(&mut app, false, Team::Blue, Vec3::X, 100.0);
        app.update();
        assert!(has_hero_marker(&app, local));
        assert!(has_hero_marker(&app, ally));
        assert!(has_hero_marker(&app, enemy));
        for hero in [ally, enemy] {
            app.world_mut().get_mut::<CombatStats>(hero).unwrap().hp = 0.0;
        }
        app.update();
        assert!(has_hero_marker(&app, local));
        assert!(!has_hero_marker(&app, ally));
        assert!(!has_hero_marker(&app, enemy));
        app.world_mut().despawn(local);
        app.world_mut().get_mut::<CombatStats>(ally).unwrap().hp = 100.0;
        app.update();
        assert!(
            app.world()
                .resource::<MinimapUiState>()
                .player_icons
                .is_empty()
        );
    }

    #[test]
    fn portraits_keep_round_team_rings_local_halo_and_recover_from_missing_thumbnail() {
        let mut app = marker_app();
        let local = spawn_hero(&mut app, true, Team::Green, Vec3::ZERO, 100.0);
        let ally = spawn_hero(&mut app, false, Team::Green, Vec3::X, 100.0);
        let enemy = spawn_hero(&mut app, false, Team::Blue, Vec3::Z, 100.0);
        app.world_mut()
            .entity_mut(ally)
            .insert(NetworkAvatar(Some("agnes".to_owned())));
        app.world_mut()
            .entity_mut(enemy)
            .insert(NetworkAvatar(Some("unavailable-avatar".to_owned())));
        app.world_mut()
            .resource_mut::<AvatarThumbnails>()
            .0
            .insert("agnes".to_owned(), Handle::default());
        app.update();

        for (hero, team, is_local, has_image, fallback) in [
            (local, Team::Green, true, false, "R"),
            (ally, Team::Green, false, true, ""),
            (enemy, Team::Blue, false, false, "U"),
        ] {
            let (icon, _) = app.world().resource::<MinimapUiState>().player_icons[&hero];
            let icon = app.world().entity(icon);
            let node = icon.get::<Node>().unwrap();
            assert_eq!(node.border_radius, BorderRadius::MAX);
            assert_eq!(node.width, Val::Px(if is_local { 30.0 } else { 24.0 }));
            assert_eq!(
                *icon.get::<BorderColor>().unwrap(),
                BorderColor::all(if is_local {
                    ui_theme::GOLD
                } else {
                    team_color(team)
                })
            );
            let portrait = app.world().entity(icon.get::<Children>().unwrap()[0]);
            let portrait_node = portrait.get::<Node>().unwrap();
            assert_eq!(portrait_node.border_radius, BorderRadius::MAX);
            assert_eq!(
                *portrait.get::<BorderColor>().unwrap(),
                BorderColor::all(team_color(team))
            );
            assert_eq!(
                portrait_node.border,
                UiRect::all(Val::Px(if is_local { 2.0 } else { 0.0 }))
            );
            assert_eq!(portrait.get::<ImageNode>().is_some(), has_image);
            if !has_image {
                let text = app.world().entity(portrait.get::<Children>().unwrap()[0]);
                assert_eq!(text.get::<Text>().unwrap().0, fallback);
            }
        }

        let old_icon = app.world().resource::<MinimapUiState>().player_icons[&enemy].0;
        app.world_mut()
            .resource_mut::<AvatarThumbnails>()
            .0
            .insert("unavailable-avatar".to_owned(), Handle::default());
        app.update();
        let new_icon = app.world().resource::<MinimapUiState>().player_icons[&enemy].0;
        assert_ne!(old_icon, new_icon);
        assert!(app.world().get_entity(old_icon).is_err());
        let portrait = app.world().entity(new_icon).get::<Children>().unwrap()[0];
        assert!(app.world().entity(portrait).get::<ImageNode>().is_some());
    }

    #[test]
    fn qa_diagnostics_reports_computed_dpi_and_rotated_edges_and_excludes_hidden_nodes() {
        let mut app = marker_app();
        let local = spawn_hero(&mut app, true, Team::Green, Vec3::ZERO, 100.0);
        app.update();
        let state = app.world().resource::<MinimapUiState>();
        let container = state.container.unwrap();
        let icon = state.player_icons[&local].0;
        for (entity, size) in [(container, 464.0), (icon, 60.0)] {
            app.world_mut().entity_mut(entity).insert((
                ComputedNode {
                    size: Vec2::splat(size),
                    inverse_scale_factor: 0.5,
                    ..default()
                },
                UiGlobalTransform::from_translation(Vec2::new(600.0, 800.0)),
                // This fixture supplies computed layout without the render
                // visibility plugin; mirror its resolved visible state too.
                InheritedVisibility::VISIBLE,
            ));
        }
        let edge = app
            .world_mut()
            .spawn((
                Node::default(),
                ComputedNode {
                    size: Vec2::new(80.0, 2.0),
                    inverse_scale_factor: 0.5,
                    ..default()
                },
                UiGlobalTransform::from(bevy::math::Affine2::from_angle_translation(
                    std::f32::consts::FRAC_PI_2,
                    Vec2::new(300.0, 400.0),
                )),
                CameraFootprintEdge(0),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let mut diagnostics =
            bevy::ecs::system::SystemState::<MinimapQaScene>::new(app.world_mut());
        let summary = diagnostics.get(app.world()).diagnostics();
        assert_eq!(
            summary["container_rect"],
            serde_json::json!([184.0, 284.0, 416.0, 516.0])
        );
        assert_eq!(summary["hero_markers"]["local"], 1);
        assert_eq!(
            summary["camera_edges"][0]["a"],
            serde_json::json!([150.0, 180.0])
        );
        assert_eq!(
            summary["camera_edges"][0]["b"],
            serde_json::json!([150.0, 220.0])
        );
        app.world_mut().get_mut::<Node>(edge).unwrap().display = Display::None;
        app.world_mut().get_mut::<Node>(icon).unwrap().display = Display::None;
        let summary = diagnostics.get(app.world()).diagnostics();
        assert_eq!(summary["hero_markers"]["local"], 0);
        assert_eq!(summary["camera_edges"], serde_json::json!([]));
    }

    #[test]
    fn map_projection_and_clicks_round_trip_at_upper_left_and_phone_scales() {
        let layout = MapLayout::default();
        for (inset, scale) in [
            (Vec2::splat(26.0), 1.0),
            (Vec2::new(37.0, 17.0), 132.0 / 252.0),
        ] {
            let rect = Rect::from_corners(inset, inset + Vec2::splat(MINIMAP_INNER_SIZE * scale));
            for world in [
                layout.home_spawn,
                layout.away_spawn,
                Vec3::ZERO,
                Vec3::new(-20.0, 0.0, 50.0),
            ] {
                let result = minimap_cursor_to_world(
                    layout,
                    rect,
                    rect.min + map_point(layout, world) * scale,
                )
                .unwrap();
                assert!(result.xz().distance(world.xz()) < 0.001);
            }
            // Decorative frame clicks must not become map orders.
            assert!(minimap_cursor_to_world(layout, rect, rect.min - Vec2::X).is_none());
            for viewport in [
                Vec2::new(960.0, 540.0),
                Vec2::new(1280.0, 720.0),
                Vec2::new(1600.0, 1000.0),
            ] {
                let old_lower_right = viewport - Vec2::splat(142.0);
                assert!(minimap_cursor_to_world(layout, rect, old_lower_right).is_none());
            }
        }
        let home = map_point(layout, layout.home_spawn);
        let away = map_point(layout, layout.away_spawn);
        assert!(home.x < away.x && home.y > away.y);
    }
    #[test]
    fn shared_detection_reveals_enemy_only_in_living_allied_observer_range() {
        let observers = [
            (Vec2::ZERO, HERO_SIGHT),
            (Vec2::new(80.0, 0.0), MINION_SIGHT),
        ];
        assert!(hero_marker_visible(
            Some(Team::Green),
            Team::Green,
            true,
            Vec2::splat(200.0),
            &[]
        ));
        assert!(hero_marker_visible(
            Some(Team::Green),
            Team::Blue,
            true,
            Vec2::new(32.0, 0.0),
            &observers
        ));
        assert!(!hero_marker_visible(
            Some(Team::Green),
            Team::Blue,
            true,
            Vec2::new(32.1, 0.0),
            &observers
        ));
        assert!(hero_marker_visible(
            Some(Team::Green),
            Team::Blue,
            true,
            Vec2::new(100.0, 0.0),
            &observers
        ));
        assert!(!hero_marker_visible(
            Some(Team::Green),
            Team::Blue,
            true,
            Vec2::ZERO,
            &[]
        ));
        assert!(!hero_marker_visible(
            Some(Team::Green),
            Team::Green,
            false,
            Vec2::ZERO,
            &observers
        ));
        assert!(!hero_marker_visible(
            None,
            Team::Green,
            true,
            Vec2::ZERO,
            &observers
        ));
    }
    #[test]
    fn camera_edges_preserve_ground_projection_and_clip_without_distortion() {
        let origin = Vec3::new(-24.0, 28.0, 0.0);
        assert!(
            ray_ground_point(origin, -origin.normalize())
                .unwrap()
                .length()
                < 0.001
        );
        assert!(ray_ground_point(origin, Vec3::X).is_none());
        assert!(ray_ground_point(origin, Vec3::Y).is_none());
        let (a, b) = clip_map_segment(Vec2::new(-20.0, 10.0), Vec2::new(20.0, 50.0)).unwrap();
        assert_eq!(a, Vec2::new(0.0, 30.0));
        assert_eq!(b, Vec2::new(20.0, 50.0));
        assert!(clip_map_segment(Vec2::new(-20.0, 0.0), Vec2::new(-10.0, 20.0)).is_none());
        assert!(clip_map_segment(Vec2::splat(f32::NAN), Vec2::ZERO).is_none());
    }
    #[test]
    fn high_dpi_computed_bounds_use_logical_cursor_coordinates() {
        let node = ComputedNode {
            size: Vec2::splat(464.0),
            inverse_scale_factor: 0.5,
            ..default()
        };
        let transform = UiGlobalTransform::from_translation(Vec2::splat(284.0));
        let rect = container_rect(&node, &transform).unwrap();
        assert_eq!(rect.size(), Vec2::splat(232.0));
        assert_eq!(rect.min, Vec2::splat(26.0));
        assert_eq!(rect.center(), Vec2::splat(142.0));
    }
    #[test]
    fn right_click_emits_one_move_without_panning_and_alt_does_not_reuse_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Touches>()
            .init_resource::<MapLayout>()
            .init_resource::<CameraState>()
            .init_resource::<MinimapNavigationState>()
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, handle_minimap_navigation_system);
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::splat(142.0)));
        let window_entity = app
            .world_mut()
            .spawn((window, bevy::window::PrimaryWindow))
            .id();
        app.world_mut().spawn((
            MinimapContainer,
            ComputedNode {
                size: Vec2::splat(464.0),
                inverse_scale_factor: 0.5,
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::splat(284.0)),
        ));
        app.world_mut().resource_mut::<CameraState>().locked = false;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        let nav = app.world().resource::<MinimapNavigationState>();
        assert_eq!(nav.movement_target.unwrap().xz(), Vec2::ZERO);
        assert!(nav.focus_target.is_none());
        assert!(!app.world().resource::<CameraState>().locked);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .movement_target
                .is_none()
        );
        for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .reset_all();
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Right);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(alt);
            app.update();
            assert!(
                app.world()
                    .resource::<MinimapNavigationState>()
                    .movement_target
                    .is_none()
            );
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
        }
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(20.0, 142.0)));
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .movement_target
                .is_none()
        );
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::splat(142.0)));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .focus_target
                .is_some()
        );
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .movement_target
                .is_none()
        );
        assert!(app.world().resource::<CameraState>().locked);
    }

    #[test]
    fn minimap_click_does_not_escape_modal_or_debug_flight() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Touches>()
            .init_resource::<MapLayout>()
            .init_resource::<CameraState>()
            .init_resource::<MinimapNavigationState>()
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, handle_minimap_navigation_system);
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::splat(142.0)));
        app.world_mut().spawn((window, bevy::window::PrimaryWindow));
        app.world_mut().spawn((
            MinimapContainer,
            ComputedNode {
                size: Vec2::splat(232.0),
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::splat(142.0)),
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = true;
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .focus_target
                .is_none()
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .debug_flight = true;
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .focus_target
                .is_none()
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .debug_flight = false;
        app.update();
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .focus_target
                .is_some()
        );
        assert!(
            app.world()
                .resource::<MinimapNavigationState>()
                .consumed_primary_click
        );
    }
}
