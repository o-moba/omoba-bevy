use bevy::{
    input::mouse::MouseButton,
    math::{Dir3, primitives::InfinitePlane3d},
    prelude::*,
    window::PrimaryWindow,
};

use crate::camera::MainCamera;
use crate::net::{NetworkCommand, NetworkPlayerId, RemotePlayer};
use crate::player::Player;

pub const MAX_HP: f32 = 100.0;
pub const MAX_MANA: f32 = 100.0;
pub const SPELL_MANA_COST: f32 = 20.0;

const BAR_WIDTH: f32 = 1.45;
const BAR_HEIGHT: f32 = 0.09;
const BAR_DEPTH: f32 = 0.09;
const TARGET_PICK_RADIUS: f32 = 4.0;
const TARGET_MARKER_SIZE: f32 = 2.0;
const TARGET_MARKER_THICKNESS: f32 = 0.08;
const TARGET_MARKER_Y: f32 = 0.24;
const SKILL_BUTTON_SIZE: f32 = 80.0;
const SKILL_BUTTON_MARGIN: f32 = 20.0;
const SKILL_BUTTON_COLOR: Color = Color::srgba(0.12, 0.12, 0.12, 0.75);
const SKILL_BUTTON_HOVER_COLOR: Color = Color::srgba(0.18, 0.18, 0.18, 0.85);
const SKILL_BUTTON_PRESS_COLOR: Color = Color::srgba(0.28, 0.28, 0.28, 0.95);

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TargetState>()
            .add_systems(Startup, setup_combat_visual_assets)
            .add_systems(Startup, setup_combat_ui)
            .add_systems(
                Update,
                (
                    select_target_system,
                    clear_invalid_target_system,
                    auto_select_target_system,
                    cast_spell_system,
                    skill_button_system,
                    update_target_marker_system,
                )
                    .chain(),
            );
        app.add_systems(
            Update,
            (spawn_combat_bars_system, update_combat_bars_system),
        );
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct CombatStats {
    pub hp: f32,
    pub max_hp: f32,
    pub mana: f32,
    pub max_mana: f32,
}

impl Default for CombatStats {
    fn default() -> Self {
        Self {
            hp: MAX_HP,
            max_hp: MAX_HP,
            mana: MAX_MANA,
            max_mana: MAX_MANA,
        }
    }
}

impl CombatStats {
    pub fn is_alive(self) -> bool {
        self.hp > 0.0
    }
}

#[derive(Resource)]
struct CombatVisualAssets {
    hp_bg_material: Handle<StandardMaterial>,
    hp_fill_material: Handle<StandardMaterial>,
    mana_bg_material: Handle<StandardMaterial>,
    mana_fill_material: Handle<StandardMaterial>,
    bar_mesh: Handle<Mesh>,
}

#[derive(Resource, Default)]
struct TargetState {
    selected_entity: Option<Entity>,
    selected_player_id: Option<u64>,
    marker_entity: Option<Entity>,
}

#[derive(Component, Default)]
struct CombatBars {
    hp_fill: Option<Entity>,
    mana_fill: Option<Entity>,
}

#[derive(Component)]
struct TargetMarker;

#[derive(Component)]
struct SkillButton;

fn setup_combat_visual_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let bar_mesh = meshes.add(Mesh::from(Cuboid::new(BAR_WIDTH, BAR_HEIGHT, BAR_DEPTH)));
    let marker_mesh = meshes.add(Mesh::from(Cuboid::new(
        TARGET_MARKER_SIZE,
        TARGET_MARKER_THICKNESS,
        TARGET_MARKER_SIZE,
    )));

    let hp_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.02, 0.02),
        perceptual_roughness: 1.0,
        ..default()
    });
    let hp_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.15, 0.18),
        perceptual_roughness: 0.9,
        ..default()
    });
    let mana_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.04, 0.06, 0.18),
        perceptual_roughness: 1.0,
        ..default()
    });
    let mana_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.52, 0.95),
        perceptual_roughness: 0.6,
        ..default()
    });
    let target_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 0.84, 0.24, 0.45),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    let marker_entity = commands
        .spawn((
            Mesh3d(marker_mesh.clone()),
            MeshMaterial3d(target_material.clone()),
            Transform::from_xyz(0.0, -50.0, 0.0),
            Visibility::Hidden,
            TargetMarker,
            Name::new("TargetMarker"),
        ))
        .id();

    commands.insert_resource(CombatVisualAssets {
        hp_bg_material,
        hp_fill_material,
        mana_bg_material,
        mana_fill_material,
        bar_mesh,
    });
    commands.insert_resource(TargetState {
        marker_entity: Some(marker_entity),
        ..default()
    });
}

fn setup_combat_ui(mut commands: Commands) {
    commands.spawn((
        Button,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(SKILL_BUTTON_MARGIN),
            bottom: Val::Px(SKILL_BUTTON_MARGIN),
            width: Val::Px(SKILL_BUTTON_SIZE),
            height: Val::Px(SKILL_BUTTON_SIZE),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(SKILL_BUTTON_COLOR),
        SkillButton,
        Name::new("SkillButton"),
    ));
}

