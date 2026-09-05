use bevy::{
    camera::primitives::Aabb, ecs::system::SystemParam, input::mouse::MouseButton,
    math::primitives::Rectangle, prelude::*, window::PrimaryWindow,
};

use crate::camera::MainCamera;
use crate::debug_console::DebugConsole;
use crate::input_bindings::{SKILL_CAST_KEYS, SKILL_UPGRADE_KEY};
use crate::minimap::MinimapNavigationState;
use crate::model_scale::NormalizeModelScale;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkMinion, NetworkMinionId,
    NetworkNeutral, NetworkNeutralId, NetworkPlayerId, NetworkStructure, NetworkStructureId,
    PlayerProgression, RemotePlayer, StructureKind, TargetId, TargetKind,
};
use crate::player::{MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::team::{Team, TeamSelection};
use shared::{
    HeroClass, MAX_ABILITY_RANK, SkillSlot, TargetingMode, ability_for_class_slot,
    scaled_cast_range, scaled_cooldown, scaled_mana_cost, unlocked_slots_for_level,
};

/// Must match server `server/src/balance.rs` player baselines (display / local defaults).
pub const MAX_HP: f32 = 100.0;
/// Must match server `server/src/balance.rs` player baselines (display / local defaults).
pub const MAX_MANA: f32 = 100.0;

/// Local per-slot cast cooldown mirror for HUD feedback (the server remains
/// authoritative; values come from the shared class kit numbers).
#[derive(Resource, Default)]
pub struct LocalCastCooldown {
    pub remaining_secs: [f32; 4],
}

const BAR_WIDTH: f32 = 1.45;
const BAR_HEIGHT: f32 = 0.09;
const BAR_LAYER_OFFSET: f32 = 0.01;
const MANA_BAR_OFFSET_Y: f32 = -0.15;
const BAR_HEAD_CLEARANCE: f32 = 0.28;
const MIN_PLAYER_BAR_Y: f32 = 1.4;
const TOWER_BAR_Y: f32 = 3.6;
const BASE_TOWER_BAR_Y: f32 = 4.8;
const PLAYER_PICK_RADIUS_PX: f32 = 52.0;
const MINION_PICK_RADIUS_PX: f32 = 48.0;
const NEUTRAL_PICK_RADIUS_PX: f32 = 52.0;
const TOWER_PICK_RADIUS_PX: f32 = 56.0;
const BASE_TOWER_PICK_RADIUS_PX: f32 = 68.0;
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
const SKILL_SLOT_SIZE: f32 = 64.0;
const SKILL_SLOT_GAP: f32 = 8.0;
const SKILL_BUTTON_MARGIN: f32 = 20.0;
const SKILL_BUTTON_COLOR: Color = Color::srgba(0.12, 0.12, 0.12, 0.75);
const SKILL_BUTTON_HOVER_COLOR: Color = Color::srgba(0.18, 0.18, 0.18, 0.85);
const SKILL_BUTTON_PRESS_COLOR: Color = Color::srgba(0.28, 0.28, 0.28, 0.95);
const SKILL_UPGRADE_READY_COLOR: Color = Color::srgba(0.20, 0.62, 0.26, 0.95);
const SKILL_UPGRADE_HOVER_COLOR: Color = Color::srgba(0.26, 0.72, 0.32, 0.98);
const SKILL_UPGRADE_IDLE_COLOR: Color = Color::srgba(0.16, 0.16, 0.18, 0.55);

pub struct CombatPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CombatPointerInputSet;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorldMovementInputSet;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TargetState>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<WorldPointerState>()
            .init_resource::<PendingCast>()
            .add_systems(Startup, setup_combat_visual_assets)
            .add_systems(Startup, setup_combat_ui)
            .add_systems(Update, select_target_system.in_set(CombatPointerInputSet))
            .add_systems(
                Update,
                (
                    tick_local_cast_cooldown,
                    clear_invalid_target_system,
                    cast_spell_system,
                    skill_button_system,
                    resolve_pending_cast_system,
                    skill_upgrade_input_system,
                    update_skill_bar_system,
                    update_target_marker_system,
                )
                    .chain()
                    .after(WorldMovementInputSet),
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
    is_2d: bool,
    hp_bg_material: Handle<StandardMaterial>,
    hp_fill_material: Handle<StandardMaterial>,
    mana_bg_material: Handle<StandardMaterial>,
    mana_fill_material: Handle<StandardMaterial>,
    bar_mesh: Handle<Mesh>,
}

#[derive(Resource, Default)]
pub struct TargetState {
    pub selected_entity: Option<Entity>,
    pub selected_target: Option<TargetId>,
    marker_entity: Option<Entity>,
}

/// Per-frame routing state that prevents one physical press from both
/// attacking a unit and issuing a ground movement command.
#[derive(Resource, Default)]
pub(crate) struct WorldPointerState {
    pub(crate) consumed_primary_press: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingCastRequest {
    slot: usize,
    target_entity: Option<Entity>,
    target: Option<TargetId>,
    approach_announced: bool,
}

#[derive(Resource, Default)]
pub(crate) struct PendingCast {
    request: Option<PendingCastRequest>,
}

impl PendingCast {
    pub(crate) fn cancel(&mut self) {
        self.request = None;
    }
}

#[derive(SystemParam)]
struct TargetCandidates<'w, 's> {
    players: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkPlayerId,
            &'static CombatStats,
            &'static Team,
        ),
        (With<RemotePlayer>, Without<Player>),
    >,
    structures: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkStructureId,
            &'static CombatStats,
            &'static Team,
            &'static StructureKind,
        ),
        With<NetworkStructure>,
    >,
    minions: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkMinionId,
            &'static CombatStats,
            &'static Team,
        ),
        With<NetworkMinion>,
    >,
    neutrals: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static NetworkNeutralId,
            &'static CombatStats,
        ),
        With<NetworkNeutral>,
    >,
}

