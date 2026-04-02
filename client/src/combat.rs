use bevy::{
    camera::primitives::Aabb,
    input::mouse::MouseButton,
    math::{
        primitives::{InfinitePlane3d, Rectangle},
        Dir3,
    },
    prelude::*,
    window::PrimaryWindow,
};

use crate::camera::MainCamera;
use crate::debug_console::DebugConsole;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkMinion, NetworkMinionId, NetworkNeutral,
    NetworkNeutralId, NetworkPlayerId, NetworkStructure, NetworkStructureId, PlayerProgression,
    RemotePlayer, StructureKind, TargetId, TargetKind,
};
use shared::{SkillSlot, TargetingMode, ability_for_slot, scaled_mana_cost};
use crate::player::Player;
use crate::team::Team;

pub const MAX_HP: f32 = 100.0;
pub const MAX_MANA: f32 = 100.0;

/// Must match `MELEE_RANGE_XZ` in `server/src/main.rs` (TASK-07 / AD2).
pub const MELEE_RANGE_XZ: f32 = 2.75;
/// Must match `SPELL_MANA_COST` for skill 1 on the server.
pub const MELEE_MANA_COST: f32 = 20.0;
/// Must match `MAX_MELEE_SKILL_RANK` on the server.
pub const MAX_MELEE_SKILL_RANK: u32 = 5;

