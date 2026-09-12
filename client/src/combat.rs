use bevy::{
    camera::primitives::Aabb, ecs::system::SystemParam, input::mouse::MouseButton,
    math::primitives::Rectangle, prelude::*, window::PrimaryWindow,
};

use crate::camera::MainCamera;
use crate::input_bindings::{SKILL_CAST_KEYS, SKILL_UPGRADE_KEY};
use crate::input_context::{GameplayInputContext, InputContextSet};
use crate::minimap::MinimapNavigationState;
use crate::model_scale::NormalizeModelScale;
use crate::net::{
    GameState, GameStateSnapshot, NetworkCommand, NetworkHeroClass, NetworkMinion, NetworkMinionId,
    NetworkNeutral, NetworkNeutralId, NetworkPlayerId, NetworkStructure, NetworkStructureId,
    PlayerProgression, RemotePlayer, StructureKind, TargetId, TargetKind,
};
use crate::player::{MovementTarget, Player};
use crate::sprite::PlayerVisualMode;
use crate::targeting::{BasicAttackState, TargetAimPreview};
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
    /// Total duration last applied to each active cooldown. Lets an equipment
    /// or rank update change the deadline without rescaling elapsed time.
    total_secs: [f32; 4],
}

/// One visible action message, replaced in place and expired after three seconds.
#[derive(Resource, Default)]
pub(crate) struct ActionFeedback {
    pub text: String,
    remaining: f32,
}

impl ActionFeedback {
    pub(crate) fn push_line(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.remaining = 3.0;
    }
}

#[derive(Component)]
struct ActionFeedbackText;