#[derive(Component, Default)]
struct CombatBars {
    hp_fill: Option<Entity>,
    mana_fill: Option<Entity>,
}

#[derive(Component)]
struct TargetMarker;

#[derive(Component)]
struct SkillBarSlot {
    slot: usize,
}

#[derive(Component)]
struct SkillUpgradeButton {
    slot: usize,
}

#[derive(Component)]
struct SkillRankLabel {
    slot: usize,
}

/// Ability-name caption on a hotbar slot; follows the selected class kit.
#[derive(Component)]
struct SkillNameLabel {
    slot: usize,
}

fn tick_local_cast_cooldown(time: Res<Time>, mut cd: ResMut<LocalCastCooldown>) {
    for remaining in cd.remaining_secs.iter_mut() {
        if *remaining > 0.0 {
            *remaining = (*remaining - time.delta_secs()).max(0.0);
        }
    }
}

/// The class whose kit drives the local HUD: server-replicated when available,
/// otherwise the pre-join selection.
fn local_hero_class(
    replicated: Option<Option<&NetworkHeroClass>>,
    selection: &TeamSelection,
) -> HeroClass {
    replicated
        .flatten()
        .map(|class| class.0)
        .unwrap_or(selection.hero_class)
}

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
    mode: Res<PlayerVisualMode>,
) {
    let is_2d = *mode == PlayerVisualMode::Sprite2d;
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

    let mut marker_commands = commands.spawn((
        Transform::from_xyz(0.0, -50.0, 0.0),
        Visibility::Hidden,
        TargetMarker,
        Name::new("TargetMarker"),
    ));
    if is_2d {
        marker_commands.insert(Sprite::from_color(
            Color::srgba(1.0, 0.84, 0.24, 0.45),
            Vec2::splat(TARGET_MARKER_SIZE),
        ));
    }
    let marker_entity = marker_commands
        .with_children(|parent| {
            if is_2d {
                return;
            }
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
        is_2d,
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
                column_gap: Val::Px(SKILL_SLOT_GAP),
                align_items: AlignItems::Center,
                ..default()
            },
            ZIndex(12),
            Name::new("SkillBarRoot"),
        ))
        .with_children(|row| {
            for i in 0..4 {
                let label = crate::input_bindings::SKILL_SLOT_KEY_LABELS[i];
                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    Name::new(format!("SkillColumn-{label}")),
                ))
                .with_children(|col| {
                    // Upgrade arrow above the slot; bright when a point can be spent.
                    col.spawn((
                        Button,
                        Node {
                            width: Val::Px(SKILL_SLOT_SIZE),
                            height: Val::Px(22.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(SKILL_UPGRADE_IDLE_COLOR),
                        SkillUpgradeButton { slot: i },
                        Name::new(format!("SkillUpgrade-{label}")),
                    ))
                    .with_children(|arrow| {
                        arrow.spawn((
                            Text::new("\u{2191}"),
                            TextFont {
                                font_size: 16.0,
                                ..default()
                            },
                            TextColor::WHITE,
                        ));
                    });

                    col.spawn((
                        Button,
                        Node {
                            width: Val::Px(SKILL_SLOT_SIZE),
                            height: Val::Px(SKILL_SLOT_SIZE),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(1.0),
                            ..default()
                        },
                        BackgroundColor(SKILL_BUTTON_COLOR),
                        SkillBarSlot { slot: i },
                        Name::new(format!("SkillSlot-{label}")),
                    ))
                    .with_children(|slot| {
                        slot.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: 20.0,
                                ..default()
                            },
                            TextColor::WHITE,
                        ));
                        slot.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 9.5,
                                ..default()
                            },
                            TextColor(Color::srgba(0.88, 0.90, 0.94, 1.0)),
                            SkillNameLabel { slot: i },
                        ));
                        slot.spawn((
                            Text::new("Lv 1"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(Color::srgba(0.82, 0.84, 0.90, 1.0)),
                            SkillRankLabel { slot: i },
                        ));
                    });
                });
            }
        });
}