const BAR_WIDTH: f32 = 1.45;
const BAR_HEIGHT: f32 = 0.09;
const BAR_LAYER_OFFSET: f32 = 0.01;
const MANA_BAR_OFFSET_Y: f32 = -0.15;
const BAR_HEAD_CLEARANCE: f32 = 0.28;
const MIN_PLAYER_BAR_Y: f32 = 1.4;
const TOWER_BAR_Y: f32 = 3.6;
const BASE_TOWER_BAR_Y: f32 = 4.8;
const TARGET_PICK_RADIUS: f32 = 4.0;
const TARGET_MARKER_SIZE: f32 = 2.0;
const TARGET_MARKER_THICKNESS: f32 = 0.08;
const TARGET_MARKER_Y: f32 = 0.24;
const TARGET_MARKER_EDGE: f32 = 0.18;
const TARGET_MARKER_PULSE_AMPLITUDE: f32 = 0.09;
const TARGET_MARKER_BOB_AMPLITUDE: f32 = 0.06;
const TARGET_MARKER_SPIN_SPEED: f32 = 2.6;
const PLAYER_MARKER_RADIUS: f32 = 1.25;
const MINION_MARKER_RADIUS: f32 = 1.05;
const NEUTRAL_MARKER_RADIUS: f32 = 1.1;
const TOWER_MARKER_RADIUS: f32 = 2.0;
const BASE_TOWER_MARKER_RADIUS: f32 = 3.75;
const SKILL_BUTTON_SIZE: f32 = 72.0;
const SKILL_BUTTON_GAP: f32 = 8.0;
const SKILL_BUTTON_MARGIN: f32 = 20.0;
const SKILL_BUTTON_COLOR: Color = Color::srgba(0.12, 0.12, 0.12, 0.75);

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
                    cast_spell_system,
                    skill_button_system,
                    melee_skill_upgrade_system,
                    update_target_marker_system,
                )
                    .chain(),
            );
        app.add_systems(
            PostUpdate,
            (
                spawn_combat_bars_system,
                update_combat_bars_system,
                sync_combat_bar_transforms_system,
            )
                .chain(),
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
    selected_target: Option<TargetId>,
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
struct HotbarSlotButton(SkillSlot);

#[derive(Component)]
struct HotbarSlotLabel(SkillSlot);

#[derive(Component)]
struct CombatBarRoot;

#[derive(Component)]
struct CombatBarAnchor {
    target: Entity,
    y_offset: f32,
}

fn setup_combat_visual_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let bar_mesh = meshes.add(Mesh::from(Rectangle::new(BAR_WIDTH, BAR_HEIGHT)));
    let marker_segment_mesh = meshes.add(Mesh::from(Cuboid::new(1.0, 1.0, 1.0)));

    let hp_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.02, 0.02),
        perceptual_roughness: 1.0,
        unlit: true,
        ..default()
    });
    let hp_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.15, 0.18),
        perceptual_roughness: 0.9,
        unlit: true,
        ..default()
    });
    let mana_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.04, 0.06, 0.18),
        perceptual_roughness: 1.0,
        unlit: true,
        ..default()
    });
    let mana_fill_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.52, 0.95),
        perceptual_roughness: 0.6,
        unlit: true,
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
            Transform::from_xyz(0.0, -50.0, 0.0),
            Visibility::Hidden,
            TargetMarker,
            Name::new("TargetMarker"),
        ))
        .with_children(|parent| {
            // Four thin segments form a ring-like selection outline.
            let horizontal_scale = Vec3::new(1.0, TARGET_MARKER_THICKNESS, TARGET_MARKER_EDGE);
            let vertical_scale = Vec3::new(TARGET_MARKER_EDGE, TARGET_MARKER_THICKNESS, 1.0);
            parent.spawn((
                Mesh3d(marker_segment_mesh.clone()),
                MeshMaterial3d(target_material.clone()),
                Transform {
                    translation: Vec3::new(0.0, 0.0, 0.5),
                    scale: horizontal_scale,
                    ..default()
                },
                Name::new("TargetMarker-North"),
            ));
            parent.spawn((
                Mesh3d(marker_segment_mesh.clone()),
                MeshMaterial3d(target_material.clone()),
                Transform {
                    translation: Vec3::new(0.0, 0.0, -0.5),
                    scale: horizontal_scale,
                    ..default()
                },
                Name::new("TargetMarker-South"),
            ));
            parent.spawn((
                Mesh3d(marker_segment_mesh.clone()),
                MeshMaterial3d(target_material.clone()),
                Transform {
                    translation: Vec3::new(0.5, 0.0, 0.0),
                    scale: vertical_scale,
                    ..default()
                },
                Name::new("TargetMarker-East"),
            ));
            parent.spawn((
                Mesh3d(marker_segment_mesh),
                MeshMaterial3d(target_material),
                Transform {
                    translation: Vec3::new(-0.5, 0.0, 0.0),
                    scale: vertical_scale,
                    ..default()
                },
                Name::new("TargetMarker-West"),
            ));
        })
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
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(SKILL_BUTTON_MARGIN),
                bottom: Val::Px(SKILL_BUTTON_MARGIN),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(SKILL_BUTTON_GAP),
                ..default()
            },
            Name::new("HotbarRow"),
        ))
        .with_children(|row| {
            for slot in [
                SkillSlot::Q,
                SkillSlot::W,
                SkillSlot::E,
                SkillSlot::R,
            ] {
                let key = match slot {
                    SkillSlot::Q => "Q",
                    SkillSlot::W => "W",
                    SkillSlot::E => "E",
                    SkillSlot::R => "R",
                };
                row.spawn((
                    Button,
                    Node {
                        width: Val::Px(SKILL_BUTTON_SIZE),
                        height: Val::Px(SKILL_BUTTON_SIZE),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(SKILL_BUTTON_COLOR),
                    HotbarSlotButton(slot),
                    Name::new(format!("Hotbar-{key}")),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new(key),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        HotbarSlotLabel(slot),
                    ));
                });
            }
        });
}

