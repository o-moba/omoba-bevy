use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::camera::CameraState;
use crate::combat::CombatStats;
use crate::maps::MapLayout;
use crate::net::{ClientSession, NetworkMinion, NetworkStructure, RemotePlayer, StructureKind};
use crate::player::PLAYER_SIZE;
use crate::player::Player;
use crate::team::Team;

const MINIMAP_MARGIN: f32 = 16.0;
const MINIMAP_SIZE: f32 = 220.0;
const MINIMAP_INNER_SIZE: f32 = 200.0;
const MINIMAP_PADDING: f32 = (MINIMAP_SIZE - MINIMAP_INNER_SIZE) * 0.5;

const PLAYER_ICON_SIZE: f32 = 6.0;
const MINION_ICON_SIZE: f32 = 3.0;
const TOWER_ICON_SIZE: f32 = 8.0;
const BASE_ICON_SIZE: f32 = 12.0;

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapUiState>()
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
                    sync_minimap_visibility_for_session.after(update_minimap_icons_system),
                ),
            );
    }
}

#[derive(Resource, Default)]
pub struct MinimapNavigationState {
    pub focus_target: Option<Vec3>,
    pub consumed_primary_click: bool,
}

#[derive(Resource, Default)]
struct MinimapUiState {
    container: Option<Entity>,
    player_icons: HashMap<Entity, Entity>,
    structure_icons: HashMap<Entity, Entity>,
    minion_icons: HashMap<Entity, Entity>,
}

#[derive(Component)]
struct MinimapRoot;

#[derive(Component)]
struct MinimapContainer;

#[derive(Component)]
struct MinimapIcon;

fn setup_minimap_ui(mut commands: Commands, mut state: ResMut<MinimapUiState>) {
    let mut container_entity = None;

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(MINIMAP_MARGIN),
                top: Val::Px(MINIMAP_MARGIN),
                width: Val::Px(MINIMAP_SIZE),
                height: Val::Px(MINIMAP_SIZE),
                padding: UiRect::all(Val::Px(MINIMAP_PADDING)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.07, 0.80)),
            BorderColor::all(Color::srgba(0.73, 0.78, 0.88, 0.65)),
            MinimapRoot,
            Name::new("MinimapRoot"),
        ))
        .with_children(|parent| {
            let container = parent
                .spawn((
                    Node {
                        position_type: PositionType::Relative,
                        width: Val::Px(MINIMAP_INNER_SIZE),
                        height: Val::Px(MINIMAP_INNER_SIZE),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.08, 0.12, 0.10, 0.95)),
                    MinimapContainer,
                    Name::new("MinimapContainer"),
                ))
                .id();
            container_entity = Some(container);
        });

    state.container = container_entity;
}

fn handle_minimap_navigation_system(
    window_query: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    map_layout: Res<MapLayout>,
    mut cam_state: ResMut<CameraState>,
    mut nav_state: ResMut<MinimapNavigationState>,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    nav_state.consumed_primary_click = false;
    if !context.gameplay_allowed() {
        return;
    }

    let Ok(window) = window_query.single() else {
        return;
    };

    if let Some(cursor) = window.cursor_position() {
        let world_target = minimap_cursor_to_world(*map_layout, cursor);
        if mouse_input.just_pressed(MouseButton::Left) && world_target.is_some() {
            nav_state.consumed_primary_click = true;
        }
        if mouse_input.pressed(MouseButton::Left) {
            if let Some(target) = world_target {
                nav_state.focus_target = Some(target);
                cam_state.locked = true;
            }
        }
    }

    for touch in touches.iter_just_pressed() {
        if minimap_cursor_to_world(*map_layout, touch.position()).is_some() {
            nav_state.consumed_primary_click = true;
        }
    }
    for touch in touches.iter() {
        if let Some(target) = minimap_cursor_to_world(*map_layout, touch.position()) {
            nav_state.focus_target = Some(target);
            cam_state.locked = true;
        }
    }
}