/// Reflect the selected class kit + server ranks on the hotbar and light the
/// upgrade arrows when the local player has a skill point to spend and the
/// slot is below the shared max rank.
#[allow(clippy::type_complexity)]
fn update_skill_bar_system(
    progression: Query<(&PlayerProgression, Option<&NetworkHeroClass>), With<Player>>,
    team_selection: Res<TeamSelection>,
    mut rank_labels: Query<(&SkillRankLabel, &mut Text), Without<SkillNameLabel>>,
    mut name_labels: Query<(&SkillNameLabel, &mut Text), Without<SkillRankLabel>>,
    mut upgrade_buttons: Query<
        (
            &SkillUpgradeButton,
            &Interaction,
            &mut BackgroundColor,
            &mut Node,
        ),
        With<Button>,
    >,
) {
    let local = progression.iter().next();
    let prog = local.map(|(prog, _)| *prog).unwrap_or_default();
    let class = local_hero_class(local.map(|(_, class)| class), &team_selection);

    for (label, mut text) in &mut name_labels {
        let Some(slot) = SkillSlot::from_index(label.slot as u8) else {
            continue;
        };
        let next = ability_for_class_slot(class, slot).name;
        if text.0 != next {
            text.0 = next.to_string();
        }
    }

    for (label, mut text) in &mut rank_labels {
        let rank = prog.ranks.get(label.slot).copied().unwrap_or(1).max(1);
        let next = format!("Lv {rank}");
        if text.0 != next {
            text.0 = next;
        }
    }

    for (button, interaction, mut color, mut node) in &mut upgrade_buttons {
        let rank = prog.ranks.get(button.slot).copied().unwrap_or(1).max(1);
        let can_upgrade = prog.skill_points > 0 && rank < MAX_ABILITY_RANK;
        // Arrow only shows when a point can actually be spent on this slot.
        let display = if can_upgrade {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let next_color = if matches!(interaction, Interaction::Hovered | Interaction::Pressed) {
            SKILL_UPGRADE_HOVER_COLOR
        } else {
            SKILL_UPGRADE_READY_COLOR
        };
        *color = next_color.into();
    }
}

/// Arrow click or the upgrade key spends a point on the matching slot. The server
/// is authoritative: it ignores the request when no point is available.
fn skill_upgrade_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    progression: Query<&PlayerProgression, With<Player>>,
    upgrade_buttons: Query<
        (&SkillUpgradeButton, &Interaction),
        (Changed<Interaction>, With<Button>),
    >,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    let can_spend = progression
        .iter()
        .next()
        .is_some_and(|prog| prog.skill_points > 0);

    if can_spend && keyboard.just_pressed(SKILL_UPGRADE_KEY) {
        command_writer.write(NetworkCommand::UpgradeSkill { slot: 0 });
    }

    for (button, interaction) in &upgrade_buttons {
        if matches!(interaction, Interaction::Pressed) && can_spend {
            command_writer.write(NetworkCommand::UpgradeSkill {
                slot: button.slot as u8,
            });
        }
    }
}