fn select_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    game_state: Option<Res<GameStateSnapshot>>,
    local_player: Query<(&Transform, &Team), With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    player_candidates: Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats, &Team),
        (With<RemotePlayer>, Without<Player>),
    >,
    structure_candidates: Query<
        (
            Entity,
            &Transform,
            &NetworkStructureId,
            &CombatStats,
            &Team,
            &StructureKind,
        ),
        With<NetworkStructure>,
    >,
    minion_candidates: Query<
        (Entity, &Transform, &NetworkMinionId, &CombatStats, &Team),
        With<NetworkMinion>,
    >,
    neutral_candidates: Query<
        (Entity, &Transform, &NetworkNeutralId, &CombatStats),
        With<NetworkNeutral>,
    >,
    mut target_state: ResMut<TargetState>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok((local_transform, local_team)) = local_player.single() else {
        return;
    };

    let mut select_entity: Option<(Entity, TargetId)> = None;
    if keyboard_input.just_pressed(KeyCode::Tab) {
        select_entity = find_nearest_enemy_target(
            local_transform.translation,
            *local_team,
            &player_candidates,
            &minion_candidates,
            &neutral_candidates,
            &structure_candidates,
        );
    }

    if mouse_input.just_pressed(MouseButton::Middle) {
        let Ok(window) = window_query.single() else {
            return;
        };
        let Ok((camera, camera_transform)) = camera_query.single() else {
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

        select_entity = find_target_near_point(
            click_point,
            *local_team,
            &player_candidates,
            &minion_candidates,
            &neutral_candidates,
            &structure_candidates,
        );
    }

    if let Some((entity, target_id)) = select_entity {
        target_state.selected_entity = Some(entity);
        target_state.selected_target = Some(target_id);
        info!(
            "Target selected: id={} ({:?})",
            target_id.id, target_id.kind
        );
    }
}

fn clear_invalid_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut target_state: ResMut<TargetState>,
    combat_stats_query: Query<&CombatStats>,
) {
    if keyboard_input.just_pressed(KeyCode::Backspace) {
        target_state.selected_entity = None;
        target_state.selected_target = None;
        return;
    }

    if let Some(entity) = target_state.selected_entity {
        let Ok(stats) = combat_stats_query.get(entity) else {
            target_state.selected_entity = None;
            target_state.selected_target = None;
            return;
        };
        if !stats.is_alive() {
            target_state.selected_entity = None;
            target_state.selected_target = None;
        }
    }
}

fn cast_spell_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    game_state: Option<Res<GameStateSnapshot>>,
    ability_bar: Res<LocalAbilityBar>,
    local_stats_query: Query<&CombatStats, With<Player>>,
    local_prog: Query<&PlayerProgression, With<Player>>,
    local_player: Query<(&Transform, &Team), With<Player>>,
    target_transforms: Query<&Transform>,
    target_teams: Query<&Team>,
    mut target_state: ResMut<TargetState>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut console: ResMut<DebugConsole>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Ok(local_stats) = local_stats_query.single() else {
        return;
    };
    let Ok((local_transform, local_team)) = local_player.single() else {
        return;
    };
    let Ok(progression) = local_prog.single() else {
        return;
    };
    let target = resolve_cast_target(&mut target_state);
    let Some(target) = target else {
        let message = "No target selected. Use TAB or middle mouse click to select.";
        console.push_line(message);
        info!("{message}");
        return;
    };
    let Some(entity) = target_state.selected_entity else {
        let message = "No target entity for melee (re-select target).";
        console.push_line(message);
        info!("{message}");
        return;
    };
    let Ok(target_transform) = target_transforms.get(entity) else {
        let message = "Target is no longer valid.";
        console.push_line(message);
        info!("{message}");
        return;
    };
    let target_team = target_teams.get(entity).ok().copied();
    if let Some(reason) = local_melee_cast_block_reason(
        target,
        *local_team,
        target_team,
        local_transform.translation,
        target_transform.translation,
        local_stats.mana,
    ) {
        console.push_line(reason);
        info!("{reason}");
        return;
    }

    command_writer.write(NetworkCommand::Cast { target });
    let message = format!(
        "Melee (rank {}) -> {} {} (mana {:.0})",
        progression.skill1_rank.max(1),
        match target.kind {
            TargetKind::Player => "player",
            TargetKind::Minion => "minion",
            TargetKind::Structure => "structure",
            TargetKind::Neutral => "neutral",
        },
        target.id,
        local_stats.mana
    );
    console.push_line(message.clone());
    info!("{message}");
}