fn select_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    local_player: Query<&Transform, With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    candidates: Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
    mut target_state: ResMut<TargetState>,
) {
    let Ok(local_transform) = local_player.get_single() else {
        return;
    };

    let mut select_entity: Option<(Entity, u64)> = None;
    if keyboard_input.just_pressed(KeyCode::Tab) {
        select_entity = candidates
            .iter()
            .filter(|(_, _, _, stats)| stats.is_alive())
            .min_by(|(_, a_transform, _, _), (_, b_transform, _, _)| {
                let a_dist = a_transform
                    .translation
                    .distance_squared(local_transform.translation);
                let b_dist = b_transform
                    .translation
                    .distance_squared(local_transform.translation);
                a_dist.total_cmp(&b_dist)
            })
            .map(|(entity, _, id, _)| (entity, id.0));
    }

    if mouse_input.just_pressed(MouseButton::Middle) {
        let Ok(window) = window_query.get_single() else {
            return;
        };
        let Ok((camera, camera_transform)) = camera_query.get_single() else {
            return;
        };
        let Some(cursor_pos) = window.cursor_position() else {
            return;
        };
        let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
            return;
        };
        let Ok(plane_normal) = Dir3::new(Vec3::Y) else {
            return;
        };
        let plane = InfinitePlane3d::new(plane_normal);
        let Some(distance) = ray.intersect_plane(Vec3::ZERO, plane) else {
            return;
        };
        let click_point = ray.get_point(distance);

        select_entity = candidates
            .iter()
            .filter(|(_, _, _, stats)| stats.is_alive())
            .filter_map(|(entity, transform, player_id, _)| {
                let planar_dist = transform.translation.xz().distance(click_point.xz());
                if planar_dist <= TARGET_PICK_RADIUS {
                    Some((entity, player_id.0, planar_dist))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(entity, id, _)| (entity, id));
    }

    if let Some((entity, player_id)) = select_entity {
        target_state.selected_entity = Some(entity);
        target_state.selected_player_id = Some(player_id);
        info!("Target selected: player_id={player_id}");
    }
}

fn clear_invalid_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut target_state: ResMut<TargetState>,
    combat_stats_query: Query<&CombatStats>,
) {
    if keyboard_input.just_pressed(KeyCode::Backspace) {
        target_state.selected_entity = None;
        target_state.selected_player_id = None;
        return;
    }

    if let Some(entity) = target_state.selected_entity {
        let Ok(stats) = combat_stats_query.get(entity) else {
            target_state.selected_entity = None;
            target_state.selected_player_id = None;
            return;
        };
        if !stats.is_alive() {
            target_state.selected_entity = None;
            target_state.selected_player_id = None;
        }
    }
}

fn auto_select_target_system(
    local_player: Query<&Transform, With<Player>>,
    candidates: Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
    mut target_state: ResMut<TargetState>,
) {
    if target_state.selected_entity.is_some() {
        return;
    }
    let Ok(local_transform) = local_player.get_single() else {
        return;
    };

    let selected = candidates
        .iter()
        .filter(|(_, _, _, stats)| stats.is_alive())
        .min_by(|(_, a_transform, _, _), (_, b_transform, _, _)| {
            let a_dist = a_transform
                .translation
                .distance_squared(local_transform.translation);
            let b_dist = b_transform
                .translation
                .distance_squared(local_transform.translation);
            a_dist.total_cmp(&b_dist)
        })
        .map(|(entity, _, id, _)| (entity, id.0));

    if let Some((entity, id)) = selected {
        target_state.selected_entity = Some(entity);
        target_state.selected_player_id = Some(id);
    }
}

fn cast_spell_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    local_stats_query: Query<&CombatStats, With<Player>>,
    local_player: Query<&Transform, With<Player>>,
    candidates: Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
    mut target_state: ResMut<TargetState>,
    mut command_writer: EventWriter<NetworkCommand>,
) {
    if !keyboard_input.just_pressed(KeyCode::KeyQ) {
        return;
    }

    let Ok(local_stats) = local_stats_query.get_single() else {
        return;
    };
    if local_stats.mana < SPELL_MANA_COST {
        info!(
            "Not enough mana to cast. Need {:.0}, current {:.0}",
            SPELL_MANA_COST, local_stats.mana
        );
        return;
    };
    let target_id = resolve_cast_target(&mut target_state, local_player.get_single().ok(), &candidates);
    if let Some(target_id) = target_id {
        command_writer.send(NetworkCommand::Cast { target_id });
    } else {
        info!("No target available. Use TAB or middle mouse click to select.");
    }
}