fn select_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    game_state: Option<Res<GameStateSnapshot>>,
    local_player: Query<(&Transform, &Team), With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    candidates: TargetCandidates,
    mut target_state: ResMut<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut pointer_state: ResMut<WorldPointerState>,
    visual_mode: Res<PlayerVisualMode>,
    minimap_nav: Option<Res<MinimapNavigationState>>,
    ui_interactions: Query<&Interaction, With<Button>>,
) {
    pointer_state.consumed_primary_press = false;
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
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        );
    }

    let Ok(window) = window_query.single() else {
        return;
    };
    let primary_position = primary_press_position(&mouse_input, &touches, window);
    let middle_position = mouse_input
        .just_pressed(MouseButton::Middle)
        .then(|| window.cursor_position())
        .flatten();
    let pointer_position = primary_position.or(middle_position);

    if let Some(pointer_position) = pointer_position {
        let pointer_over_ui = ui_interactions
            .iter()
            .any(|interaction| *interaction != Interaction::None);
        let pointer_on_minimap = minimap_nav
            .as_ref()
            .is_some_and(|nav| nav.consumed_primary_click);
        if pointer_over_ui || pointer_on_minimap {
            return;
        }
        let Ok((camera, camera_transform)) = camera_query.single() else {
            return;
        };
        select_entity = find_target_near_screen(
            pointer_position,
            camera,
            camera_transform,
            *visual_mode,
            *local_team,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        );
    }

    if let Some((entity, target_id)) = select_entity {
        target_state.selected_entity = Some(entity);
        target_state.selected_target = Some(target_id);
        if primary_position.is_some() {
            pointer_state.consumed_primary_press = true;
            pending_cast.request = Some(PendingCastRequest {
                slot: SkillSlot::Q.index(),
                target_entity: Some(entity),
                target: Some(target_id),
                approach_announced: false,
            });
        }
        info!(
            "Target selected: id={} ({:?})",
            target_id.id, target_id.kind
        );
    }
}

fn primary_press_position(
    mouse_input: &ButtonInput<MouseButton>,
    touches: &Touches,
    window: &Window,
) -> Option<Vec2> {
    if mouse_input.just_pressed(MouseButton::Left) {
        return window.cursor_position();
    }
    touches
        .iter_just_pressed()
        .next()
        .map(|touch| touch.position())
}

fn clear_invalid_target_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut target_state: ResMut<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    combat_stats_query: Query<&CombatStats>,
) {
    if keyboard_input.just_pressed(KeyCode::Backspace) {
        target_state.selected_entity = None;
        target_state.selected_target = None;
        pending_cast.cancel();
        return;
    }

    if let Some(entity) = target_state.selected_entity {
        let Ok(stats) = combat_stats_query.get(entity) else {
            target_state.selected_entity = None;
            target_state.selected_target = None;
            pending_cast.cancel();
            return;
        };
        if !stats.is_alive() {
            target_state.selected_entity = None;
            target_state.selected_target = None;
            pending_cast.cancel();
        }
    }
}