fn hotbar_interaction_system(
    mut interactions: Query<
        (&HotbarSlotButton, &Interaction),
        (Changed<Interaction>, With<Button>),
    >,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    game_state: Option<Res<GameStateSnapshot>>,
    ability_bar: Res<LocalAbilityBar>,
    local_stats_query: Query<&CombatStats, With<Player>>,
    local_prog: Query<&PlayerProgression, With<Player>>,
    local_player: Query<(&Transform, &Team), With<Player>>,
    target_transforms: Query<&Transform>,
    target_teams: Query<&Team>,
    mut target_state: ResMut<TargetState>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut console: ResMut<DebugConsole>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    for (interaction, mut color) in interactions.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = SKILL_BUTTON_PRESS_COLOR.into();
                let Ok(local_stats) = local_stats_query.single() else {
                    continue;
                };
                let Ok((local_transform, local_team)) = local_player.single() else {
                    continue;
                };
                let Ok(progression) = local_prog.single() else {
                    continue;
                };
                let target = resolve_cast_target(&mut target_state);
                let Some(target) = target else {
                    let message = "No target selected. Use TAB or middle mouse click to select.";
                    console.push_line(message);
                    info!("{message}");
                    continue;
                };
                let Some(entity) = target_state.selected_entity else {
                    let message = "No target entity for melee (re-select target).";
                    console.push_line(message);
                    info!("{message}");
                    continue;
                };
                let Ok(target_transform) = target_transforms.get(entity) else {
                    let message = "Target is no longer valid.";
                    console.push_line(message);
                    info!("{message}");
                    continue;
                };
                let target_team = target_teams.get(entity).ok().copied();
                if let Some(reason) = local_melee_cast_block_reason(
                    target,
                    *local_team,
                    target_team,
                    local_transform.translation,
                    target_transform.translation,
                    local_stats.mana,
                ) {
                    console.push_line(reason);
                    info!("{reason}");
                    continue;
                }
                command_writer.write(NetworkCommand::Cast { target });
                let message = format!(
                    "Melee (rank {}) -> {} {} (mana {:.0})",
                    progression.skill1_rank.max(1),
                    match target.kind {
                        TargetKind::Player => "player",
                        TargetKind::Minion => "minion",
                        TargetKind::Structure => "structure",
                        TargetKind::Neutral => "neutral",
                    },
                    target.id,
                    local_stats.mana
                );
                console.push_line(message.clone());
                info!("{message}");
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
    players_without_bars: Query<
        (
            Entity,
            Option<&StructureKind>,
            Option<&NetworkMinion>,
            Option<&NetworkNeutral>,
        ),
        (With<CombatStats>, Without<CombatBars>),
    >,
) {
    for (entity, structure_kind, minion_marker, neutral_marker) in players_without_bars.iter() {
        let bar_y = match structure_kind.copied() {
            Some(StructureKind::Tower) => TOWER_BAR_Y,
            Some(StructureKind::BaseTower) => BASE_TOWER_BAR_Y,
            None => 2.1,
        };
        let show_mana_bar =
            structure_kind.is_none() && minion_marker.is_none() && neutral_marker.is_none();
        let mut bars = CombatBars::default();
        let bar_root = commands
            .spawn((
                Transform::from_xyz(0.0, bar_y, 0.0),
                Visibility::default(),
                CombatBarRoot,
                CombatBarAnchor {
                    target: entity,
                    y_offset: bar_y,
                },
                Name::new("CombatBarRoot"),
            ))
            .id();

        commands.entity(bar_root).with_children(|parent| {
            parent.spawn((
                Mesh3d(assets.bar_mesh.clone()),
                MeshMaterial3d(assets.hp_bg_material.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                Name::new("HpBarBg"),
            ));

            let hp_fill = parent
                .spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.hp_fill_material.clone()),
                    Transform::from_xyz(0.0, 0.0, BAR_LAYER_OFFSET),
                    Name::new("HpBarFill"),
                ))
                .id();

            bars.hp_fill = Some(hp_fill);
            if show_mana_bar {
                parent.spawn((
                    Mesh3d(assets.bar_mesh.clone()),
                    MeshMaterial3d(assets.mana_bg_material.clone()),
                    Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, 0.0),
                    Name::new("ManaBarBg"),
                ));

                let mana_fill = parent
                    .spawn((
                        Mesh3d(assets.bar_mesh.clone()),
                        MeshMaterial3d(assets.mana_fill_material.clone()),
                        Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, BAR_LAYER_OFFSET),
                        Name::new("ManaBarFill"),
                    ))
                    .id();
                bars.mana_fill = Some(mana_fill);
            }
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