fn skill_button_system(
    mut interactions: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SkillButton>),
    >,
    local_stats_query: Query<&CombatStats, With<Player>>,
    local_player: Query<&Transform, With<Player>>,
    candidates: Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
    mut target_state: ResMut<TargetState>,
    mut command_writer: EventWriter<NetworkCommand>,
) {
    for (interaction, mut color) in interactions.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = SKILL_BUTTON_PRESS_COLOR.into();
                let Ok(local_stats) = local_stats_query.get_single() else {
                    continue;
                };
                if local_stats.mana < SPELL_MANA_COST {
                    continue;
                }
                if let Some(target_id) =
                    resolve_cast_target(&mut target_state, local_player.get_single().ok(), &candidates)
                {
                    command_writer.send(NetworkCommand::Cast { target_id });
                }
            }
            Interaction::Hovered => {
                *color = SKILL_BUTTON_HOVER_COLOR.into();
            }
            Interaction::None => {
                *color = SKILL_BUTTON_COLOR.into();
            }
        }
    }
}

fn spawn_combat_bars_system(
    mut commands: Commands,
    assets: Res<CombatVisualAssets>,
    players_without_bars: Query<Entity, (With<CombatStats>, Without<CombatBars>)>,
) {
    for entity in players_without_bars.iter() {
        let mut bars = CombatBars::default();
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Mesh3d(assets.bar_mesh.clone()),
                MeshMaterial3d(assets.hp_bg_material.clone()),
                Transform::from_xyz(0.0, 2.1, 0.0),
                Name::new("HpBarBg"),
            ));

            let hp_fill = parent
                .spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.hp_fill_material.clone()),
                    Transform::from_xyz(0.0, 2.1, 0.06),
                    Name::new("HpBarFill"),
                ))
                .id();

            parent.spawn((
                Mesh3d(assets.bar_mesh.clone()),
                MeshMaterial3d(assets.mana_bg_material.clone()),
                Transform::from_xyz(0.0, 1.95, 0.0),
                Name::new("ManaBarBg"),
            ));

            let mana_fill = parent
                .spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.mana_fill_material.clone()),
                    Transform::from_xyz(0.0, 1.95, 0.06),
                    Name::new("ManaBarFill"),
                ))
                .id();

            bars.hp_fill = Some(hp_fill);
            bars.mana_fill = Some(mana_fill);
        });
        commands.entity(entity).insert(bars);
    }
}

fn update_combat_bars_system(
    owners: Query<(&CombatStats, &CombatBars)>,
    mut transforms: Query<&mut Transform>,
) {
    for (stats, bars) in owners.iter() {
        let hp_ratio = if stats.max_hp > 0.0 {
            (stats.hp / stats.max_hp).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mana_ratio = if stats.max_mana > 0.0 {
            (stats.mana / stats.max_mana).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if let Some(hp_fill) = bars.hp_fill {
            if let Ok(mut transform) = transforms.get_mut(hp_fill) {
                transform.scale.x = hp_ratio.max(0.001);
                transform.translation.x = (hp_ratio - 1.0) * BAR_WIDTH * 0.5;
            }
        }

        if let Some(mana_fill) = bars.mana_fill {
            if let Ok(mut transform) = transforms.get_mut(mana_fill) {
                transform.scale.x = mana_ratio.max(0.001);
                transform.translation.x = (mana_ratio - 1.0) * BAR_WIDTH * 0.5;
            }
        }
    }
}

fn update_target_marker_system(
    time: Res<Time>,
    target_state: Res<TargetState>,
    targets: Query<&Transform, Without<TargetMarker>>,
    mut marker_query: Query<(&mut Transform, &mut Visibility), With<TargetMarker>>,
) {
    let Some(marker_entity) = target_state.marker_entity else {
        return;
    };
    let Ok((mut marker_transform, mut marker_visibility)) = marker_query.get_mut(marker_entity)
    else {
        return;
    };

    let Some(target_entity) = target_state.selected_entity else {
        *marker_visibility = Visibility::Hidden;
        return;
    };
    let Ok(target_transform) = targets.get(target_entity) else {
        *marker_visibility = Visibility::Hidden;
        return;
    };

    *marker_visibility = Visibility::Visible;
    marker_transform.translation = Vec3::new(
        target_transform.translation.x,
        TARGET_MARKER_Y,
        target_transform.translation.z,
    );
    let pulse = 1.0 + 0.08 * (time.elapsed_secs() * 7.5).sin();
    marker_transform.scale = Vec3::new(pulse, 1.0, pulse);
}

fn resolve_cast_target(
    target_state: &mut TargetState,
    local_transform: Option<&Transform>,
    candidates: &Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats),
        (With<RemotePlayer>, Without<Player>),
    >,
) -> Option<u64> {
    if let Some(selected_id) = target_state.selected_player_id {
        return Some(selected_id);
    }
    let local_transform = local_transform?;
    let selected = candidates
        .iter()
        .filter(|(_, _, _, stats)| stats.is_alive())
        .min_by(|(_, a_transform, _, _), (_, b_transform, _, _)| {
            let a_dist = a_transform
                .translation
                .distance_squared(local_transform.translation);
            let b_dist = b_transform
                .translation
                .distance_squared(local_transform.translation);
            a_dist.total_cmp(&b_dist)
        })
        .map(|(entity, _, id, _)| (entity, id.0));

    if let Some((entity, id)) = selected {
        target_state.selected_entity = Some(entity);
        target_state.selected_player_id = Some(id);
        return Some(id);
    }

    None
}