/// Resolves the target and sends a slot cast for the local player's class kit.
/// Client-side checks (unlock level, local cooldown, target presence) exist for
/// responsive UX only; the server re-validates everything authoritatively.
fn try_cast_slot(
    slot_index: usize,
    class: HeroClass,
    local: (&CombatStats, PlayerProgression, Option<&NetworkPlayerId>),
    selected_target: Option<TargetId>,
    command_writer: &mut MessageWriter<NetworkCommand>,
    console: &mut DebugConsole,
    cast_cd: &mut LocalCastCooldown,
) -> bool {
    let Some(slot) = SkillSlot::from_index(slot_index as u8) else {
        return false;
    };
    let (stats, prog, net_id) = local;
    if !stats.is_alive() {
        return false;
    }
    let def = ability_for_class_slot(class, slot);
    if !unlocked_slots_for_level(prog.level.max(1))[slot.index()] {
        let message = format!(
            "{} is locked until level {}.",
            def.name,
            shared::SLOT_UNLOCK_LEVELS[slot.index()]
        );
        console.push_line(message.clone());
        info!("{message}");
        return false;
    }
    if cast_cd.remaining_secs[slot.index()] > 0.0 {
        let message = format!(
            "{} is cooling down for {:.1}s.",
            def.name,
            cast_cd.remaining_secs[slot.index()]
        );
        console.push_line(message.clone());
        info!("{message}");
        return false;
    }

    let target = match def.targeting {
        TargetingMode::SelfTarget => net_id.map(|id| TargetId {
            kind: TargetKind::Player,
            id: id.0,
        }),
        TargetingMode::UnitTarget => selected_target,
    };
    let Some(target) = target else {
        let message = match def.targeting {
            TargetingMode::UnitTarget => {
                "No target available. Click or tap an enemy, or use Tab to select."
            }
            TargetingMode::SelfTarget => "Not connected yet; self-cast unavailable.",
        };
        console.push_line(message);
        info!("{message}");
        return false;
    };

    let rank = prog.ranks[slot.index()].clamp(1, def.max_rank);
    let mana_cost = scaled_mana_cost(def, rank);
    if stats.mana < mana_cost {
        let message = format!(
            "Not enough mana for {} ({:.0}/{:.0}).",
            def.name, stats.mana, mana_cost
        );
        console.push_line(message.clone());
        info!("{message}");
        return false;
    }
    cast_cd.remaining_secs[slot.index()] = scaled_cooldown(def, rank).as_secs_f32();
    command_writer.write(NetworkCommand::Cast {
        target,
        slot: slot.index() as u8,
    });
    let message = format!(
        "Cast {} -> {} {} (mana {:.0})",
        def.name,
        match target.kind {
            TargetKind::Player => "player",
            TargetKind::Minion => "minion",
            TargetKind::Structure => "structure",
            TargetKind::Neutral => "neutral",
        },
        target.id,
        stats.mana
    );
    console.push_line(message.clone());
    info!("{message}");
    true
}

fn queue_cast_request(
    slot_index: usize,
    class: HeroClass,
    target_state: &TargetState,
    pending_cast: &mut PendingCast,
    console: &mut DebugConsole,
) {
    let Some(slot) = SkillSlot::from_index(slot_index as u8) else {
        return;
    };
    let def = ability_for_class_slot(class, slot);
    let (target_entity, target) = match def.targeting {
        TargetingMode::SelfTarget => (None, None),
        TargetingMode::UnitTarget => {
            let (Some(entity), Some(target)) =
                (target_state.selected_entity, target_state.selected_target)
            else {
                let message = "No target available. Click or tap an enemy, or use Tab to select.";
                console.push_line(message);
                info!("{message}");
                return;
            };
            (Some(entity), Some(target))
        }
    };
    pending_cast.request = Some(PendingCastRequest {
        slot: slot_index,
        target_entity,
        target,
        approach_announced: false,
    });
}

#[allow(clippy::type_complexity)]
fn cast_spell_system(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    target_state: Res<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut console: ResMut<DebugConsole>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    let Some(slot_index) = SKILL_CAST_KEYS
        .iter()
        .position(|key| keyboard_input.just_pressed(*key))
    else {
        return;
    };

    let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    queue_cast_request(
        slot_index,
        class,
        &target_state,
        &mut pending_cast,
        &mut console,
    );
}