fn sync_combat_bar_transforms_system(
    mut commands: Commands,
    camera_query: Query<&GlobalTransform, With<MainCamera>>,
    global_query: Query<&GlobalTransform>,
    aabb_query: Query<&Aabb>,
    children_query: Query<&Children>,
    mut bar_query: Query<(Entity, &CombatBarAnchor, &mut Transform), With<CombatBarRoot>>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };
    let camera_rotation = camera_transform.compute_transform().rotation;

    for (bar_entity, anchor, mut bar_transform) in bar_query.iter_mut() {
        let Ok(target_transform) = global_query.get(anchor.target) else {
            commands
                .entity(bar_entity)
                .despawn_related::<Children>()
                .despawn();
            continue;
        };
        let bar_world_y = compute_bar_world_y_for_entity(
            anchor.target,
            target_transform.translation().y + anchor.y_offset,
            &children_query,
            &aabb_query,
            &global_query,
        );
        bar_transform.translation = Vec3::new(
            target_transform.translation().x,
            bar_world_y,
            target_transform.translation().z,
        );
        bar_transform.rotation = camera_rotation;
    }
}

fn compute_bar_world_y_for_entity(
    entity: Entity,
    fallback_world_y: f32,
    children_query: &Query<&Children>,
    aabb_query: &Query<&Aabb>,
    global_query: &Query<&GlobalTransform>,
) -> f32 {
    let mut max_y = f32::NEG_INFINITY;
    let mut has_bounds = false;

    let mut sample_entity = |sample: Entity| {
        let (Ok(aabb), Ok(global)) = (aabb_query.get(sample), global_query.get(sample)) else {
            return;
        };
        let center: Vec3 = aabb.center.into();
        let half: Vec3 = aabb.half_extents.into();
        for sx in [-1.0_f32, 1.0] {
            for sy in [-1.0_f32, 1.0] {
                for sz in [-1.0_f32, 1.0] {
                    let local_corner = center + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                    let world_corner = global.transform_point(local_corner);
                    max_y = max_y.max(world_corner.y);
                    has_bounds = true;
                }
            }
        }
    };

    sample_entity(entity);
    for child in children_query.iter_descendants(entity) {
        sample_entity(child);
    }

    if !has_bounds {
        return fallback_world_y;
    }

    (max_y + BAR_HEAD_CLEARANCE).max(MIN_PLAYER_BAR_Y)
}

fn update_target_marker_system(
    time: Res<Time>,
    target_state: Res<TargetState>,
    children_query: Query<&Children>,
    aabb_query: Query<&Aabb>,
    global_query: Query<&GlobalTransform, Without<TargetMarker>>,
    structure_kinds: Query<&StructureKind, With<NetworkStructure>>,
    minions: Query<(), With<NetworkMinion>>,
    neutrals: Query<(), With<NetworkNeutral>>,
    players: Query<(), Or<(With<Player>, With<RemotePlayer>)>>,
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
    let Ok(target_transform) = global_query.get(target_entity) else {
        *marker_visibility = Visibility::Hidden;
        return;
    };

    *marker_visibility = Visibility::Visible;
    let target_translation = target_transform.translation();
    let marker_radius = if let Ok(kind) = structure_kinds.get(target_entity) {
        match kind {
            StructureKind::Tower => TOWER_MARKER_RADIUS,
            StructureKind::BaseTower => BASE_TOWER_MARKER_RADIUS,
        }
    } else if minions.get(target_entity).is_ok() {
        MINION_MARKER_RADIUS
    } else if neutrals.get(target_entity).is_ok() {
        NEUTRAL_MARKER_RADIUS
    } else if players.get(target_entity).is_ok() {
        PLAYER_MARKER_RADIUS
    } else {
        TARGET_MARKER_SIZE * 0.5
    };
    let pulse = 1.0 + TARGET_MARKER_PULSE_AMPLITUDE * (time.elapsed_secs() * 7.5).sin();
    let bob = TARGET_MARKER_BOB_AMPLITUDE * (time.elapsed_secs() * 5.0).sin();
    let marker_center_y = compute_marker_world_y_for_entity(
        target_entity,
        target_translation.y + TARGET_MARKER_Y,
        &children_query,
        &aabb_query,
        &global_query,
    );
    marker_transform.translation = Vec3::new(
        target_translation.x,
        marker_center_y + bob,
        target_translation.z,
    );
    marker_transform.rotation =
        Quat::from_rotation_y(time.elapsed_secs() * TARGET_MARKER_SPIN_SPEED);
    marker_transform.scale = Vec3::new(marker_radius * pulse, 1.0, marker_radius * pulse);
}

