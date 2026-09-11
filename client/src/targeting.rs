//! Basic attacks and touch target locking, independent of the four skill slots.
use crate::{
    camera::MainCamera,
    combat::{ActionFeedback, CombatStats, PendingCast, TargetCandidates, TargetState},
    input_context::GameplayInputContext,
    mobile_controls::{MobileAttackAim, MobileControls},
    net::{
        NetworkCommand, NetworkHeroClass, NetworkMinionId, NetworkNeutralId, NetworkPlayerId,
        NetworkStructureId, NetworkStructureProtected, PlayerBasicAttackCooldown, PlayerEquipment,
        StructureKind, TargetId, TargetKind,
    },
    player::{MovementRoute, MovementTarget, Player},
    sprite::PlayerVisualMode,
    team::{Team, TeamSelection},
};
use bevy::{ecs::system::SystemParam, prelude::*, ui::FocusPolicy, window::PrimaryWindow};

#[derive(Clone, Copy, Debug)]
pub(crate) struct BasicAttackOrder {
    pub entity: Entity,
    pub target: TargetId,
    pub repeat: bool,
}
#[derive(Resource, Default)]
pub(crate) struct BasicAttackState {
    pub order: Option<BasicAttackOrder>,
    pub remaining_secs: f32,
    pub duration_secs: f32,
    acknowledged_request_id: u64,
    prediction_after_request_id: Option<u64>,
    chasing: bool,
    stop_chase: bool,
}
impl BasicAttackState {
    pub fn cancel(&mut self) {
        self.order = None;
        self.stop_chase |= self.chasing;
        self.chasing = false;
    }
    pub fn cancel_for_movement(&mut self) {
        self.order = None;
        self.chasing = false;
        self.stop_chase = false;
    }
    pub fn start(&mut self, entity: Entity, target: TargetId, repeat: bool) {
        self.order = Some(BasicAttackOrder {
            entity,
            target,
            repeat,
        });
        self.stop_chase = true;
        self.chasing = false;
    }
}
#[derive(Resource, Default, Debug)]
pub(crate) struct TargetAimPreview {
    pub active: bool,
    pub gesture: Option<u64>,
    pub origin: Vec2,
    pub cursor: Vec2,
    pub candidate: Option<Entity>,
    pub target: Option<TargetId>,
    pub candidate_screen: Option<Vec2>,
    pub in_attack_range: bool,
}

#[derive(SystemParam)]
pub(crate) struct TargetValidity<'w, 's> {
    positions: Query<'w, 's, &'static Transform>,
    structures: Query<'w, 's, &'static StructureKind>,
    units: Query<
        'w,
        's,
        (
            &'static CombatStats,
            Option<&'static Team>,
            Option<&'static InheritedVisibility>,
            Option<&'static NetworkStructureProtected>,
            Option<&'static NetworkPlayerId>,
            Option<&'static NetworkMinionId>,
            Option<&'static NetworkNeutralId>,
            Option<&'static NetworkStructureId>,
        ),
    >,
}
impl TargetValidity<'_, '_> {
    fn radius(&self, entity: Entity, id: TargetId) -> f32 {
        match id.kind {
            TargetKind::Player => shared::PLAYER_TARGET_RADIUS,
            TargetKind::Minion => shared::MINION_TARGET_RADIUS,
            TargetKind::Neutral => shared::NEUTRAL_TARGET_RADIUS,
            TargetKind::Structure
                if self
                    .structures
                    .get(entity)
                    .is_ok_and(|kind| *kind == StructureKind::BaseTower) =>
            {
                shared::BASE_TOWER_TARGET_RADIUS
            }
            TargetKind::Structure => shared::TOWER_TARGET_RADIUS,
        }
    }

    fn position(&self, entity: Entity) -> Option<Vec3> {
        self.positions.get(entity).ok().map(|t| t.translation)
    }
    pub fn valid(&self, entity: Entity, id: TargetId, team: Team) -> bool {
        let Ok((stats, other, visible, protected, player, minion, neutral, structure)) =
            self.units.get(entity)
        else {
            return false;
        };
        let identity = match id.kind {
            TargetKind::Player => player.is_some_and(|p| p.0 == id.id),
            TargetKind::Minion => minion.is_some_and(|p| p.0 == id.id),
            TargetKind::Neutral => neutral.is_some_and(|p| p.0 == id.id),
            TargetKind::Structure => structure.is_some_and(|p| p.0 == id.id),
        };
        identity
            && stats.is_alive()
            && (if id.kind == TargetKind::Neutral {
                other.is_none_or(|other| *other != team)
            } else {
                other.is_some_and(|other| *other != team)
            })
            && visible.is_none_or(|v| v.get())
            && !protected.is_some_and(|p| p.0)
    }
}

pub(crate) fn tick_basic_attack(
    time: Res<Time>,
    mut basic: ResMut<BasicAttackState>,
    authoritative: Query<
        &PlayerBasicAttackCooldown,
        (With<Player>, Changed<PlayerBasicAttackCooldown>),
    >,
) {
    basic.remaining_secs = (basic.remaining_secs - time.delta_secs()).max(0.0);
    if let Ok(server) = authoritative.single() {
        if server.last_request_id < basic.acknowledged_request_id {
            return;
        }
        basic.acknowledged_request_id = server.last_request_id;
        if basic
            .prediction_after_request_id
            .is_some_and(|baseline| server.last_request_id <= baseline)
        {
            // The server has not processed a strike newer than this prediction.
            // Earlier snapshots cannot erase its cooldown and trigger a burst.
            basic.remaining_secs = basic.remaining_secs.max(server.remaining_secs);
        } else {
            // The high-water mark advances for accepted AND rejected strikes.
            // Retire the prediction even when the actual cooldown is shorter or
            // zero; later same-ID snapshots also carry equipment speed changes.
            basic.prediction_after_request_id = None;
            basic.remaining_secs = server.remaining_secs;
            basic.duration_secs = server.duration_secs;
        }
    }
}