fn update_action_feedback(
    time: Res<Time>,
    mut feedback: ResMut<ActionFeedback>,
    mut labels: Query<(&mut Text, &mut Node), With<ActionFeedbackText>>,
) {
    feedback.remaining = (feedback.remaining - time.delta_secs()).max(0.0);
    if feedback.remaining == 0.0 {
        feedback.text.clear();
    }
    for (mut label, mut node) in &mut labels {
        label.0.clone_from(&feedback.text);
        node.display = if feedback.text.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
}

/// Keep valid identity across a transient disconnect (whose snapshot is empty).
#[derive(Resource, Default)]
struct CombatRoundIdentity(Option<(u64, u64)>);

fn reset_round_input_state(
    mut commands: Commands,
    snapshot: Res<GameStateSnapshot>,
    mut previous: ResMut<CombatRoundIdentity>,
    mut target: ResMut<TargetState>,
    orders: (ResMut<PendingCast>, ResMut<BasicAttackState>),
    mut cooldowns: ResMut<LocalCastCooldown>,
    mut feedback: ResMut<ActionFeedback>,
    moving: Query<Entity, With<MovementTarget>>,
    mut queued: ResMut<Messages<NetworkCommand>>,
) {
    let (mut pending, mut basic) = orders;
    let identity = (snapshot.meta.server_epoch, snapshot.meta.match_id);
    if identity.0 == 0 || identity.1 == 0 {
        return;
    }
    let changed = previous.0.is_some_and(|last| last != identity);
    previous.0 = Some(identity);
    if !changed {
        return;
    }
    target.selected_entity = None;
    target.selected_target = None;
    pending.cancel();
    *basic = default();
    cooldowns.remaining_secs = [0.0; 4];
    cooldowns.total_secs = [0.0; 4];
    feedback.text.clear();
    feedback.remaining = 0.0;
    for entity in &moving {
        commands.entity(entity).remove::<MovementTarget>();
    }
    let preserved: Vec<_> = queued
        .drain()
        .filter(|command| {
            !matches!(
                command,
                NetworkCommand::BasicAttack { .. }
                    | NetworkCommand::Cast { .. }
                    | NetworkCommand::UpgradeSkill { .. }
                    | NetworkCommand::BuyItem { .. }
            )
        })
        .collect();
    queued.write_batch(preserved);
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
const SKILL_SLOT_SIZE: f32 = 100.0;
const SKILL_SLOT_GAP: f32 = 8.0;
const SKILL_BUTTON_COLOR: Color = crate::ui_theme::PANEL;
const SKILL_BUTTON_HOVER_COLOR: Color = crate::ui_theme::HOVER;
const SKILL_BUTTON_PRESS_COLOR: Color = crate::ui_theme::TILE;
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
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<WorldPointerState>()
            .init_resource::<PendingCast>()
            .init_resource::<ActionFeedback>()
            .init_resource::<CombatRoundIdentity>()
            .add_systems(
                Update,
                reset_round_input_state
                    .after(crate::net::ClientNetPipeline::ApplySnapshot)
                    .before(InputContextSet::Modal),
            )
            .add_systems(Startup, setup_combat_visual_assets)
            .add_systems(
                Startup,
                (setup_combat_ui, crate::targeting::setup_targeting_ui),
            )
            .add_systems(
                Update,
                select_target_system
                    .in_set(CombatPointerInputSet)
                    .in_set(InputContextSet::Actions),
            )
            .add_systems(
                Update,
                (
                    tick_local_cast_cooldown,
                    crate::targeting::tick_basic_attack,
                    sync_authoritative_cooldown_durations,
                    update_action_feedback,
                    crate::targeting::clear_invalid_selection,
                    cast_spell_system,
                    skill_button_system,
                    mobile_cast_system,
                    crate::targeting::mobile_basic_attack,
                    crate::targeting::resolve_basic_attack,
                    resolve_pending_cast_system,
                    skill_upgrade_input_system,
                    update_skill_bar_system,
                    adapt_mobile_combat_feedback,
                    update_target_marker_system,
                    crate::targeting::draw_targeting_ui,
                    crate::targeting::draw_locked_target,
                )
                    .chain()
                    .after(WorldMovementInputSet)
                    .in_set(InputContextSet::Actions),
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
    hp_local_material: Handle<StandardMaterial>,
    hp_friendly_material: Handle<StandardMaterial>,
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
    pub(crate) consumed_secondary_press: bool,
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
    pub(crate) fn is_pending(&self) -> bool {
        self.request.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.request = None;
    }

    #[cfg(test)]
    pub(crate) fn queued_for_movement_test() -> Self {
        Self {
            request: Some(PendingCastRequest {
                slot: 0,
                target_entity: None,
                target: None,
                approach_announced: true,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn has_queued_request(&self) -> bool {
        self.request.is_some()
    }
}

#[derive(SystemParam)]
pub(crate) struct TargetCandidates<'w, 's> {
    pub(crate) players: Query<
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
    pub(crate) structures: Query<
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
    pub(crate) minions: Query<
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
    pub(crate) neutrals: Query<
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

/// Mirror the server's deadline: last cast time + current kit/item duration.
/// Reducing haste after elapsed time must subtract the duration difference,
/// rather than multiplying the remaining time (which incorrectly rescales history).
fn sync_authoritative_cooldown_durations(
    player: Query<
        (
            &PlayerProgression,
            &NetworkHeroClass,
            &crate::net::PlayerEquipment,
        ),
        With<Player>,
    >,
    mut cooldowns: ResMut<LocalCastCooldown>,
) {
    let Ok((progression, class, equipment)) = player.single() else {
        return;
    };
    for slot in SkillSlot::ALL {
        let index = slot.index();
        let definition = ability_for_class_slot(class.0, slot);
        let duration = shared::shop::item_cooldown(
            definition,
            progression.ranks[index],
            slot,
            equipment.item_bonuses,
        )
        .as_secs_f32();
        if cooldowns.remaining_secs[index] > 0.0
            && cooldowns.total_secs[index] > 0.0
            && cooldowns.total_secs[index] != duration
        {
            cooldowns.remaining_secs[index] =
                (cooldowns.remaining_secs[index] + duration - cooldowns.total_secs[index]).max(0.0);
        }
        cooldowns.total_secs[index] = duration;
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
    let hp_local_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.85, 0.2),
        unlit: true,
        ..default()
    });
    let hp_friendly_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.85, 0.6),
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
        hp_local_material,
        hp_friendly_material,
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
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(178.0),
            left: Val::Px(264.0),
            max_width: Val::Px(490.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.88, 0.5)),
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.8)),
        ZIndex(14),
        ActionFeedbackText,
        Name::new("ActionFeedback"),
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(264.0),
                bottom: Val::Px(24.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(SKILL_SLOT_GAP),
                align_items: AlignItems::FlexEnd,
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
                            Text::new("+  UPGRADE"),
                            TextFont {
                                font_size: 11.0,
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
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(8.0)),
                            padding: UiRect::all(Val::Px(4.0)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            row_gap: Val::Px(1.0),
                            ..default()
                        },
                        BackgroundColor(SKILL_BUTTON_COLOR),
                        BorderColor::all(crate::ui_theme::EDGE),
                        SkillBarSlot { slot: i },
                        Name::new(format!("SkillSlot-{label}")),
                    ))
                    .with_children(|slot| {
                        slot.spawn((
                            Text::new(label),
                            TextLayout::new_with_justify(Justify::Center),
                            TextFont {
                                font_size: 24.0,
                                ..default()
                            },
                            TextColor::WHITE,
                        ));
                        slot.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(Color::srgba(0.88, 0.90, 0.94, 1.0)),
                            TextLayout::new_with_justify(Justify::Center),
                            SkillNameLabel { slot: i },
                        ));
                        slot.spawn((
                            Text::new("Lv 1"),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(Color::srgba(0.82, 0.84, 0.90, 1.0)),
                            TextLayout::new_with_justify(Justify::Center),
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
    progression: Query<
        (
            &PlayerProgression,
            Option<&NetworkHeroClass>,
            &CombatStats,
            &Transform,
        ),
        With<Player>,
    >,
    cooldowns: Res<LocalCastCooldown>,
    pending: Res<PendingCast>,
    target: Res<TargetState>,
    targets: Query<&Transform, Without<Player>>,
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
    let prog = local.map(|(prog, ..)| *prog).unwrap_or_default();
    let class = local_hero_class(local.map(|(_, class, ..)| class), &team_selection);

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
        let slot = SkillSlot::from_index(label.slot as u8).expect("hotbar slot");
        let definition = ability_for_class_slot(class, slot);
        let cost = scaled_mana_cost(definition, rank);
        let status = if !unlocked_slots_for_level(prog.level.max(1))[label.slot] {
            format!("Locked Lv {}", shared::SLOT_UNLOCK_LEVELS[label.slot])
        } else if cooldowns.remaining_secs[label.slot] > 0.0 {
            format!("{:.1}s cooldown", cooldowns.remaining_secs[label.slot])
        } else if local.is_some_and(|(_, _, stats, _)| stats.mana < cost) {
            "Need mana".to_string()
        } else if pending
            .request
            .is_some_and(|request| request.slot == label.slot && request.approach_announced)
        {
            "Approaching".to_string()
        } else if definition.targeting == TargetingMode::UnitTarget
            && target.selected_entity.is_none()
        {
            "Select target".to_string()
        } else if definition.targeting == TargetingMode::UnitTarget
            && local
                .zip(
                    target
                        .selected_entity
                        .and_then(|entity| targets.get(entity).ok()),
                )
                .is_some_and(|((_, _, _, player), target)| {
                    !within_cast_range(
                        player.translation,
                        target.translation,
                        scaled_cast_range(definition, rank),
                    )
                })
        {
            "Out of range".to_string()
        } else {
            "Ready".to_string()
        };
        let next = format!("Rank {rank} | {cost:.0} MP\n{status}");
        if text.0 != next {
            text.0 = next;
        }
    }

    for (button, interaction, mut color, mut node) in &mut upgrade_buttons {
        let rank = prog.ranks.get(button.slot).copied().unwrap_or(1).max(1);
        let can_upgrade = prog.skill_points > 0
            && rank < MAX_ABILITY_RANK
            && unlocked_slots_for_level(prog.level.max(1))[button.slot];
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
    context: Res<GameplayInputContext>,
) {
    if !context.gameplay_allowed() {
        return;
    }

    let Some(prog) = progression.iter().next() else {
        return;
    };
    let unlocked = unlocked_slots_for_level(prog.level.max(1));
    let eligible = |slot: usize| {
        prog.skill_points > 0 && unlocked[slot] && prog.ranks[slot] < MAX_ABILITY_RANK
    };
    if keyboard.just_pressed(SKILL_UPGRADE_KEY) {
        if let Some(slot) = (0..4).find(|slot| eligible(*slot)) {
            command_writer.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    for (button, interaction) in &upgrade_buttons {
        if matches!(interaction, Interaction::Pressed) && eligible(button.slot) {
            command_writer.write(NetworkCommand::UpgradeSkill {
                slot: button.slot as u8,
            });
        }
    }
}

fn select_target_system(
    input: (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
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
    context: Res<GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut basic: ResMut<BasicAttackState>,
    validity: crate::targeting::TargetValidity,
) {
    let (keyboard, mouse) = input;
    *pointer_state = default();
    if mobile.as_ref().is_some_and(|m| m.enabled)
        || !context.gameplay_allowed()
        || game_state
            .as_ref()
            .is_some_and(|g| !matches!(g.state, GameState::Running))
    {
        return;
    }
    let Ok((local_transform, local_team)) = local_player.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };
    if !window.focused {
        return;
    }
    let primary = mouse.just_pressed(MouseButton::Left);
    let secondary = mouse.just_pressed(MouseButton::Right)
        && !keyboard.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let mut picked = if keyboard.just_pressed(KeyCode::Tab) {
        find_nearest_enemy_target(
            local_transform.translation,
            *local_team,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        )
    } else {
        None
    };
    if primary || secondary {
        let blocked = ui_interactions.iter().any(|i| *i != Interaction::None)
            || minimap_nav
                .as_ref()
                .is_some_and(|n| n.consumed_primary_click);
        if blocked {
            return;
        }
        let (Some(position), Ok((camera, transform))) =
            (window.cursor_position(), camera_query.single())
        else {
            return;
        };
        picked = find_target_near_screen(
            position,
            camera,
            transform,
            *visual_mode,
            *local_team,
            &validity,
            &candidates.players,
            &candidates.minions,
            &candidates.neutrals,
            &candidates.structures,
        );
    }
    picked = picked.filter(|(entity, id)| validity.valid(*entity, *id, *local_team));
    if let Some((entity, id)) = picked {
        target_state.selected_entity = Some(entity);
        target_state.selected_target = Some(id);
        pending_cast.cancel();
        if secondary {
            pointer_state.consumed_secondary_press = true;
            basic.start(entity, id, true);
        } else {
            pointer_state.consumed_primary_press = primary;
            basic.cancel();
        }
    } else if primary {
        target_state.selected_entity = None;
        target_state.selected_target = None;
        pending_cast.cancel();
        basic.cancel();
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
    feedback: &mut ActionFeedback,
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
        feedback.push_line(message.clone());
        info!("{message}");
        return false;
    }
    if cast_cd.remaining_secs[slot.index()] > 0.0 {
        let message = format!(
            "{} is cooling down for {:.1}s.",
            def.name,
            cast_cd.remaining_secs[slot.index()]
        );
        feedback.push_line(message.clone());
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
        feedback.push_line(message);
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
        feedback.push_line(message.clone());
        info!("{message}");
        return false;
    }
    cast_cd.remaining_secs[slot.index()] = scaled_cooldown(def, rank).as_secs_f32();
    cast_cd.total_secs[slot.index()] = cast_cd.remaining_secs[slot.index()];
    command_writer.write(NetworkCommand::Cast {
        target,
        slot: slot.index() as u8,
    });
    let message = format!("Casting {}.", def.name);
    feedback.push_line(message.clone());
    info!("{message}");
    true
}

fn queue_cast_request(
    slot_index: usize,
    class: HeroClass,
    target_state: &TargetState,
    pending_cast: &mut PendingCast,
    feedback: &mut ActionFeedback,
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
                feedback.push_line(message);
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
    mut feedback: ResMut<ActionFeedback>,
    context: Res<GameplayInputContext>,
) {
    if !context.gameplay_allowed() {
        return;
    }

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
        &mut feedback,
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
    mut feedback: ResMut<ActionFeedback>,
    context: Res<GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
) {
    if !context.gameplay_allowed() || mobile.as_ref().is_some_and(|mobile| mobile.enabled) {
        return;
    }

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
                    &mut feedback,
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
            &Team,
        ),
        With<Player>,
    >,
    target_query: Query<(&Transform, &CombatStats), Without<Player>>,
    mut pending_cast: ResMut<PendingCast>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut feedback: ResMut<ActionFeedback>,
    mut cast_cd: ResMut<LocalCastCooldown>,
    context: Res<GameplayInputContext>,
    protection: Query<&crate::net::NetworkStructureProtected>,
    equipment: Query<&crate::net::PlayerEquipment, With<Player>>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    validity: crate::targeting::TargetValidity,
) {
    let touch_mode = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    if !context.gameplay_allowed() {
        pending_cast.cancel();
        return;
    }

    let Some(request) = pending_cast.request else {
        return;
    };
    let Ok((player_entity, player_transform, stats, progression, net_id, class, team)) =
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
    let prog = progression.copied().unwrap_or_default();
    let rank = prog.ranks[slot.index()].clamp(1, definition.max_rank);
    let rejection = if !stats.is_alive() {
        Some("Wait for respawn.".to_string())
    } else if !unlocked_slots_for_level(prog.level.max(1))[slot.index()] {
        Some(format!(
            "{} unlocks at level {}.",
            definition.name,
            shared::SLOT_UNLOCK_LEVELS[slot.index()]
        ))
    } else if cast_cd.remaining_secs[slot.index()] > 0.0 {
        Some(format!(
            "{} ready in {:.1}s.",
            definition.name,
            cast_cd.remaining_secs[slot.index()]
        ))
    } else if stats.mana < scaled_mana_cost(definition, rank) {
        Some(format!(
            "Not enough mana for {} ({:.0}/{:.0}).",
            definition.name,
            stats.mana,
            scaled_mana_cost(definition, rank)
        ))
    } else if request
        .target_entity
        .and_then(|entity| protection.get(entity).ok())
        .is_some_and(|protected| protected.0)
    {
        Some("Structure protected — destroy the preceding lane towers first.".to_string())
    } else if definition.targeting == TargetingMode::UnitTarget
        && request
            .target_entity
            .zip(request.target)
            .is_none_or(|(entity, id)| !validity.valid(entity, id, *team))
    {
        Some("Target is no longer a visible hostile unit.".to_string())
    } else {
        None
    };
    if let Some(message) = rejection {
        feedback.push_line(message);
        pending_cast.cancel();
        commands
            .entity(player_entity)
            .remove::<(MovementTarget, crate::player::MovementRoute)>();
        return;
    }

    if definition.targeting == TargetingMode::UnitTarget {
        let (Some(target_entity), Some(_target)) = (request.target_entity, request.target) else {
            pending_cast.cancel();
            return;
        };
        let Ok((target_transform, target_stats)) = target_query.get(target_entity) else {
            pending_cast.cancel();
            commands
                .entity(player_entity)
                .remove::<(MovementTarget, crate::player::MovementRoute)>();
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
            if touch_mode {
                feedback.push_line("Target out of range — move closer.");
                pending_cast.cancel();
                return;
            }
            commands.entity(player_entity).insert(MovementTarget {
                target: target_transform.translation,
            });
            if !request.approach_announced {
                let message = format!("Approaching target for {}.", definition.name);
                feedback.push_line(message.clone());
                info!("{message}");
                if let Some(request) = pending_cast.request.as_mut() {
                    request.approach_announced = true;
                }
            }
            return;
        }
    }

    commands.entity(player_entity).remove::<MovementTarget>();
    let sent = try_cast_slot(
        request.slot,
        class,
        (stats, progression.copied().unwrap_or_default(), net_id),
        request.target,
        &mut command_writer,
        &mut feedback,
        &mut cast_cd,
    );
    if sent {
        let bonuses = equipment
            .single()
            .map(|equipment| equipment.item_bonuses)
            .unwrap_or_default();
        cast_cd.remaining_secs[slot.index()] =
            shared::shop::item_cooldown(definition, rank, slot, bonuses).as_secs_f32();
        cast_cd.total_secs[slot.index()] = cast_cd.remaining_secs[slot.index()];
    }
    pending_cast.cancel();
}

/// Mobile abilities share the existing PendingCast/try_cast_slot path. The
/// assistance step changes only target choice, never range, mana or cooldowns.
#[allow(clippy::type_complexity)]
fn mobile_cast_system(
    mut mobile: Option<ResMut<crate::mobile_controls::MobileControls>>,
    context: Res<GameplayInputContext>,
    selection: Res<TeamSelection>,
    local: Query<
        (
            &Transform,
            &Team,
            &CombatStats,
            Option<&PlayerProgression>,
            Option<&NetworkHeroClass>,
        ),
        With<Player>,
    >,
    candidates: TargetCandidates,
    validity: crate::targeting::TargetValidity,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    visual_mode: Res<PlayerVisualMode>,
    target: Res<TargetState>,
    mut pending: ResMut<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
    mut commands: MessageWriter<NetworkCommand>,
) {
    let Some(mobile) = mobile.as_deref_mut().filter(|mobile| mobile.enabled) else {
        return;
    };
    if !context.gameplay_allowed() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let Ok((transform, team, stats, prog, class)) = local.single() else {
        return;
    };
    if !stats.is_alive() {
        mobile.casts.clear();
        mobile.upgrades.clear();
        return;
    }
    let prog = prog.copied().unwrap_or_default();
    for slot in mobile.upgrades.drain(..) {
        if slot < 4
            && prog.skill_points > 0
            && prog.ranks[slot] < MAX_ABILITY_RANK
            && unlocked_slots_for_level(prog.level.max(1))[slot]
        {
            commands.write(NetworkCommand::UpgradeSkill { slot: slot as u8 });
        }
    }
    let class = class.map(|class| class.0).unwrap_or(selection.hero_class);
    let intent = mobile.casts.drain(..).next_back();
    let Some(intent) = intent else {
        return;
    };
    let Some(slot) = SkillSlot::from_index(intent.slot as u8) else {
        return;
    };
    let definition = ability_for_class_slot(class, slot);
    if definition.targeting == TargetingMode::UnitTarget {
        let Ok((camera, camera_transform)) = camera.single() else {
            return;
        };
        let range = scaled_cast_range(definition, prog.ranks[intent.slot].max(1));
        let pick = if intent.aim.is_none() && target.selected_entity.is_some() {
            // An explicit lock wins even when currently out of range. The skill
            // resolver reports that range error instead of hitting another foe.
            target.selected_entity.zip(target.selected_target)
        } else {
            mobile_assisted_target(
                transform.translation,
                *team,
                range,
                intent.aim,
                &candidates,
                &validity,
                camera,
                camera_transform,
                *visual_mode,
                target.selected_entity,
            )
        };
        let Some((entity, id)) = pick else {
            feedback.push_line("No enemy in that direction and range.");
            return;
        };
        let request_target = TargetState {
            selected_entity: Some(entity),
            selected_target: Some(id),
            ..default()
        };
        queue_cast_request(
            intent.slot,
            class,
            &request_target,
            &mut pending,
            &mut feedback,
        );
        return;
    }
    queue_cast_request(intent.slot, class, &target, &mut pending, &mut feedback);
}

#[allow(clippy::too_many_arguments)]
fn mobile_assisted_target(
    position: Vec3,
    team: Team,
    range: f32,
    aim: Option<Vec2>,
    candidates: &TargetCandidates,
    validity: &crate::targeting::TargetValidity,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    mode: PlayerVisualMode,
    selected: Option<Entity>,
) -> Option<(Entity, TargetId)> {
    let screen = |p: Vec3| {
        let render = if mode == PlayerVisualMode::Sprite2d {
            crate::world2d::simulation_xz_to_render_xy(p).extend(0.0)
        } else {
            p
        };
        camera.world_to_viewport(camera_transform, render).ok()
    };
    let origin = screen(position)?;
    let viewport = camera.logical_viewport_size()?;
    let mut best: Option<(Entity, TargetId, f32)> = None;
    let mut consider = |entity: Entity, id: TargetId, p: Vec3, stats: &CombatStats, enemy: bool| {
        if !enemy || !stats.is_alive() || !validity.valid(entity, id, team) {
            return;
        }
        let distance = position.xz().distance(p.xz());
        if distance > range {
            return;
        }
        let Some(projected) = screen(p) else {
            return;
        };
        if projected.x < 0.0
            || projected.y < 0.0
            || projected.x > viewport.x
            || projected.y > viewport.y
        {
            return;
        }
        let Some(score) = mobile_target_score(
            distance,
            range,
            projected - origin,
            aim,
            selected == Some(entity),
        ) else {
            return;
        };
        if best.is_none_or(|(_, _, previous)| score < previous) {
            best = Some((entity, id, score));
        }
    };
    for (e, t, id, s, target_team) in &candidates.players {
        consider(
            e,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s, target_team) in &candidates.minions {
        consider(
            e,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    for (e, t, id, s) in &candidates.neutrals {
        consider(
            e,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            t.translation,
            s,
            true,
        );
    }
    for (e, t, id, s, target_team, _) in &candidates.structures {
        consider(
            e,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            t.translation,
            s,
            *target_team != team,
        );
    }
    best.map(|(entity, id, _)| (entity, id))
}

fn mobile_target_score(
    distance: f32,
    range: f32,
    screen_delta: Vec2,
    aim: Option<Vec2>,
    selected: bool,
) -> Option<f32> {
    if !distance.is_finite() || distance > range || range <= 0.0 {
        return None;
    }
    if let Some(aim) = aim {
        let alignment = screen_delta
            .normalize_or_zero()
            .dot(aim.normalize_or_zero());
        // Directional assist uses a 45-degree half cone and has no fallback
        // behind the player when the requested direction contains no enemy.
        (alignment >= std::f32::consts::FRAC_1_SQRT_2)
            .then_some((1.0 - alignment) * 4.0 + distance / range)
    } else {
        Some(distance / range - if selected { 2.0 } else { 0.0 })
    }
}

fn adapt_mobile_combat_feedback(
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut feedback: Query<(&mut Node, &mut TextFont), With<ActionFeedbackText>>,
) {
    let Some(mobile) = mobile.filter(|mobile| mobile.enabled) else {
        return;
    };
    for (mut node, mut font) in &mut feedback {
        let width = (mobile.viewport.x * 0.38).min(340.0);
        node.left = Val::Px((mobile.viewport.x - width) * 0.5);
        node.bottom = Val::Px(mobile.safe.bottom + 84.0 * mobile.scale());
        node.max_width = Val::Px(width);
        font.font_size = 13.0 * mobile.scale();
    }
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
    owners: Query<(&CombatStats, &CombatBars, Option<&Team>, Has<Player>)>,
    local_team: Query<&Team, With<Player>>,
    assets: Res<CombatVisualAssets>,
    mut fills: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    for (stats, bars, team, is_local) in owners.iter() {
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
            if let Ok(mut material) = fills.get_mut(hp_fill) {
                material.0 = if is_local {
                    assets.hp_local_material.clone()
                } else if team
                    .zip(local_team.single().ok())
                    .is_some_and(|(team, local)| team == local)
                {
                    assets.hp_friendly_material.clone()
                } else {
                    assets.hp_fill_material.clone()
                };
            }
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
    validity: &crate::targeting::TargetValidity,
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
    validity: &crate::targeting::TargetValidity,
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
        if !validity.valid(
            entity,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            local_team,
        ) {
            continue;
        }
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
    fn hidden_and_protected_nearest_candidates_do_not_mask_visible_targets() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = App::new();
        let hidden = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(0.1, 0.0, 0.0),
                Team::Blue,
                NetworkPlayerId(2),
                CombatStats::default(),
                InheritedVisibility::HIDDEN,
            ))
            .id();
        app.world_mut().spawn((
            NetworkStructure,
            Transform::from_xyz(0.05, 0.0, 0.0),
            Team::Blue,
            NetworkStructureId(3),
            StructureKind::BaseTower,
            CombatStats::default(),
            crate::net::NetworkStructureProtected(true),
        ));
        let visible = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(0.12, 0.0, 0.0),
                Team::Blue,
                NetworkPlayerId(4),
                CombatStats::default(),
                InheritedVisibility::VISIBLE,
            ))
            .id();
        let camera = Camera {
            computed: ComputedCameraValues {
                clip_from_view: Mat4::IDENTITY,
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 400),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        };
        let mut params = bevy::ecs::system::SystemState::<(
            TargetCandidates,
            crate::targeting::TargetValidity,
        )>::new(app.world_mut());
        let (candidates, validity) = params.get(app.world());
        let expected = Some((
            visible,
            TargetId {
                kind: TargetKind::Player,
                id: 4,
            },
        ));
        assert_eq!(
            find_nearest_enemy_target(
                Vec3::ZERO,
                Team::Green,
                &validity,
                &candidates.players,
                &candidates.minions,
                &candidates.neutrals,
                &candidates.structures
            ),
            expected
        );
        assert_eq!(
            find_target_near_screen(
                Vec2::new(440.0, 200.0),
                &camera,
                &GlobalTransform::IDENTITY,
                PlayerVisualMode::Models3d,
                Team::Green,
                &validity,
                &candidates.players,
                &candidates.minions,
                &candidates.neutrals,
                &candidates.structures
            ),
            expected
        );
        assert_eq!(
            mobile_assisted_target(
                Vec3::ZERO,
                Team::Green,
                10.0,
                None,
                &candidates,
                &validity,
                &camera,
                &GlobalTransform::IDENTITY,
                PlayerVisualMode::Models3d,
                Some(hidden)
            ),
            expected
        );
    }

    #[test]
    fn invalidated_pending_skill_stops_chase_without_cast_or_cooldown() {
        for invalidation in ["hidden", "friendly", "identity"] {
            for distance in [2.0, 30.0] {
                let mut app = App::new();
                app.add_message::<NetworkCommand>()
                    .init_resource::<TeamSelection>()
                    .init_resource::<PendingCast>()
                    .init_resource::<ActionFeedback>()
                    .init_resource::<LocalCastCooldown>()
                    .init_resource::<GameplayInputContext>()
                    .add_systems(Update, resolve_pending_cast_system);
                let player = app
                    .world_mut()
                    .spawn((
                        Player,
                        Transform::default(),
                        Team::Green,
                        CombatStats::default(),
                        PlayerProgression::default(),
                        NetworkPlayerId(1),
                        NetworkHeroClass(HeroClass::Warrior),
                    ))
                    .id();
                let target = app
                    .world_mut()
                    .spawn((
                        Transform::from_xyz(30.0, 0.0, 0.0),
                        Team::Blue,
                        CombatStats::default(),
                        NetworkMinionId(77),
                        InheritedVisibility::VISIBLE,
                    ))
                    .id();
                app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
                    slot: 0,
                    target_entity: Some(target),
                    target: Some(TargetId {
                        kind: TargetKind::Minion,
                        id: 77,
                    }),
                    approach_announced: false,
                });
                app.update();
                assert!(app.world().entity(player).contains::<MovementTarget>());
                app.world_mut()
                    .entity_mut(player)
                    .insert(crate::player::MovementRoute {
                        requested_target: Vec3::X * 30.0,
                        structure_revision: 1,
                        destination: Vec3::X * 30.0,
                        waypoints: vec![Vec3::X * 30.0],
                    });
                app.world_mut()
                    .entity_mut(target)
                    .insert(Transform::from_xyz(distance, 0.0, 0.0));
                match invalidation {
                    "hidden" => {
                        app.world_mut()
                            .entity_mut(target)
                            .insert(InheritedVisibility::HIDDEN);
                    }
                    "friendly" => {
                        app.world_mut().entity_mut(target).insert(Team::Green);
                    }
                    _ => {
                        app.world_mut()
                            .entity_mut(target)
                            .insert(NetworkMinionId(78));
                    }
                }
                app.update();
                assert!(app.world().resource::<PendingCast>().request.is_none());
                assert!(!app.world().entity(player).contains::<MovementTarget>());
                assert!(
                    !app.world()
                        .entity(player)
                        .contains::<crate::player::MovementRoute>()
                );
                assert_eq!(
                    app.world().resource::<LocalCastCooldown>().remaining_secs,
                    [0.0; 4]
                );
                assert!(
                    app.world_mut()
                        .resource_mut::<Messages<NetworkCommand>>()
                        .drain()
                        .next()
                        .is_none(),
                    "{invalidation} target at {distance} must not receive a queued skill"
                );
                assert!(
                    app.world()
                        .resource::<ActionFeedback>()
                        .text
                        .contains("visible hostile")
                );
            }
        }
    }

    #[test]
    fn desktop_mouse_select_attack_ground_and_ui_are_distinct() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<TargetState>()
            .init_resource::<PendingCast>()
            .init_resource::<WorldPointerState>()
            .init_resource::<BasicAttackState>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Update, select_target_system);
        let mut window = Window {
            resolution: bevy::window::WindowResolution::new(800, 400),
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(440.0, 200.0)));
        let window = app.world_mut().spawn((window, PrimaryWindow)).id();
        app.world_mut()
            .spawn((Player, Transform::default(), Team::Green));
        let enemy = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(0.1, 0.0, 0.0),
                Team::Blue,
                NetworkPlayerId(2),
                CombatStats::default(),
            ))
            .id();
        app.world_mut().spawn((
            MainCamera,
            GlobalTransform::IDENTITY,
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: Mat4::IDENTITY,
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 400),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(enemy)
        );
        assert!(app.world().resource::<BasicAttackState>().order.is_none());
        assert!(!app.world().resource::<PendingCast>().is_pending());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        let attack = app.world().resource::<BasicAttackState>().order.unwrap();
        assert_eq!(attack.entity, enemy);
        assert_eq!(attack.target.id, 2);
        assert!(attack.repeat);
        assert!(
            app.world()
                .resource::<WorldPointerState>()
                .consumed_secondary_press
        );
        assert!(!app.world().resource::<PendingCast>().is_pending());
        app.world_mut().resource_mut::<BasicAttackState>().cancel();
        let ui = app.world_mut().spawn((Button, Interaction::Hovered)).id();
        app.update();
        assert!(app.world().resource::<BasicAttackState>().order.is_none());
        app.world_mut().despawn(ui);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::AltLeft);
        app.update();
        assert!(app.world().resource::<BasicAttackState>().order.is_none());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(700.0, 100.0)));
        app.update();
        assert!(
            !app.world()
                .resource::<WorldPointerState>()
                .consumed_secondary_press
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
    }

    #[test]
    fn directional_mobile_assist_respects_range_and_does_not_snap_behind_aim() {
        assert!(mobile_target_score(6.0, 5.0, Vec2::X, Some(Vec2::X), false).is_none());
        assert!(mobile_target_score(3.0, 5.0, Vec2::NEG_X, Some(Vec2::X), true).is_none());
        assert!(mobile_target_score(3.0, 5.0, Vec2::Y, Some(Vec2::X), false).is_none());
        let forward = mobile_target_score(3.0, 5.0, Vec2::X, Some(Vec2::X), false).unwrap();
        let edge =
            mobile_target_score(3.0, 5.0, Vec2::new(1.0, 0.5), Some(Vec2::X), false).unwrap();
        assert!(forward < edge);
        assert!(
            mobile_target_score(4.0, 5.0, Vec2::X, None, true)
                < mobile_target_score(1.0, 5.0, Vec2::X, None, false)
        );
    }

    #[test]
    fn mobile_pending_cast_in_range_emits_and_out_of_range_never_starts_a_chase() {
        let mut app = App::new();
        let mut mobile = crate::mobile_controls::MobileControls::default();
        mobile.enabled = true;
        app.insert_resource(mobile)
            .add_message::<NetworkCommand>()
            .init_resource::<TeamSelection>()
            .init_resource::<PendingCast>()
            .init_resource::<ActionFeedback>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, resolve_pending_cast_system);
        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats::default(),
                PlayerProgression::default(),
                NetworkHeroClass(HeroClass::Warrior),
                NetworkPlayerId(1),
                Team::Green,
            ))
            .id();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::from_xyz(100.0, 0.0, 0.0),
                CombatStats::default(),
                Team::Blue,
                NetworkPlayerId(2),
            ))
            .id();
        let request = PendingCastRequest {
            slot: 0,
            target_entity: Some(enemy),
            target: Some(TargetId {
                kind: TargetKind::Player,
                id: 2,
            }),
            approach_announced: false,
        };
        app.world_mut().resource_mut::<PendingCast>().request = Some(request);
        app.update();
        assert!(app.world().resource::<PendingCast>().request.is_none());
        assert!(!app.world().entity(player).contains::<MovementTarget>());
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs,
            [0.0; 4]
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
        app.world_mut()
            .entity_mut(enemy)
            .get_mut::<Transform>()
            .unwrap()
            .translation = Vec3::X;
        app.world_mut().resource_mut::<PendingCast>().request = Some(request);
        app.update();
        let emitted: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert_eq!(emitted.len(), 1);
        assert!(matches!(emitted[0], NetworkCommand::Cast { slot: 0, .. }));
        assert!(app.world().resource::<LocalCastCooldown>().remaining_secs[0] > 0.0);
        assert!(!app.world().entity(player).contains::<MovementTarget>());
    }

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
    fn explicit_q_skill_request_preserves_the_exact_authoritative_target() {
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
        let mut feedback = ActionFeedback::default();
        queue_cast_request(
            SkillSlot::Q.index(),
            HeroClass::Warrior,
            &state,
            &mut pending,
            &mut feedback,
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
            .insert_resource(ActionFeedback::default())
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, resolve_pending_cast_system);

        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CombatStats::default(),
                PlayerProgression::default(),
                NetworkPlayerId(1),
                Team::Green,
                NetworkHeroClass(HeroClass::Warrior),
            ))
            .id();
        let target = app
            .world_mut()
            .spawn((
                Transform::from_xyz(30.0, 0.0, 0.0),
                CombatStats::default(),
                Team::Blue,
                NetworkMinionId(77),
            ))
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
            .insert_resource(ActionFeedback::default())
            .init_resource::<GameplayInputContext>()
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
            Team::Green,
            NetworkHeroClass(HeroClass::Warrior),
        ));
        let target = app
            .world_mut()
            .spawn((
                Transform::from_xyz(2.0, 0.0, 0.0),
                CombatStats::default(),
                Team::Blue,
                NetworkMinionId(88),
            ))
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
        let mut feedback = ActionFeedback::default();
        queue_cast_request(
            SkillSlot::W.index(),
            HeroClass::Warrior,
            &state,
            &mut pending,
            &mut feedback,
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

    #[test]
    fn actual_cast_and_upgrade_systems_obey_help_pause_and_debug_context() {
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<TeamSelection>()
            .init_resource::<PendingCast>()
            .init_resource::<TargetState>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<ActionFeedback>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<crate::pause_menu::PauseMenuState>()
            .insert_resource(GameStateSnapshot {
                state: GameState::Running,
                ..default()
            })
            .add_plugins((
                crate::input_context::InputContextPlugin,
                crate::help_overlay::HelpOverlayPlugin,
            ))
            .add_systems(
                Update,
                (
                    cast_spell_system,
                    skill_upgrade_input_system,
                    resolve_pending_cast_system,
                )
                    .chain()
                    .in_set(InputContextSet::Actions),
            );
        app.world_mut().spawn((
            Player,
            Transform::default(),
            CombatStats::default(),
            PlayerProgression {
                level: 6,
                skill_points: 2,
                ..default()
            },
            NetworkHeroClass(HeroClass::Cleric),
            NetworkPlayerId(1),
            Team::Green,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyU);
        app.update(); // Automatic first-match help must suppress these same-frame keys.
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
        assert!(app.world().resource::<GameplayInputContext>().modal_open);
        app.world_mut()
            .resource_mut::<crate::help_overlay::HelpOverlayVisible>()
            .0 = false;
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = true;
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<crate::pause_menu::PauseMenuState>()
            .open = false;
        let mut debug = crate::debug_console::DebugConsole::default();
        debug.ui_enabled = true;
        app.insert_resource(debug);
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .debug_flight = true;
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .debug_flight = false;
        app.update();
        let emitted: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        assert!(
            emitted
                .iter()
                .any(|command| matches!(command, NetworkCommand::Cast { slot: 1, .. }))
        );
        assert!(
            emitted
                .iter()
                .any(|command| matches!(command, NetworkCommand::UpgradeSkill { .. }))
        );
    }

    #[test]
    fn protected_base_rejection_is_visible_and_does_not_approach_or_start_cooldown() {
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .init_resource::<TeamSelection>()
            .init_resource::<PendingCast>()
            .init_resource::<ActionFeedback>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<GameplayInputContext>()
            .add_systems(Update, resolve_pending_cast_system);
        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats::default(),
                PlayerProgression::default(),
                NetworkHeroClass(HeroClass::Warrior),
                NetworkPlayerId(1),
                Team::Green,
            ))
            .id();
        let base = app
            .world_mut()
            .spawn((
                Transform::from_xyz(100.0, 0.0, 0.0),
                CombatStats::default(),
                crate::net::NetworkStructureProtected(true),
                Team::Blue,
                NetworkStructureId(2),
            ))
            .id();
        app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
            slot: 0,
            target_entity: Some(base),
            target: Some(TargetId {
                kind: TargetKind::Structure,
                id: 2,
            }),
            approach_announced: false,
        });
        app.update();
        assert!(
            app.world()
                .resource::<ActionFeedback>()
                .text
                .contains("Structure protected")
        );
        assert!(app.world().resource::<PendingCast>().request.is_none());
        assert!(!app.world().entity(player).contains::<MovementTarget>());
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs,
            [0.0; 4]
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
    fn feedback_expires_in_place_and_hotbar_shows_server_rank_lock_mana_and_cooldown() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ActionFeedback>()
            .init_resource::<TeamSelection>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<TargetState>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<PendingCast>()
            .add_systems(
                Startup,
                (setup_combat_ui, crate::targeting::setup_targeting_ui),
            )
            .add_systems(Update, (update_action_feedback, update_skill_bar_system));
        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                CombatStats {
                    mana: 0.0,
                    ..default()
                },
                PlayerProgression {
                    level: 2,
                    ranks: [2, 1, 1, 1],
                    ..default()
                },
                NetworkHeroClass(HeroClass::Cleric),
            ))
            .id();
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[0] = 1.5;
        app.world_mut()
            .resource_mut::<ActionFeedback>()
            .push_line("Not enough mana.");
        app.update();
        let mut labels = app.world_mut().query::<(&SkillRankLabel, &Text)>();
        let text: Vec<_> = labels
            .iter(app.world())
            .map(|(slot, text)| (slot.slot, text.0.clone()))
            .collect();
        assert!(text.iter().any(|(slot, text)| *slot == 0
            && text.contains("Rank 2")
            && text.contains("1.5s cooldown")));
        assert!(
            text.iter()
                .any(|(slot, text)| *slot == 1 && text.contains("Need mana"))
        );
        assert!(
            text.iter()
                .any(|(slot, text)| *slot == 3 && text.contains("Locked Lv 6"))
        );
        let count = app.world().entities().len();
        for _ in 0..5 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs(1));
            app.update();
        }
        assert!(app.world().resource::<ActionFeedback>().text.is_empty());
        assert_eq!(app.world().entities().len(), count);
        assert!(app.world().entity(player).contains::<CombatStats>());
    }

    #[test]
    fn target_selection_keys_obey_modal_context_at_the_ecs_boundary() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Touches>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<TargetState>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<PendingCast>()
            .init_resource::<WorldPointerState>()
            .insert_resource(PlayerVisualMode::Models3d)
            .add_systems(Update, select_target_system);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((Player, Transform::default(), Team::Green));
        let enemy = app
            .world_mut()
            .spawn((
                RemotePlayer,
                Transform::from_xyz(2.0, 0.0, 0.0),
                Team::Blue,
                NetworkPlayerId(2),
                CombatStats::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Tab);
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = true;
        app.update();
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        app.world_mut()
            .resource_mut::<GameplayInputContext>()
            .modal_open = false;
        app.update();
        assert_eq!(
            app.world().resource::<TargetState>().selected_entity,
            Some(enemy)
        );
    }

    #[test]
    fn round_identity_clears_old_intents_cooldowns_and_queued_casts_but_reconnect_does_not() {
        let mut app = App::new();
        app.add_message::<NetworkCommand>()
            .init_resource::<CombatRoundIdentity>()
            .init_resource::<TargetState>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<PendingCast>()
            .init_resource::<LocalCastCooldown>()
            .init_resource::<ActionFeedback>()
            .insert_resource(GameStateSnapshot {
                meta: shared::protocol::SnapshotMeta::new(10, 1, 50),
                state: GameState::Running,
                ..default()
            })
            .add_systems(Update, reset_round_input_state);
        app.update();
        let actor = app
            .world_mut()
            .spawn(MovementTarget {
                target: Vec3::X * 50.0,
            })
            .id();
        app.world_mut()
            .resource_mut::<TargetState>()
            .selected_entity = Some(actor);
        app.world_mut()
            .resource_mut::<LocalCastCooldown>()
            .remaining_secs[3] = 40.0;
        app.world_mut().resource_mut::<PendingCast>().request = Some(PendingCastRequest {
            slot: 3,
            target_entity: Some(actor),
            target: None,
            approach_announced: true,
        });
        app.world_mut().resource_mut::<GameStateSnapshot>().meta = default(); // teardown gap
        app.update();
        app.world_mut().resource_mut::<GameStateSnapshot>().meta =
            shared::protocol::SnapshotMeta::new(10, 1, 55);
        app.update();
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs[3],
            40.0
        );
        assert!(app.world().entity(actor).contains::<MovementTarget>());
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .write(NetworkCommand::Cast {
                target: TargetId {
                    kind: TargetKind::Player,
                    id: 2,
                },
                slot: 3,
            });
        app.world_mut().resource_mut::<GameStateSnapshot>().meta =
            shared::protocol::SnapshotMeta::new(10, 2, 2); // zero snapshot was dropped
        app.update();
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs,
            [0.0; 4]
        );
        assert!(app.world().resource::<PendingCast>().request.is_none());
        assert!(
            app.world()
                .resource::<TargetState>()
                .selected_entity
                .is_none()
        );
        assert!(!app.world().entity(actor).contains::<MovementTarget>());
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .count(),
            0
        );
    }
    #[test]
    fn buying_haste_adjusts_active_deadlines_without_rescaling_elapsed_time() {
        use crate::net::PlayerEquipment;
        use shared::shop::{ItemId, item_bonuses, item_cooldown};
        let mut app = App::new();
        app.init_resource::<LocalCastCooldown>()
            .add_systems(Update, sync_authoritative_cooldown_durations);
        let hero = app
            .world_mut()
            .spawn((
                Player,
                PlayerProgression::default(),
                NetworkHeroClass(HeroClass::Mage),
                PlayerEquipment::default(),
            ))
            .id();
        app.update();
        let old = app.world().resource::<LocalCastCooldown>().total_secs;
        for (index, duration) in old.iter().enumerate() {
            app.world_mut()
                .resource_mut::<LocalCastCooldown>()
                .remaining_secs[index] = *duration * 0.5;
        }
        let bonuses = item_bonuses(&[ItemId::FocusCharm, ItemId::SwiftGrip]);
        app.world_mut().entity_mut(hero).insert(PlayerEquipment {
            item_bonuses: bonuses,
            ..default()
        });
        app.update();
        for slot in SkillSlot::ALL {
            let index = slot.index();
            let current = item_cooldown(
                ability_for_class_slot(HeroClass::Mage, slot),
                1,
                slot,
                bonuses,
            )
            .as_secs_f32();
            let expected = (old[index] * 0.5 + current - old[index]).max(0.0);
            assert!(
                (app.world().resource::<LocalCastCooldown>().remaining_secs[index] - expected)
                    .abs()
                    < 0.0001
            );
            assert!(
                expected < current * 0.5,
                "elapsed time is preserved, not scaled"
            );
        }
        // A second identical authoritative snapshot cannot repeatedly shorten it.
        let once = app.world().resource::<LocalCastCooldown>().remaining_secs;
        app.update();
        assert_eq!(
            app.world().resource::<LocalCastCooldown>().remaining_secs,
            once
        );
    }
}