fn compute_marker_world_y_for_entity(
    entity: Entity,
    fallback_world_y: f32,
    children_query: &Query<&Children>,
    aabb_query: &Query<&Aabb>,
    global_query: &Query<&GlobalTransform, Without<TargetMarker>>,
) -> f32 {
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut has_bounds = false;

    let mut sample_entity = |sample: Entity| {
        let (Ok(aabb), Ok(global)) = (aabb_query.get(sample), global_query.get(sample)) else {
            return;
        };
        let center: Vec3 = aabb.center.into();
        let half: Vec3 = aabb.half_extents.into();
        for sx in [-1.0_f32, 1.0] {
            for sy in [-1.0_f32, 1.0] {
                for sz in [-1.0_f32, 1.0] {
                    let local_corner = center + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                    let world_corner = global.transform_point(local_corner);
                    min_y = min_y.min(world_corner.y);
                    max_y = max_y.max(world_corner.y);
                    has_bounds = true;
                }
            }
        }
    };

    sample_entity(entity);
    for child in children_query.iter_descendants(entity) {
        sample_entity(child);
    }

    if !has_bounds {
        return fallback_world_y;
    }

    (min_y + max_y) * 0.5
}

fn find_nearest_enemy_target(
    local_pos: Vec3,
    local_team: Team,
    player_candidates: &Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats, &Team),
        (With<RemotePlayer>, Without<Player>),
    >,
    minion_candidates: &Query<
        (Entity, &Transform, &NetworkMinionId, &CombatStats, &Team),
        With<NetworkMinion>,
    >,
    neutral_candidates: &Query<
        (Entity, &Transform, &NetworkNeutralId, &CombatStats),
        With<NetworkNeutral>,
    >,
    structure_candidates: &Query<
        (
            Entity,
            &Transform,
            &NetworkStructureId,
            &CombatStats,
            &Team,
            &StructureKind,
        ),
        With<NetworkStructure>,
    >,
) -> Option<(Entity, TargetId)> {
    let mut best: Option<(Entity, TargetId, f32)> = None;

    for (entity, transform, id, stats, team) in player_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.map_or(true, |(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Player,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats, team) in minion_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.map_or(true, |(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Minion,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats) in neutral_candidates.iter() {
        if !stats.is_alive() {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.map_or(true, |(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Neutral,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    for (entity, transform, id, stats, team, _kind) in structure_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist_sq = transform.translation.distance_squared(local_pos);
        if best.map_or(true, |(_, _, best_dist)| dist_sq < best_dist) {
            best = Some((
                entity,
                TargetId {
                    kind: TargetKind::Structure,
                    id: id.0,
                },
                dist_sq,
            ));
        }
    }

    best.map(|(entity, target, _)| (entity, target))
}

fn find_target_near_point(
    click_point: Vec3,
    local_team: Team,
    player_candidates: &Query<
        (Entity, &Transform, &NetworkPlayerId, &CombatStats, &Team),
        (With<RemotePlayer>, Without<Player>),
    >,
    minion_candidates: &Query<
        (Entity, &Transform, &NetworkMinionId, &CombatStats, &Team),
        With<NetworkMinion>,
    >,
    neutral_candidates: &Query<
        (Entity, &Transform, &NetworkNeutralId, &CombatStats),
        With<NetworkNeutral>,
    >,
    structure_candidates: &Query<
        (
            Entity,
            &Transform,
            &NetworkStructureId,
            &CombatStats,
            &Team,
            &StructureKind,
        ),
        With<NetworkStructure>,
    >,
) -> Option<(Entity, TargetId)> {
    let mut best: Option<(Entity, TargetId, f32)> = None;

    for (entity, transform, id, stats, team) in player_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist = transform.translation.xz().distance(click_point.xz());
        if dist <= TARGET_PICK_RADIUS {
            if best.map_or(true, |(_, _, best_dist)| dist < best_dist) {
                best = Some((
                    entity,
                    TargetId {
                        kind: TargetKind::Player,
                        id: id.0,
                    },
                    dist,
                ));
            }
        }
    }

    for (entity, transform, id, stats, team) in minion_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist = transform.translation.xz().distance(click_point.xz());
        if dist <= TARGET_PICK_RADIUS {
            if best.map_or(true, |(_, _, best_dist)| dist < best_dist) {
                best = Some((
                    entity,
                    TargetId {
                        kind: TargetKind::Minion,
                        id: id.0,
                    },
                    dist,
                ));
            }
        }
    }

    for (entity, transform, id, stats) in neutral_candidates.iter() {
        if !stats.is_alive() {
            continue;
        }
        let dist = transform.translation.xz().distance(click_point.xz());
        if dist <= TARGET_PICK_RADIUS {
            if best.map_or(true, |(_, _, best_dist)| dist < best_dist) {
                best = Some((
                    entity,
                    TargetId {
                        kind: TargetKind::Neutral,
                        id: id.0,
                    },
                    dist,
                ));
            }
        }
    }

    for (entity, transform, id, stats, team, _kind) in structure_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let dist = transform.translation.xz().distance(click_point.xz());
        if dist <= TARGET_PICK_RADIUS {
            if best.map_or(true, |(_, _, best_dist)| dist < best_dist) {
                best = Some((
                    entity,
                    TargetId {
                        kind: TargetKind::Structure,
                        id: id.0,
                    },
                    dist,
                ));
            }
        }
    }

    best.map(|(entity, target, _)| (entity, target))
}

fn resolve_cast_target(target_state: &mut TargetState) -> Option<TargetId> {
    target_state.selected_target
}

fn horizontal_distance_squared_xz(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz
}

/// Client-side checks aligned with server melee validation (TASK-07).
fn local_melee_cast_block_reason(
    target: TargetId,
    local_team: Team,
    target_team: Option<Team>,
    local_translation: Vec3,
    target_world_translation: Vec3,
    mana: f32,
) -> Option<&'static str> {
    if target.kind == TargetKind::Structure {
        return Some("Melee skill cannot target structures.");
    }
    if matches!(target.kind, TargetKind::Player | TargetKind::Minion) {
        let Some(t_team) = target_team else {
            return Some("Target is no longer valid.");
        };
        if t_team == local_team {
            return Some("Melee cannot target allied units.");
        }
    }
    if mana < MELEE_MANA_COST {
        return Some("Not enough mana for melee (20).");
    }
    let max_sq = MELEE_RANGE_XZ * MELEE_RANGE_XZ;
    if horizontal_distance_squared_xz(local_translation, target_world_translation) > max_sq {
        return Some("Target is out of melee range.");
    }
    None
}

fn melee_skill_upgrade_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    game_state: Option<Res<GameStateSnapshot>>,
    local_prog: Query<&PlayerProgression, With<Player>>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut console: ResMut<DebugConsole>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    if !keyboard_input.just_pressed(KeyCode::Digit1) {
        return;
    }
    let Ok(prog) = local_prog.single() else {
        return;
    };
    if prog.skill_points == 0 {
        let message = "No skill points to upgrade melee.";
        console.push_line(message);
        info!("{message}");
        return;
    }
    if prog.skill1_rank >= MAX_MELEE_SKILL_RANK {
        let message = "Melee skill is already max rank.";
        console.push_line(message);
        info!("{message}");
        return;
    }
    command_writer.write(NetworkCommand::UpgradeMeleeSkill);
    let message = "Sent melee skill upgrade request.";
    console.push_line(message);
    info!("{message}");
}