fn update_minimap_icons_system(
    mut commands: Commands,
    map_layout: Res<MapLayout>,
    mut state: ResMut<MinimapUiState>,
    local_players: Query<(Entity, &Transform, &Team, &CombatStats), With<Player>>,
    remote_players: Query<
        (Entity, &Transform, &Team, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
    structures: Query<
        (Entity, &Transform, &Team, &StructureKind, &CombatStats),
        With<NetworkStructure>,
    >,
    minions: Query<(Entity, &Transform, &Team, &CombatStats), With<NetworkMinion>>,
) {
    let Some(container) = state.container else {
        return;
    };
    let map_layout = *map_layout;

    let mut seen_players = HashSet::new();
    for (entity, transform, team, stats) in local_players.iter() {
        if !stats.is_alive() {
            continue;
        }
        seen_players.insert(entity);
        sync_minimap_icon(
            &mut commands,
            container,
            &mut state.player_icons,
            entity,
            transform.translation,
            PLAYER_ICON_SIZE,
            player_color(*team, true),
            map_layout,
            "MinimapLocalPlayer",
        );
    }
    for (entity, transform, team, stats) in remote_players.iter() {
        if !stats.is_alive() {
            continue;
        }
        seen_players.insert(entity);
        sync_minimap_icon(
            &mut commands,
            container,
            &mut state.player_icons,
            entity,
            transform.translation,
            PLAYER_ICON_SIZE,
            player_color(*team, false),
            map_layout,
            "MinimapRemotePlayer",
        );
    }
    despawn_removed_icons(&mut commands, &mut state.player_icons, &seen_players);

    let mut seen_structures = HashSet::new();
    for (entity, transform, team, kind, stats) in structures.iter() {
        if !stats.is_alive() {
            continue;
        }
        seen_structures.insert(entity);
        let icon_size = match kind {
            StructureKind::Tower => TOWER_ICON_SIZE,
            StructureKind::BaseTower => BASE_ICON_SIZE,
        };
        sync_minimap_icon(
            &mut commands,
            container,
            &mut state.structure_icons,
            entity,
            transform.translation,
            icon_size,
            structure_color(*team, *kind),
            map_layout,
            "MinimapStructure",
        );
    }
    despawn_removed_icons(&mut commands, &mut state.structure_icons, &seen_structures);

    let mut seen_minions = HashSet::new();
    for (entity, transform, team, stats) in minions.iter() {
        if !stats.is_alive() {
            continue;
        }
        seen_minions.insert(entity);
        sync_minimap_icon(
            &mut commands,
            container,
            &mut state.minion_icons,
            entity,
            transform.translation,
            MINION_ICON_SIZE,
            minion_color(*team),
            map_layout,
            "MinimapMinion",
        );
    }
    despawn_removed_icons(&mut commands, &mut state.minion_icons, &seen_minions);
}

fn sync_minimap_visibility_for_session(
    client_session: Res<ClientSession>,
    mut roots: Query<&mut Visibility, With<MinimapRoot>>,
) {
    let vis = if client_session.is_connected() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in &mut roots {
        *v = vis;
    }
}

fn sync_minimap_icon(
    commands: &mut Commands,
    container: Entity,
    icon_map: &mut HashMap<Entity, Entity>,
    world_entity: Entity,
    world_pos: Vec3,
    icon_size: f32,
    color: Color,
    map_layout: MapLayout,
    icon_name: &str,
) {
    let (left, top) = world_to_minimap(map_layout, world_pos, icon_size);

    if let Some(icon_entity) = icon_map.get(&world_entity).copied() {
        let mut icon_commands = commands.entity(icon_entity);
        icon_commands.insert((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                width: Val::Px(icon_size),
                height: Val::Px(icon_size),
                ..default()
            },
            BackgroundColor(color),
            Visibility::Visible,
        ));
        return;
    }

    let mut created = None;
    commands.entity(container).with_children(|parent| {
        let icon = parent.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                width: Val::Px(icon_size),
                height: Val::Px(icon_size),
                ..default()
            },
            BackgroundColor(color),
            MinimapIcon,
            Name::new(icon_name.to_owned()),
        ));
        created = Some(icon.id());
    });

    if let Some(icon_entity) = created {
        icon_map.insert(world_entity, icon_entity);
    }
}

fn despawn_removed_icons(
    commands: &mut Commands,
    icon_map: &mut HashMap<Entity, Entity>,
    seen: &HashSet<Entity>,
) {
    let stale_entities = icon_map
        .keys()
        .copied()
        .filter(|entity| !seen.contains(entity))
        .collect::<Vec<_>>();

    for stale_entity in stale_entities {
        if let Some(icon_entity) = icon_map.remove(&stale_entity) {
            commands.entity(icon_entity).despawn();
        }
    }
}