pub(crate) fn clear_invalid_selection(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    context: Res<GameplayInputContext>,
    window: Query<&Window, With<PrimaryWindow>>,
    local: Query<(Entity, &Team, &CombatStats), With<Player>>,
    validity: TargetValidity,
    mut target: ResMut<TargetState>,
    mut basic: ResMut<BasicAttackState>,
    mut pending: ResMut<PendingCast>,
    mut preview: ResMut<TargetAimPreview>,
) {
    let local = local.single().ok();
    let blocked = !context.gameplay_allowed()
        || window.single().is_ok_and(|w| !w.focused)
        || local.is_none_or(|(_, _, stats)| !stats.is_alive());
    let invalid = target
        .selected_entity
        .zip(target.selected_target)
        .is_some_and(|(entity, id)| {
            local.is_none_or(|(_, team, _)| !validity.valid(entity, id, *team))
        });
    if blocked
        || invalid
        || keys.just_pressed(KeyCode::Backspace)
        || keys.just_pressed(KeyCode::KeyS)
    {
        let stop_movement = blocked
            || keys.just_pressed(KeyCode::Backspace)
            || keys.just_pressed(KeyCode::KeyS)
            || basic.chasing;
        basic.cancel();
        pending.cancel();
        *preview = default();
        target.selected_entity = None;
        target.selected_target = None;
        if let Some((entity, _, _)) = local.filter(|_| stop_movement) {
            commands
                .entity(entity)
                .remove::<(MovementTarget, MovementRoute)>();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_basic_attack(
    mut commands: Commands,
    context: Res<GameplayInputContext>,
    local: Query<
        (
            Entity,
            &Transform,
            &CombatStats,
            &Team,
            Option<&NetworkHeroClass>,
            Option<&PlayerEquipment>,
        ),
        With<Player>,
    >,
    positions: Query<(&Transform, Option<&StructureKind>), Without<Player>>,
    validity: TargetValidity,
    selection: Res<TeamSelection>,
    mobile: Option<Res<MobileControls>>,
    pending: Res<PendingCast>,
    mut basic: ResMut<BasicAttackState>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut feedback: ResMut<ActionFeedback>,
) {
    let Ok((player, transform, stats, team, class, equipment)) = local.single() else {
        basic.cancel();
        return;
    };
    if basic.stop_chase {
        commands
            .entity(player)
            .remove::<(MovementTarget, MovementRoute)>();
        basic.stop_chase = false;
    }
    if !context.gameplay_allowed() || !stats.is_alive() {
        basic.cancel();
        return;
    }
    let Some(order) = basic.order else {
        return;
    };
    if !validity.valid(order.entity, order.target, *team) {
        basic.cancel();
        return;
    }
    // A newly requested skill takes priority over the repeated basic attack.
    if pending.is_pending() {
        return;
    }
    let Ok((position, structure_kind)) = positions.get(order.entity) else {
        basic.cancel();
        return;
    };
    let class = class.map_or(selection.hero_class, |c| c.0);
    let definition = shared::basic_attack_for_class(class);
    let bonuses = equipment.map_or_else(Default::default, |e| e.item_bonuses);
    let radius = match order.target.kind {
        TargetKind::Player => shared::PLAYER_TARGET_RADIUS,
        TargetKind::Minion => shared::MINION_TARGET_RADIUS,
        TargetKind::Neutral => shared::NEUTRAL_TARGET_RADIUS,
        TargetKind::Structure if structure_kind == Some(&StructureKind::BaseTower) => {
            shared::BASE_TOWER_TARGET_RADIUS
        }
        TargetKind::Structure => shared::TOWER_TARGET_RADIUS,
    };
    let range = definition.range + radius - 0.08;
    let distance = transform
        .translation
        .xz()
        .distance(position.translation.xz());
    let phone = mobile.as_ref().is_some_and(|m| m.enabled);
    if distance > range {
        if phone {
            feedback.push_line("Target out of attack range — move closer.");
            basic.cancel();
        } else {
            let direction = (position.translation - transform.translation)
                .with_y(0.0)
                .normalize_or_zero();
            let destination = position.translation - direction * (range * 0.90);
            commands.entity(player).insert(MovementTarget {
                target: destination,
            });
            if !basic.chasing {
                feedback.push_line("Approaching selected target.");
            }
            basic.chasing = true;
        }
        return;
    }
    if basic.chasing {
        commands
            .entity(player)
            .remove::<(MovementTarget, MovementRoute)>();
        basic.chasing = false;
    }
    if basic.remaining_secs > 0.0 {
        return;
    }
    outgoing.write(NetworkCommand::BasicAttack {
        target: order.target,
    });
    basic.duration_secs = shared::shop::basic_attack_cooldown(definition, bonuses).as_secs_f32();
    basic.remaining_secs = basic.duration_secs;
    // Snapshot acknowledgment is a baseline, not a packet resend token. If a
    // datagram is lost the next local deadline may request a fresh strike; the
    // network sender always assigns its own monotonically increasing wire ID.
    basic.prediction_after_request_id = Some(basic.acknowledged_request_id);
    if !order.repeat {
        basic.order = None;
    }
}

fn screen_position(
    camera: &Camera,
    transform: &GlobalTransform,
    mode: PlayerVisualMode,
    p: Vec3,
) -> Option<Vec2> {
    let render = if mode == PlayerVisualMode::Sprite2d {
        crate::world2d::simulation_xz_to_render_xy(p).extend(crate::world2d::layer::ACTOR)
    } else {
        p
    };
    let screen = camera.world_to_viewport(transform, render).ok()?;
    let size = camera.logical_viewport_size()?;
    (screen.is_finite() && screen.cmpge(Vec2::ZERO).all() && screen.cmple(size).all())
        .then_some(screen)
}
pub(crate) fn aim_cursor(origin: Vec2, viewport: Vec2, aim: MobileAttackAim) -> Vec2 {
    let reach = (viewport.y * 0.70).min(viewport.x * 0.45);
    origin + aim.direction.normalize_or_zero() * aim.extent.clamp(0.0, 1.0) * reach
}
/// The reticle chooses proximity in two dimensions, so thumb distance can select
/// the second enemy along the same bearing instead of always snapping to the first.
pub(crate) fn reticle_score(
    origin: Vec2,
    cursor: Vec2,
    point: Vec2,
    pick_radius: f32,
) -> Option<f32> {
    let vector = cursor - origin;
    if vector.length_squared() < 4.0 || !vector.is_finite() || !point.is_finite() {
        return None;
    }
    let delta = point - origin;
    if delta.dot(vector) <= 0.0 {
        return None;
    }
    let distance = cursor.distance(point);
    (distance <= pick_radius).then_some(distance)
}

#[allow(clippy::too_many_arguments)]
fn pick_mobile(
    origin_position: Vec3,
    team: Team,
    range: f32,
    aim: Option<MobileAttackAim>,
    candidates: &TargetCandidates,
    validity: &TargetValidity,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    mode: PlayerVisualMode,
) -> Option<(Entity, TargetId, Vec2)> {
    let origin = screen_position(camera, camera_transform, mode, origin_position)?;
    let viewport = camera.logical_viewport_size()?;
    let cursor = aim.map(|a| aim_cursor(origin, viewport, a));
    let mut best: Option<(Entity, TargetId, Vec2, f32)> = None;
    let mut consider = |entity: Entity, id: TargetId, position: Vec3| {
        if !validity.valid(entity, id, team) {
            return;
        }
        let distance = position.xz().distance(origin_position.xz());
        if distance > range + validity.radius(entity, id) - 0.08 {
            return;
        }
        let Some(screen) = screen_position(camera, camera_transform, mode, position) else {
            return;
        };
        let score = if let Some(cursor) = cursor {
            let Some(score) = reticle_score(origin, cursor, screen, 36.0) else {
                return;
            };
            score
        } else {
            // A deliberate existing lock is resolved by the caller. Automatic
            // acquisition prefers nearby champions, then farm/objective targets.
            distance
                + if id.kind == TargetKind::Player {
                    0.0
                } else {
                    range
                }
        };
        if best.is_none_or(|(_, prev, _, old)| score < old || (score == old && id.id < prev.id)) {
            best = Some((entity, id, screen, score));
        }
    };
    for (e, t, id, _, _) in &candidates.players {
        consider(
            e,
            TargetId {
                kind: TargetKind::Player,
                id: id.0,
            },
            t.translation,
        );
    }
    for (e, t, id, _, _) in &candidates.minions {
        consider(
            e,
            TargetId {
                kind: TargetKind::Minion,
                id: id.0,
            },
            t.translation,
        );
    }
    for (e, t, id, _) in &candidates.neutrals {
        consider(
            e,
            TargetId {
                kind: TargetKind::Neutral,
                id: id.0,
            },
            t.translation,
        );
    }
    for (e, t, id, _, _, _) in &candidates.structures {
        consider(
            e,
            TargetId {
                kind: TargetKind::Structure,
                id: id.0,
            },
            t.translation,
        );
    }
    best.map(|(e, id, p, _)| (e, id, p))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn mobile_basic_attack(
    mut mobile: Option<ResMut<MobileControls>>,
    context: Res<GameplayInputContext>,
    local: Query<(&Transform, &Team, &CombatStats, Option<&NetworkHeroClass>), With<Player>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
    selection: Res<TeamSelection>,
    candidates: TargetCandidates,
    validity: TargetValidity,
    mut target: ResMut<TargetState>,
    mut preview: ResMut<TargetAimPreview>,
    mut basic: ResMut<BasicAttackState>,
    pending: Res<PendingCast>,
    mut feedback: ResMut<ActionFeedback>,
) {
    let previous_preview = std::mem::take(&mut *preview);
    let Some(mobile) = mobile.as_deref_mut().filter(|m| m.enabled) else {
        return;
    };
    let Ok((position, team, stats, class)) = local.single() else {
        mobile.attacks.clear();
        return;
    };
    if !context.gameplay_allowed() || !stats.is_alive() {
        mobile.attacks.clear();
        basic.cancel();
        return;
    }
    let Ok((camera, camera_transform)) = camera.single() else {
        mobile.attacks.clear();
        return;
    };
    let class = class.map_or(selection.hero_class, |c| c.0);
    let range = shared::basic_attack_for_class(class).range;
    let select_range = range.max(24.0);
    if let Some(aim) = mobile.attack_aim() {
        basic.cancel();
        if let (Some(origin), Some(viewport)) = (
            screen_position(camera, camera_transform, *mode, position.translation),
            camera.logical_viewport_size(),
        ) {
            let pick = pick_mobile(
                position.translation,
                *team,
                select_range,
                Some(aim),
                &candidates,
                &validity,
                camera,
                camera_transform,
                *mode,
            );
            *preview = TargetAimPreview {
                active: true,
                gesture: mobile.attack_gesture(),
                origin,
                cursor: aim_cursor(origin, viewport, aim),
                candidate: pick.map(|p| p.0),
                target: pick.map(|p| p.1),
                candidate_screen: pick.map(|p| p.2),
                in_attack_range: pick.is_some_and(|p| {
                    validity.position(p.0).is_some_and(|v| {
                        position.translation.xz().distance(v.xz())
                            <= range + validity.radius(p.0, p.1) - 0.08
                    })
                }),
            };
        }
    }
    if mobile.attack_cancelled() {
        basic.cancel();
        mobile.attacks.clear();
        *preview = default();
        return;
    }
    if mobile.skill_aiming() {
        basic.cancel();
        mobile.attacks.clear();
        *preview = default();
        return;
    }
    if mobile.attack_pressed() && !mobile.held_basic_attack() {
        basic.cancel();
    }
    // Explicit skill releases are consumed earlier and remain pending until
    // their resolver; do not silently turn a simultaneous release into an attack.
    if pending.is_pending() {
        mobile.attacks.clear();
        return;
    }
    let intent = mobile.attacks.drain(..).next_back();
    let held = mobile.held_basic_attack();
    if intent.is_none() && !held {
        return;
    }
    if basic.remaining_secs > 0.0 && intent.is_none() {
        return;
    }
    let aim = intent.and_then(|i| i.aim);
    let pick = if let Some(aim) = aim {
        // Commit the exact preview shown for this press. Revalidate its final
        // reticle/visibility, but never replace a dead/moved candidate with B.
        let gesture = intent.map(|i| i.gesture);
        previous_preview
            .candidate
            .zip(previous_preview.target)
            .filter(|(entity, id)| {
                if !previous_preview.active
                    || previous_preview.gesture != gesture
                    || !validity.valid(*entity, *id, *team)
                {
                    return false;
                }
                let Some(p) = validity.position(*entity) else {
                    return false;
                };
                let (Some(origin), Some(point), Some(viewport)) = (
                    screen_position(camera, camera_transform, *mode, position.translation),
                    screen_position(camera, camera_transform, *mode, p),
                    camera.logical_viewport_size(),
                ) else {
                    return false;
                };
                position.translation.xz().distance(p.xz())
                    <= select_range + validity.radius(*entity, *id) - 0.08
                    && reticle_score(origin, aim_cursor(origin, viewport, aim), point, 36.0)
                        .is_some()
            })
    } else if let Some((e, id)) = target.selected_entity.zip(target.selected_target) {
        validity.valid(e, id, *team).then_some((e, id))
    } else {
        pick_mobile(
            position.translation,
            *team,
            range,
            None,
            &candidates,
            &validity,
            camera,
            camera_transform,
            *mode,
        )
        .map(|(e, id, _)| (e, id))
    };
    if let Some((entity, id)) = pick {
        target.selected_entity = Some(entity);
        target.selected_target = Some(id);
        basic.start(entity, id, false);
    } else {
        basic.cancel();
        if intent.is_some() {
            feedback.push_line(if aim.is_some() {
                "No target under the reticle."
            } else {
                "No enemy in range."
            });
        }
    }
}

#[derive(Component)]
pub(crate) struct LockedTargetIndicator;
#[derive(Component)]
pub(crate) struct LockedTargetLabel;

pub(crate) fn draw_locked_target(
    target: Res<TargetState>,
    basic: Res<BasicAttackState>,
    validity: TargetValidity,
    local: Query<&Team, With<Player>>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mode: Res<PlayerVisualMode>,
    settings: Option<Res<crate::model_scale::ModelScaleSettings>>,
    mut indicator: Query<&mut Node, With<LockedTargetIndicator>>,
    mut label: Query<&mut Text, With<LockedTargetLabel>>,
) {
    let Ok(mut node) = indicator.single_mut() else {
        return;
    };
    node.display = Display::None;
    let Some((entity, id)) = target.selected_entity.zip(target.selected_target) else {
        return;
    };
    let (Ok(team), Ok((camera, transform)), Some(position)) =
        (local.single(), camera.single(), validity.position(entity))
    else {
        return;
    };
    if !validity.valid(entity, id, *team) {
        return;
    }
    let Some(foot) = screen_position(camera, transform, *mode, position) else {
        return;
    };
    let model_height = match id.kind {
        TargetKind::Player => settings.as_ref().map_or(2.1, |s| s.target_height),
        TargetKind::Structure => 3.6,
        _ => 1.1,
    };
    let height = if *mode == PlayerVisualMode::Models3d {
        camera
            .world_to_viewport(transform, position + Vec3::Y * model_height)
            .ok()
            .map_or(40.0, |head| (head.y - foot.y).abs())
    } else {
        40.0
    };
    let width = match id.kind {
        TargetKind::Player => 46.0,
        TargetKind::Structure => 72.0,
        _ => 34.0,
    };
    *node = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(foot.x - width / 2.0),
        top: Val::Px(foot.y - height.clamp(24.0, 100.0) - 10.0),
        width: Val::Px(width),
        height: Val::Px(height.clamp(24.0, 100.0) + 18.0),
        border: UiRect::all(Val::Px(3.0)),
        border_radius: BorderRadius::all(Val::Px(7.0)),
        ..default()
    };
    if let Ok(mut label) = label.single_mut() {
        label.0 = if basic.order.is_some_and(|order| order.target == id) {
            "ATTACK"
        } else {
            "LOCKED"
        }
        .into();
    }
}

#[derive(Component)]
pub(crate) enum AimVisual {
    Vector,
    Reticle,
    Candidate,
    Label,
}
pub(crate) fn setup_targeting_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            BorderColor::all(crate::ui_theme::GOLD),
            BackgroundColor(Color::srgba(1.0, 0.8, 0.1, 0.06)),
            ZIndex(32),
            FocusPolicy::Pass,
            LockedTargetIndicator,
            Name::new("LockedTargetIndicator"),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("LOCKED"),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                TextColor(Color::srgb(0.06, 0.05, 0.01)),
                BackgroundColor(crate::ui_theme::GOLD),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(-3.0),
                    bottom: Val::Px(-17.0),
                    padding: UiRect::horizontal(Val::Px(3.0)),
                    ..default()
                },
                FocusPolicy::Pass,
                LockedTargetLabel,
            ));
        });

    for (part, name) in [
        (AimVisual::Vector, "TargetAimVector"),
        (AimVisual::Reticle, "TargetAimReticle"),
        (AimVisual::Candidate, "TargetAimCandidate"),
        (AimVisual::Label, "TargetAimLabel"),
    ] {
        let label = matches!(part, AimVisual::Label);
        let mut entity = commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            UiTransform::default(),
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::srgb(1.0, 0.78, 0.25)),
            ZIndex(35),
            FocusPolicy::Pass,
            part,
            Name::new(name),
        ));
        if label {
            entity.insert((
                Text::new(""),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(crate::ui_theme::IVORY),
            ));
        }
    }
}
pub(crate) fn draw_targeting_ui(
    preview: Res<TargetAimPreview>,
    mut nodes: Query<(
        &AimVisual,
        &mut Node,
        &mut UiTransform,
        &mut BackgroundColor,
        Option<&mut Text>,
        &mut BorderColor,
    )>,
) {
    for (part, mut node, mut transform, mut color, label, mut border) in &mut nodes {
        node.display = Display::None;
        if !preview.active {
            continue;
        }
        let ready = preview.candidate.is_some() && preview.in_attack_range;
        let tint = if ready {
            Color::srgb(1.0, 0.78, 0.25)
        } else {
            Color::srgba(0.6, 0.85, 0.95, 0.8)
        };
        *border = BorderColor::all(tint);
        match part {
            AimVisual::Label => {
                *node = Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((preview.cursor.x - 65.0).max(8.0)),
                    top: Val::Px((preview.cursor.y - 42.0).max(8.0)),
                    width: Val::Px(160.0),
                    ..default()
                };
                if let Some(mut text) = label {
                    text.0 = if ready {
                        "RELEASE TO ATTACK"
                    } else if preview.candidate.is_some() {
                        "LOCK · MOVE CLOSER"
                    } else {
                        "AIM AT A TARGET"
                    }
                    .into();
                }
                *color = BackgroundColor(Color::srgba(0.01, 0.04, 0.05, 0.78));
            }
            AimVisual::Vector => {
                let (line, rotation) =
                    crate::minimap::line_node(preview.origin, preview.cursor, 3.0);
                *node = line;
                *transform = rotation;
                *color = BackgroundColor(tint);
            }
            AimVisual::Reticle | AimVisual::Candidate => {
                let point = if matches!(part, AimVisual::Candidate) {
                    let Some(p) = preview.candidate_screen else {
                        continue;
                    };
                    p
                } else {
                    preview.cursor
                };
                let size = if matches!(part, AimVisual::Candidate) {
                    48.0
                } else {
                    22.0
                };
                *node = Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(point.x - size / 2.0),
                    top: Val::Px(point.y - size / 2.0),
                    width: Val::Px(size),
                    height: Val::Px(size),
                    border: UiRect::all(Val::Px(if ready { 3.0 } else { 2.0 })),
                    border_radius: BorderRadius::all(Val::Percent(50.0)),
                    ..default()
                };
                *transform = UiTransform::default();
                *color = BackgroundColor(Color::NONE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attack_app(
        class: shared::HeroClass,
        inventory: Vec<shared::shop::ItemId>,
        phone: bool,
    ) -> (App, Entity, Entity) {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = phone;
        app.init_resource::<Time>()
            .init_resource::<BasicAttackState>()
            .init_resource::<TargetAimPreview>()
            .init_resource::<TargetState>()
            .init_resource::<PendingCast>()
            .init_resource::<ActionFeedback>()
            .init_resource::<TeamSelection>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<NetworkCommand>()
            .insert_resource(mobile)
            .add_systems(Update, (tick_basic_attack, resolve_basic_attack).chain());
        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::default(),
                Team::Green,
                CombatStats {
                    mana: 0.0,
                    ..default()
                },
                NetworkHeroClass(class),
                NetworkPlayerId(1),
                PlayerEquipment {
                    item_bonuses: shared::shop::item_bonuses(&inventory),
                    inventory,
                    ..default()
                },
            ))
            .id();
        let enemy = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 0.0, 0.0),
                Team::Blue,
                CombatStats::default(),
                NetworkPlayerId(2),
            ))
            .id();
        (app, player, enemy)
    }
    fn order(app: &mut App, enemy: Entity) {
        app.world_mut().resource_mut::<BasicAttackState>().start(
            enemy,
            TargetId {
                kind: TargetKind::Player,
                id: 2,
            },
            true,
        );
    }
    fn commands(app: &mut App) -> Vec<NetworkCommand> {
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect()
    }

    fn replicate_cooldown(app: &mut App, player: Entity, id: u64, duration: f32, remaining: f32) {
        app.world_mut()
            .entity_mut(player)
            .insert(PlayerBasicAttackCooldown {
                last_request_id: id,
                duration_secs: duration,
                remaining_secs: remaining,
            });
    }

    #[test]
    fn snapshots_before_prediction_do_not_erase_it_but_acknowledgment_replaces_it() {
        let (mut app, player, enemy) = attack_app(shared::HeroClass::Warrior, vec![], false);
        replicate_cooldown(&mut app, player, 7, 0.9, 0.0);
        order(&mut app, enemy);
        app.update();
        assert_eq!(commands(&mut app).len(), 1);
        assert_eq!(
            app.world()
                .resource::<BasicAttackState>()
                .prediction_after_request_id,
            Some(7)
        );

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(100));
        replicate_cooldown(&mut app, player, 7, 0.9, 0.0);
        app.update();
        assert!(commands(&mut app).is_empty());
        assert!((app.world().resource::<BasicAttackState>().remaining_secs - 0.8).abs() < 0.0001);
        replicate_cooldown(&mut app, player, 8, 0.9, 0.65);
        app.update();
        let basic = app.world().resource::<BasicAttackState>();
        assert_eq!(basic.remaining_secs, 0.65);
        assert_eq!(basic.prediction_after_request_id, None);
        assert!(commands(&mut app).is_empty());
    }

    #[test]
    fn rejected_strike_acknowledgment_removes_phantom_cooldown_without_restarting_cancelled_order()
    {
        for actual_remaining in [0.0, 0.03] {
            let (mut app, player, enemy) = attack_app(shared::HeroClass::Mage, vec![], false);
            replicate_cooldown(&mut app, player, 11, 1.1, 0.0);
            order(&mut app, enemy);
            app.update();
            assert_eq!(commands(&mut app).len(), 1);
            app.world_mut()
                .resource_mut::<BasicAttackState>()
                .cancel_for_movement();
            replicate_cooldown(&mut app, player, 12, 1.1, actual_remaining);
            app.update();
            let basic = app.world().resource::<BasicAttackState>();
            assert_eq!(basic.remaining_secs, actual_remaining);
            assert_eq!(basic.prediction_after_request_id, None);
            assert!(basic.order.is_none());
            assert!(commands(&mut app).is_empty());
        }
    }

    #[test]
    fn acknowledged_equipment_speed_changes_follow_actual_deadline_and_old_ids_cannot_rewind() {
        let (mut app, player, enemy) = attack_app(shared::HeroClass::Warrior, vec![], false);
        replicate_cooldown(&mut app, player, 4, 0.9, 0.0);
        order(&mut app, enemy);
        app.update();
        assert_eq!(commands(&mut app).len(), 1);
        replicate_cooldown(&mut app, player, 5, 0.9, 0.6);
        app.update();
        let faster = shared::shop::basic_attack_cooldown(
            shared::basic_attack_for_class(shared::HeroClass::Warrior),
            shared::shop::item_bonuses(&[shared::shop::ItemId::SwiftGrip]),
        )
        .as_secs_f32();
        replicate_cooldown(&mut app, player, 5, faster, faster - 0.4);
        app.update();
        let basic = app.world().resource::<BasicAttackState>();
        assert_eq!(basic.duration_secs, faster);
        assert_eq!(basic.remaining_secs, faster - 0.4);
        // An old acknowledgment cannot revive its longer timer or erase this one.
        replicate_cooldown(&mut app, player, 4, 0.9, 0.0);
        app.update();
        assert_eq!(
            app.world().resource::<BasicAttackState>().remaining_secs,
            faster - 0.4
        );
        assert!(commands(&mut app).is_empty());
    }

    #[test]
    fn repeat_uses_independent_class_equipment_deadlines_without_mana_or_skill() {
        for (class, inventory) in [
            (shared::HeroClass::Warrior, vec![]),
            (shared::HeroClass::Mage, vec![]),
            (
                shared::HeroClass::Ranger,
                vec![shared::shop::ItemId::SwiftGrip],
            ),
        ] {
            let deadline = shared::shop::basic_attack_cooldown(
                shared::basic_attack_for_class(class),
                shared::shop::item_bonuses(&inventory),
            )
            .as_secs_f32();
            let (mut app, player, enemy) = attack_app(class, inventory, false);
            order(&mut app, enemy);
            let mut fired = Vec::new();
            for frame in 0..=400 {
                if frame > 0 {
                    app.world_mut()
                        .resource_mut::<Time>()
                        .advance_by(std::time::Duration::from_millis(10));
                }
                app.update();
                for command in commands(&mut app) {
                    assert!(matches!(
                        command,
                        NetworkCommand::BasicAttack {
                            target: TargetId {
                                kind: TargetKind::Player,
                                id: 2
                            }
                        }
                    ));
                    fired.push(frame as f32 * 0.01);
                }
            }
            assert!(fired.len() >= 4, "{class:?}: {fired:?}");
            for interval in fired.windows(2).map(|p| p[1] - p[0]) {
                assert!(
                    interval >= deadline - 0.0001 && interval <= deadline + 0.011,
                    "{interval}/{deadline}"
                );
            }
            assert_eq!(app.world().get::<CombatStats>(player).unwrap().mana, 0.0);
        }
    }
    #[test]
    fn held_phone_button_repeats_basic_without_touching_q_cooldown() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let (mut app, _, enemy) = attack_app(shared::HeroClass::Mage, vec![], true);
        app.world_mut()
            .entity_mut(enemy)
            .insert(crate::net::RemotePlayer);
        app.world_mut()
            .get_mut::<Transform>(enemy)
            .unwrap()
            .translation = Vec3::X * 0.1;
        app.world_mut()
            .resource_mut::<MobileControls>()
            .start_attack_hold_for_test();
        app.insert_resource(PlayerVisualMode::Models3d)
            .init_resource::<crate::combat::LocalCastCooldown>()
            .add_systems(
                Update,
                mobile_basic_attack
                    .after(tick_basic_attack)
                    .before(resolve_basic_attack),
            );
        app.world_mut().spawn((
            MainCamera,
            GlobalTransform::IDENTITY,
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: Mat4::IDENTITY,
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(844, 390),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        let mut fired = 0;
        for _ in 0..260 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_millis(10));
            app.update();
            for command in commands(&mut app) {
                assert!(matches!(command, NetworkCommand::BasicAttack { .. }));
                fired += 1;
            }
        }
        assert_eq!(fired, 3);
        assert_eq!(
            app.world()
                .resource::<crate::combat::LocalCastCooldown>()
                .remaining_secs,
            [0.0; 4]
        );
    }
    #[test]
    fn drag_release_never_substitutes_a_new_enemy_for_the_preview() {
        use crate::mobile_controls::MobileAttackIntent;
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        for reject in 0..3 {
            let (mut app, _, enemy) = attack_app(shared::HeroClass::Mage, vec![], true);
            app.world_mut()
                .entity_mut(enemy)
                .insert(crate::net::RemotePlayer);
            app.world_mut()
                .get_mut::<Transform>(enemy)
                .unwrap()
                .translation = Vec3::X * 0.1;
            app.insert_resource(PlayerVisualMode::Models3d)
                .add_systems(Update, mobile_basic_attack.before(resolve_basic_attack));
            app.world_mut().spawn((
                MainCamera,
                GlobalTransform::IDENTITY,
                Camera {
                    computed: ComputedCameraValues {
                        clip_from_view: Mat4::IDENTITY,
                        target_info: Some(RenderTargetInfo {
                            physical_size: UVec2::new(844, 390),
                            scale_factor: 1.0,
                        }),
                        ..default()
                    },
                    ..default()
                },
            ));
            let selected = TargetId {
                kind: TargetKind::Player,
                id: 2,
            };
            let origin = Vec2::new(422.0, 195.0);
            let point = origin + Vec2::X * 42.2;
            *app.world_mut().resource_mut::<TargetAimPreview>() = TargetAimPreview {
                active: true,
                gesture: Some(7),
                origin,
                cursor: point,
                candidate: Some(enemy),
                target: Some(selected),
                candidate_screen: Some(point),
                in_attack_range: true,
            };
            if reject == 1 {
                app.world_mut().get_mut::<CombatStats>(enemy).unwrap().hp = 0.0;
            }
            app.world_mut().spawn((
                crate::net::RemotePlayer,
                Transform::from_xyz(0.1, 0.0, 0.0),
                Team::Blue,
                CombatStats::default(),
                NetworkPlayerId(3),
            ));
            app.world_mut()
                .resource_mut::<MobileControls>()
                .attacks
                .push(MobileAttackIntent {
                    gesture: if reject == 2 { 8 } else { 7 },
                    aim: Some(MobileAttackAim {
                        direction: Vec2::X,
                        extent: 42.2 / 273.0,
                    }),
                });
            app.update();
            let sent = commands(&mut app);
            if reject == 0 {
                assert!(
                    matches!(sent.as_slice(),[NetworkCommand::BasicAttack{target}] if *target==selected)
                );
            } else {
                assert!(sent.is_empty());
                assert!(
                    app.world()
                        .resource::<TargetState>()
                        .selected_target
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn desktop_chase_tracks_a_moving_target_without_attacking_early() {
        let (mut app, player, enemy) = attack_app(shared::HeroClass::Warrior, vec![], false);
        app.world_mut()
            .get_mut::<Transform>(enemy)
            .unwrap()
            .translation = Vec3::X * 16.0;
        order(&mut app, enemy);
        app.update();
        let first = app.world().get::<MovementTarget>(player).unwrap().target;
        assert!(first.x > 10.0);
        assert!(commands(&mut app).is_empty());
        app.world_mut()
            .get_mut::<Transform>(enemy)
            .unwrap()
            .translation = Vec3::Z * 16.0;
        app.update();
        let next = app.world().get::<MovementTarget>(player).unwrap().target;
        assert!(next.z > 10.0 && next.x.abs() < 0.01);
        assert!(commands(&mut app).is_empty());
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = Vec3::Z * 13.0;
        app.update();
        assert_eq!(commands(&mut app).len(), 1);
        assert!(app.world().get::<MovementTarget>(player).is_none());
    }
    #[test]
    fn chase_is_desktop_only_and_invalid_targets_never_emit() {
        for phone in [false, true] {
            let (mut app, player, enemy) = attack_app(shared::HeroClass::Warrior, vec![], phone);
            app.world_mut()
                .get_mut::<Transform>(enemy)
                .unwrap()
                .translation = Vec3::X * 12.0;
            order(&mut app, enemy);
            app.update();
            assert!(commands(&mut app).is_empty());
            assert_eq!(app.world().get::<MovementTarget>(player).is_some(), !phone);
            if !phone {
                app.world_mut()
                    .get_mut::<Transform>(enemy)
                    .unwrap()
                    .translation = Vec3::X;
                app.update();
                assert_eq!(commands(&mut app).len(), 1);
                assert!(app.world().get::<MovementTarget>(player).is_none());
            }
        }
        for invalid in 0..5 {
            let (mut app, _, enemy) = attack_app(shared::HeroClass::Warrior, vec![], false);
            order(&mut app, enemy);
            match invalid {
                0 => {
                    app.world_mut().get_mut::<CombatStats>(enemy).unwrap().hp = 0.0;
                }
                1 => {
                    *app.world_mut().get_mut::<Team>(enemy).unwrap() = Team::Green;
                }
                2 => {
                    app.world_mut()
                        .entity_mut(enemy)
                        .insert(InheritedVisibility::HIDDEN);
                }
                3 => {
                    app.world_mut()
                        .entity_mut(enemy)
                        .insert(NetworkStructureProtected(true));
                }
                _ => {
                    app.world_mut().despawn(enemy);
                }
            }
            app.update();
            assert!(commands(&mut app).is_empty());
            assert!(app.world().resource::<BasicAttackState>().order.is_none());
        }
    }
    #[test]
    fn attack_order_replaces_an_old_ground_route_even_when_already_in_range() {
        let (mut app, player, enemy) = attack_app(shared::HeroClass::Mage, vec![], false);
        app.world_mut().entity_mut(player).insert((
            MovementTarget {
                target: Vec3::Z * 15.0,
            },
            MovementRoute {
                requested_target: Vec3::Z * 15.0,
                destination: Vec3::Z * 15.0,
                waypoints: vec![Vec3::Z * 15.0],
                structure_revision: 0,
            },
        ));
        order(&mut app, enemy);
        app.update();
        assert_eq!(commands(&mut app).len(), 1);
        assert!(app.world().get::<MovementTarget>(player).is_none());
        assert!(app.world().get::<MovementRoute>(player).is_none());
    }
    #[test]
    fn movement_cancel_preserves_new_route_and_modal_or_focus_clears_attack() {
        let (mut app, player, enemy) = attack_app(shared::HeroClass::Warrior, vec![], false);
        app.world_mut()
            .get_mut::<Transform>(enemy)
            .unwrap()
            .translation = Vec3::X * 12.0;
        order(&mut app, enemy);
        app.update();
        assert!(app.world().get::<MovementTarget>(player).is_some());
        app.world_mut()
            .resource_mut::<BasicAttackState>()
            .cancel_for_movement();
        app.world_mut().entity_mut(player).insert(MovementTarget {
            target: Vec3::Z * 4.0,
        });
        app.update();
        assert_eq!(
            app.world().get::<MovementTarget>(player).unwrap().target,
            Vec3::Z * 4.0
        );
        assert!(commands(&mut app).is_empty());
        app.add_systems(Update, clear_invalid_selection.before(resolve_basic_attack));
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        for focus in [false, true] {
            order(&mut app, enemy);
            app.world_mut().get_mut::<Window>(window).unwrap().focused = focus;
            app.world_mut()
                .resource_mut::<GameplayInputContext>()
                .modal_open = focus;
            app.update();
            assert!(app.world().resource::<BasicAttackState>().order.is_none());
            assert!(app.world().get::<MovementTarget>(player).is_none());
        }
    }
    #[test]
    fn reticle_growth_selects_distance_and_rejects_behind_or_empty_direction() {
        let origin = Vec2::new(400.0, 200.0);
        let viewport = Vec2::new(844.0, 390.0);
        let near = aim_cursor(
            origin,
            viewport,
            MobileAttackAim {
                direction: Vec2::X,
                extent: 0.25,
            },
        );
        let far = aim_cursor(
            origin,
            viewport,
            MobileAttackAim {
                direction: Vec2::X,
                extent: 0.75,
            },
        );
        assert!(far.distance(origin) > near.distance(origin) * 2.9);
        assert!(reticle_score(origin, near, near, 36.0).is_some());
        assert!(reticle_score(origin, near, far, 36.0).is_none());
        assert!(reticle_score(origin, far, far, 36.0).is_some());
        assert!(reticle_score(origin, far, origin - Vec2::X * 20.0, 36.0).is_none());
        assert!(reticle_score(origin, origin, near, 36.0).is_none());
        assert_eq!(
            aim_cursor(
                origin,
                viewport,
                MobileAttackAim {
                    direction: Vec2::X,
                    extent: 20.0
                }
            ),
            aim_cursor(
                origin,
                viewport,
                MobileAttackAim {
                    direction: Vec2::X,
                    extent: 1.0
                }
            )
        );
    }
}