#[allow(clippy::type_complexity)]
fn skill_button_system(
    mut interactions: Query<
        (&Interaction, &SkillBarSlot, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    game_state: Option<Res<GameStateSnapshot>>,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    target_state: Res<TargetState>,
    mut pending_cast: ResMut<PendingCast>,
    mut console: ResMut<DebugConsole>,
) {
    if let Some(game_state) = game_state.as_ref() {
        if !matches!(game_state.state, GameState::Running) {
            return;
        }
    }
    for (interaction, bar_slot, mut color) in interactions.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = SKILL_BUTTON_PRESS_COLOR.into();
                let Ok((_stats, _prog, _net_id, class)) = local_player.single() else {
                    continue;
                };
                let class = local_hero_class(Some(class), &team_selection);
                queue_cast_request(
                    bar_slot.slot,
                    class,
                    &target_state,
                    &mut pending_cast,
                    &mut console,
                );
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

#[allow(clippy::type_complexity)]
fn resolve_pending_cast_system(
    mut commands: Commands,
    team_selection: Res<TeamSelection>,
    local_player: Query<
        (
            Entity,
            &Transform,
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkPlayerId>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    target_query: Query<(&Transform, &CombatStats), Without<Player>>,
    mut pending_cast: ResMut<PendingCast>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut console: ResMut<DebugConsole>,
    mut cast_cd: ResMut<LocalCastCooldown>,
) {
    let Some(request) = pending_cast.request else {
        return;
    };
    let Ok((player_entity, player_transform, stats, progression, net_id, class)) =
        local_player.single()
    else {
        pending_cast.cancel();
        return;
    };
    let class = local_hero_class(Some(class), &team_selection);
    let Some(slot) = SkillSlot::from_index(request.slot as u8) else {
        pending_cast.cancel();
        return;
    };
    let definition = ability_for_class_slot(class, slot);

    if definition.targeting == TargetingMode::UnitTarget {
        let (Some(target_entity), Some(_target)) = (request.target_entity, request.target) else {
            pending_cast.cancel();
            return;
        };
        let Ok((target_transform, target_stats)) = target_query.get(target_entity) else {
            pending_cast.cancel();
            return;
        };
        if !target_stats.is_alive() {
            pending_cast.cancel();
            return;
        }
        let progression = progression.copied().unwrap_or_default();
        let rank = progression.ranks[slot.index()].clamp(1, definition.max_rank);
        let cast_range = scaled_cast_range(definition, rank);
        if !within_cast_range(
            player_transform.translation,
            target_transform.translation,
            cast_range,
        ) {
            commands.entity(player_entity).insert(MovementTarget {
                target: target_transform.translation,
            });
            if !request.approach_announced {
                let message = format!("Approaching target for {}.", definition.name);
                console.push_line(message.clone());
                info!("{message}");
                if let Some(request) = pending_cast.request.as_mut() {
                    request.approach_announced = true;
                }
            }
            return;
        }
    }

    commands.entity(player_entity).remove::<MovementTarget>();
    let _sent = try_cast_slot(
        request.slot,
        class,
        (stats, progression.copied().unwrap_or_default(), net_id),
        request.target,
        &mut command_writer,
        &mut console,
        &mut cast_cd,
    );
    pending_cast.cancel();
}

fn within_cast_range(local_position: Vec3, target_position: Vec3, cast_range: f32) -> bool {
    local_position.xz().distance(target_position.xz()) <= cast_range
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
            if assets.is_2d {
                parent.spawn((
                    Sprite::from_color(
                        Color::srgb(0.15, 0.02, 0.02),
                        Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                    ),
                    Transform::default(),
                    Name::new("HpBarBg"),
                ));
                let hp_fill = parent
                    .spawn((
                        Sprite::from_color(
                            Color::srgb(0.85, 0.15, 0.18),
                            Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                        ),
                        Transform::from_xyz(0.0, 0.0, BAR_LAYER_OFFSET),
                        Name::new("HpBarFill"),
                    ))
                    .id();
                bars.hp_fill = Some(hp_fill);
                if show_mana_bar {
                    parent.spawn((
                        Sprite::from_color(
                            Color::srgb(0.04, 0.06, 0.18),
                            Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                        ),
                        Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, 0.0),
                        Name::new("ManaBarBg"),
                    ));
                    bars.mana_fill = Some(
                        parent
                            .spawn((
                                Sprite::from_color(
                                    Color::srgb(0.16, 0.52, 0.95),
                                    Vec2::new(BAR_WIDTH, BAR_HEIGHT),
                                ),
                                Transform::from_xyz(0.0, MANA_BAR_OFFSET_Y, BAR_LAYER_OFFSET),
                                Name::new("ManaBarFill"),
                            ))
                            .id(),
                    );
                }
                return;
            }
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
        // `try_insert`: the owner may have been despawned in this same frame
        // (e.g. the duplicate local-player cleanup in `apply_server_snapshot`);
        // a plain `insert` would panic when the command buffer is applied. Any
        // orphaned bar root is cleaned up by `sync_combat_bar_transforms_system`.
        commands.entity(entity).try_insert(bars);
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
    normalized_query: Query<&NormalizeModelScale>,
    mut bar_query: Query<(Entity, &CombatBarAnchor, &mut Transform), With<CombatBarRoot>>,
    mode: Res<PlayerVisualMode>,
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
        if *mode == PlayerVisualMode::Sprite2d {
            let xy = crate::world2d::simulation_xz_to_render_xy(target_transform.translation());
            bar_transform.translation = Vec3::new(
                xy.x,
                xy.y + anchor.y_offset,
                crate::world2d::layer::OVERHEAD,
            );
            bar_transform.rotation = Quat::IDENTITY;
            continue;
        }
        // Normalized player models report a deterministic head height; prefer it
        // over per-frame AABB sampling (unstable for rigged/center-pivot meshes).
        let bar_world_y = match normalized_query
            .get(anchor.target)
            .ok()
            .and_then(|n| n.head_local_y)
        {
            Some(head_local_y) => {
                target_transform.translation().y + head_local_y + BAR_HEAD_CLEARANCE
            }
            None => compute_bar_world_y_for_entity(
                anchor.target,
                target_transform.translation().y + anchor.y_offset,
                &children_query,
                &aabb_query,
                &global_query,
            ),
        };
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
    mode: Res<PlayerVisualMode>,
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
    if *mode == PlayerVisualMode::Sprite2d {
        let xy = crate::world2d::simulation_xz_to_render_xy(target_translation);
        marker_transform.translation = Vec3::new(xy.x, xy.y, crate::world2d::layer::MARKER);
        marker_transform.rotation =
            Quat::from_rotation_z(time.elapsed_secs() * TARGET_MARKER_SPIN_SPEED);
        marker_transform.scale = Vec3::splat(marker_radius * pulse / (TARGET_MARKER_SIZE * 0.5));
        return;
    }
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
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
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
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
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
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
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
        if best.is_none_or(|(_, _, best_dist)| dist_sq < best_dist) {
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

fn find_target_near_screen(
    pointer_position: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    visual_mode: PlayerVisualMode,
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
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            PLAYER_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats, team) in minion_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            MINION_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats) in neutral_candidates.iter() {
        if !stats.is_alive() {
            continue;
        }
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            NEUTRAL_PICK_RADIUS_PX,
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
        );
    }

    for (entity, transform, id, stats, team, kind) in structure_candidates.iter() {
        if !stats.is_alive() || *team == local_team {
            continue;
        }
        let radius = match kind {
            StructureKind::Tower => TOWER_PICK_RADIUS_PX,
            StructureKind::BaseTower => BASE_TOWER_PICK_RADIUS_PX,
        };
        consider_screen_target(
            &mut best,
            pointer_position,
            camera,
            camera_transform,
            visual_mode,
            transform.translation,
            radius,
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
        );
    }

    best.map(|(entity, target, _)| (entity, target))
}

#[allow(clippy::too_many_arguments)]
fn consider_screen_target(
    best: &mut Option<(Entity, TargetId, f32)>,
    pointer_position: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    visual_mode: PlayerVisualMode,
    simulation_position: Vec3,
    pick_radius_px: f32,
    entity: Entity,
    target: TargetId,
) {
    let render_position = if visual_mode == PlayerVisualMode::Sprite2d {
        let xy = crate::world2d::simulation_xz_to_render_xy(simulation_position);
        Vec3::new(xy.x, xy.y, crate::world2d::layer::ACTOR)
    } else {
        simulation_position
    };
    let Ok(screen_position) = camera.world_to_viewport(camera_transform, render_position) else {
        return;
    };
    let Some(distance) = screen_pick_distance(pointer_position, screen_position, pick_radius_px)
    else {
        return;
    };
    if best.is_none_or(|(_, _, best_distance)| distance < best_distance) {
        *best = Some((entity, target, distance));
    }
}

fn screen_pick_distance(pointer: Vec2, actor_center: Vec2, radius_px: f32) -> Option<f32> {
    let distance = pointer.distance(actor_center);
    (distance <= radius_px).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_hit_areas_are_touch_sized_and_screen_bounded() {
        const {
            assert!(SKILL_SLOT_SIZE >= 48.0);
        }
        for radius in [
            PLAYER_PICK_RADIUS_PX,
            MINION_PICK_RADIUS_PX,
            NEUTRAL_PICK_RADIUS_PX,
            TOWER_PICK_RADIUS_PX,
            BASE_TOWER_PICK_RADIUS_PX,
        ] {
            assert!(radius >= 48.0);
            assert!(screen_pick_distance(Vec2::ZERO, Vec2::new(radius, 0.0), radius).is_some());
            assert!(
                screen_pick_distance(Vec2::ZERO, Vec2::new(radius + 0.1, 0.0), radius).is_none()
            );
        }
    }

    #[test]
    fn primary_target_request_is_q_for_the_exact_authoritative_target() {
        let entity = Entity::PLACEHOLDER;
        let target = TargetId {
            kind: TargetKind::Minion,
            id: 77,
        };
        let state = TargetState {
            selected_entity: Some(entity),
            selected_target: Some(target),
            marker_entity: None,
        };
        let mut pending = PendingCast::default();
        let mut console = DebugConsole::default();
        queue_cast_request(
            SkillSlot::Q.index(),
            HeroClass::Warrior,
            &state,
            &mut pending,
            &mut console,
        );
        assert_eq!(
            pending.request,
            Some(PendingCastRequest {
                slot: SkillSlot::Q.index(),
                target_entity: Some(entity),
                target: Some(target),
                approach_announced: false,
            })
        );
    }

    #[test]
    fn cast_range_uses_horizontal_gameplay_distance() {
        assert!(within_cast_range(
            Vec3::ZERO,
            Vec3::new(3.0, 99.0, 4.0),
            5.0
        ));
        assert!(!within_cast_range(
            Vec3::ZERO,
            Vec3::new(3.01, 0.0, 4.0),
            5.0
        ));
    }

    #[test]
    fn pending_unit_cast_approaches_then_emits_once_in_range() {
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .insert_resource(TeamSelection::default())
            .insert_resource(PendingCast::default())
            .insert_resource(LocalCastCooldown::default())
            .insert_resource(DebugConsole::default())
            .add_systems(Update, resolve_pending_cast_system);

        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CombatStats::default(),
                PlayerProgression::default(),
                NetworkPlayerId(1),
                NetworkHeroClass(HeroClass::Warrior),
            ))
            .id();
        let target = app
            .world_mut()
            .spawn((Transform::from_xyz(30.0, 0.0, 0.0), CombatStats::default()))
            .id();
        app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
            slot: SkillSlot::Q.index(),
            target_entity: Some(target),
            target: Some(TargetId {
                kind: TargetKind::Minion,
                id: 77,
            }),
            approach_announced: false,
        });

        app.update();
        assert!(app.world().entity(player).contains::<MovementTarget>());
        assert!(app.world().resource::<PendingCast>().request.is_some());
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs[0],
            0.0
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );

        app.world_mut()
            .entity_mut(player)
            .insert(Transform::from_xyz(20.0, 0.0, 0.0));
        app.update();
        assert!(app.world().resource::<PendingCast>().request.is_none());
        assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
        let commands: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            NetworkCommand::Cast {
                target: TargetId {
                    kind: TargetKind::Minion,
                    id: 77
                },
                slot: 0
            }
        ));
    }

    #[test]
    fn insufficient_mana_rejects_without_phantom_cooldown() {
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .insert_resource(TeamSelection::default())
            .insert_resource(PendingCast::default())
            .insert_resource(LocalCastCooldown::default())
            .insert_resource(DebugConsole::default())
            .add_systems(Update, resolve_pending_cast_system);

        let exhausted = CombatStats {
            mana: 0.0,
            ..default()
        };
        app.world_mut().spawn((
            Player,
            Transform::default(),
            exhausted,
            PlayerProgression::default(),
            NetworkPlayerId(1),
            NetworkHeroClass(HeroClass::Warrior),
        ));
        let target = app
            .world_mut()
            .spawn((Transform::from_xyz(2.0, 0.0, 0.0), CombatStats::default()))
            .id();
        app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
            slot: SkillSlot::Q.index(),
            target_entity: Some(target),
            target: Some(TargetId {
                kind: TargetKind::Minion,
                id: 88,
            }),
            approach_announced: false,
        });

        app.update();
        assert!(app.world().resource::<PendingCast>().request.is_none());
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs[0],
            0.0
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
    }

    #[test]
    fn self_target_hotbar_request_needs_no_selected_enemy() {
        let state = TargetState::default();
        let mut pending = PendingCast::default();
        let mut console = DebugConsole::default();
        queue_cast_request(
            SkillSlot::W.index(),
            HeroClass::Warrior,
            &state,
            &mut pending,
            &mut console,
        );
        assert_eq!(
            pending.request,
            Some(PendingCastRequest {
                slot: SkillSlot::W.index(),
                target_entity: None,
                target: None,
                approach_announced: false,
            })
        );
    }
}