fn world_to_minimap(layout: MapLayout, world_pos: Vec3, icon_size: f32) -> (f32, f32) {
    let map_size = layout.size();
    let normalized_x = ((world_pos.x - layout.min.x) / map_size.x.max(0.001)).clamp(0.0, 1.0);
    let normalized_z = ((world_pos.z - layout.min.y) / map_size.y.max(0.001)).clamp(0.0, 1.0);

    // Base orientation chosen to match team side placement and movement direction.
    let base_x = 1.0 - normalized_x;
    let base_y = 1.0 - normalized_z;

    // Rotate minimap projection 90 degrees clockwise around the minimap center.
    let theta = std::f32::consts::FRAC_PI_2;
    let cos_t = theta.cos();
    let sin_t = theta.sin();

    let centered_x = base_x - 0.5;
    let centered_y_up = 0.5 - base_y;
    let rotated_x = centered_x * cos_t + centered_y_up * sin_t;
    let rotated_y_up = -centered_x * sin_t + centered_y_up * cos_t;

    let rotated_x_norm = rotated_x + 0.5;
    let rotated_y_norm = 0.5 - rotated_y_up;

    let left = (rotated_x_norm * MINIMAP_INNER_SIZE - icon_size * 0.5)
        .clamp(0.0, MINIMAP_INNER_SIZE - icon_size);
    let top = (rotated_y_norm * MINIMAP_INNER_SIZE - icon_size * 0.5)
        .clamp(0.0, MINIMAP_INNER_SIZE - icon_size);

    (left, top)
}

fn minimap_cursor_to_world(layout: MapLayout, cursor_pos: Vec2) -> Option<Vec3> {
    let inner_left = MINIMAP_MARGIN + MINIMAP_PADDING;
    let inner_top = MINIMAP_MARGIN + MINIMAP_PADDING;
    let inner_right = inner_left + MINIMAP_INNER_SIZE;
    let inner_bottom = inner_top + MINIMAP_INNER_SIZE;

    if cursor_pos.x < inner_left
        || cursor_pos.x > inner_right
        || cursor_pos.y < inner_top
        || cursor_pos.y > inner_bottom
    {
        return None;
    }

    let rotated_x_norm = ((cursor_pos.x - inner_left) / MINIMAP_INNER_SIZE).clamp(0.0, 1.0);
    let rotated_y_norm = ((cursor_pos.y - inner_top) / MINIMAP_INNER_SIZE).clamp(0.0, 1.0);

    let rotated_x = rotated_x_norm - 0.5;
    let rotated_y_up = 0.5 - rotated_y_norm;

    let theta = std::f32::consts::FRAC_PI_2;
    let cos_t = theta.cos();
    let sin_t = theta.sin();
    let centered_x = rotated_x * cos_t - rotated_y_up * sin_t;
    let centered_y_up = rotated_x * sin_t + rotated_y_up * cos_t;

    let base_x = centered_x + 0.5;
    let base_y = 0.5 - centered_y_up;
    let normalized_x = (1.0 - base_x).clamp(0.0, 1.0);
    let normalized_z = (1.0 - base_y).clamp(0.0, 1.0);

    let map_size = layout.size();
    let world_x = layout.min.x + normalized_x * map_size.x;
    let world_z = layout.min.y + normalized_z * map_size.y;
    let clamped = layout.clamp_position(Vec3::new(world_x, PLAYER_SIZE * 0.5, world_z));
    Some(clamped)
}

fn player_color(team: Team, is_local: bool) -> Color {
    match (team, is_local) {
        (Team::Green, true) => Color::srgba(0.45, 1.0, 0.55, 1.0),
        (Team::Blue, true) => Color::srgba(0.50, 0.74, 1.0, 1.0),
        (Team::Green, false) => Color::srgba(0.18, 0.86, 0.30, 0.95),
        (Team::Blue, false) => Color::srgba(0.24, 0.50, 0.96, 0.95),
    }
}

fn minion_color(team: Team) -> Color {
    match team {
        Team::Green => Color::srgba(0.30, 0.84, 0.34, 0.92),
        Team::Blue => Color::srgba(0.35, 0.59, 0.98, 0.92),
    }
}

fn structure_color(team: Team, kind: StructureKind) -> Color {
    match (team, kind) {
        (Team::Green, StructureKind::Tower) => Color::srgba(0.22, 0.72, 0.30, 0.95),
        (Team::Blue, StructureKind::Tower) => Color::srgba(0.26, 0.47, 0.88, 0.95),
        (Team::Green, StructureKind::BaseTower) => Color::srgba(0.66, 1.0, 0.64, 1.0),
        (Team::Blue, StructureKind::BaseTower) => Color::srgba(0.70, 0.84, 1.0, 1.0),
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::input_context::GameplayInputContext;

    #[test]
    fn minimap_click_does_not_escape_modal_or_debug_flight() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Touches>()
            .init_resource::<MapLayout>()
            .init_resource::<CameraState>()
            .init_resource::<MinimapNavigationState>()
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, handle_minimap_navigation_system);
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::new(120.0, 120.0)));
        app.world_mut().spawn((window, bevy::window::PrimaryWindow));
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
