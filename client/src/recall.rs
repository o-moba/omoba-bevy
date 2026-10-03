//! Recall input and presentation: authority owns channel timing and teleportation.
use bevy::prelude::*;
use shared::utility::UtilityAction;

use crate::{
    combat::PendingCast,
    domain::{CombatStats, MovementRoute, MovementTarget, Player},
    input_context::{CombatPointerInputSet, GameplayInputContext, InputContextSet},
    mobile_controls::{MobileControls, MobileControlsSet},
    net::{ClientNetPipeline, NetworkCommand, PlayerUtility},
    targeting::BasicAttackState,
};

pub(crate) struct RecallPlugin;
impl Plugin for RecallPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "qa")]
        app.init_resource::<RecallVisualProof>();
        app.insert_gizmo_config(
            RecallGizmos,
            bevy::gizmos::config::GizmoConfig {
                line: bevy::gizmos::config::GizmoLineConfig {
                    width: 3.5,
                    ..default()
                },
                depth_bias: 0.0,
                ..default()
            },
        )
        .init_resource::<RecallInput>()
        .add_systems(
            Update,
            (
                prepare_recall_input
                    .after(MobileControlsSet::Input)
                    .after(InputContextSet::Resolve)
                    .before(CombatPointerInputSet),
                cancel_for_new_movement
                    .after(InputContextSet::Actions)
                    .before(ClientNetPipeline::SendLocalState),
            ),
        )
        .add_systems(
            PostUpdate,
            draw_recall_channel
                .after(crate::net::NetworkGroundingSet)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[derive(Default, Reflect, bevy::gizmos::config::GizmoConfigGroup)]
struct RecallGizmos;

/// Opt-in QA records actual draw submissions, not merely a replicated timer.
#[cfg(feature = "qa")]
#[derive(Resource, Default)]
pub(crate) struct RecallVisualProof {
    pub drawn_heroes: usize,
    pub max_remaining_secs: f32,
}

/// No persistent effect entity: cancellation/finish/fog removal stops drawing
/// on the same frame that the authoritative channel disappears.
fn recall_progress(remaining: f32, alive: bool, visible: bool) -> Option<f32> {
    (alive && visible && remaining.is_finite() && remaining > 0.0)
        .then(|| 1.0 - (remaining / shared::utility::RECALL_CHANNEL_SECS).clamp(0.0, 1.0))
}

fn draw_recall_channel(
    mut gizmos: Gizmos<RecallGizmos>,
    time: Res<Time<Real>>,
    context: Res<GameplayInputContext>,
    mode: Res<crate::sprite::PlayerVisualMode>,
    map: Res<crate::maps::MapLayout>,
    #[cfg(feature = "qa")] mut proof: ResMut<RecallVisualProof>,
    heroes: Query<(
        &Transform,
        &CombatStats,
        &PlayerUtility,
        Option<&InheritedVisibility>,
    )>,
) {
    #[cfg(feature = "qa")]
    {
        proof.drawn_heroes = 0;
        proof.max_remaining_secs = 0.0;
    }
    if !context.running {
        return;
    }
    let phase = time.elapsed_secs() * 1.8;
    let cyan = Color::srgba(0.25, 1.0, 0.93, 0.96);
    let gold = Color::srgba(1.0, 0.79, 0.3, 0.98);
    let is_2d = *mode == crate::sprite::PlayerVisualMode::Sprite2d;
    for (transform, stats, utility, visible) in &heroes {
        let Some(progress) = recall_progress(
            utility.state.recall_remaining_secs,
            stats.is_alive(),
            visible.is_none_or(|v| v.get()),
        ) else {
            continue;
        };
        #[cfg(feature = "qa")]
        {
            proof.drawn_heroes += 1;
            proof.max_remaining_secs = proof
                .max_remaining_secs
                .max(utility.state.recall_remaining_secs);
        }
        let p = transform.translation;
        let center = Vec3::new(p.x, map.terrain_height_3d(p.x, p.z) + 0.16, p.z);
        let point = |angle: f32, radius: f32, height: f32| {
            center + Vec3::new(angle.cos() * radius, height, angle.sin() * radius)
        };
        let mut line = |a: Vec3, b: Vec3, color: Color| {
            if is_2d {
                gizmos.line_2d(
                    crate::world2d::simulation_xz_to_render_xy(a),
                    crate::world2d::simulation_xz_to_render_xy(b),
                    color,
                );
            } else {
                gizmos.line(a, b, color);
            }
        };
        // Outer rune circle and a gold arc filling toward the teleport.
        for i in 0..64 {
            let a = i as f32 * std::f32::consts::TAU / 64.0;
            let b = (i + 1) as f32 * std::f32::consts::TAU / 64.0;
            line(point(a, 2.1, 0.0), point(b, 2.1, 0.0), cyan);
            if i as f32 / 64.0 <= progress {
                line(point(a, 2.4, 0.02), point(b, 2.4, 0.02), gold);
            }
        }
        // Eight rotating runes and rising sparks clearly identify a channel.
        for i in 0..8 {
            let a = phase + i as f32 * std::f32::consts::TAU / 8.0;
            line(point(a - 0.10, 1.9, 0.02), point(a, 1.6, 0.02), gold);
            line(point(a, 1.6, 0.02), point(a + 0.10, 1.9, 0.02), gold);
            let h = ((time.elapsed_secs() * 0.8 + i as f32 * 0.17) % 1.0) * 2.4;
            line(point(a, 0.75, h), point(a + 0.12, 0.72, h + 0.35), cyan);
        }
    }
}

#[derive(Resource, Default)]
struct RecallInput {
    /// Protect the request-to-first-snapshot gap, without inventing UI progress.
    pending_until: f64,
    cancelled: bool,
    interrupted_this_frame: bool,
}
impl RecallInput {
    fn active(&self, remaining: f32, now: f64) -> bool {
        remaining > 0.0 || self.pending_until > now
    }
    fn begin(&mut self, now: f64) {
        self.pending_until = now + 2.0;
        self.cancelled = false;
    }
}

fn prepare_recall_input(
    time: Res<Time<Real>>,
    context: Res<GameplayInputContext>,
    keys: Res<ButtonInput<KeyCode>>,
    mobile: Option<Res<MobileControls>>,
    local: Query<(Entity, &CombatStats, &PlayerUtility), With<Player>>,
    mut input: ResMut<RecallInput>,
    mut basic: ResMut<BasicAttackState>,
    mut pending: ResMut<PendingCast>,
    mut commands: Commands,
    mut network: MessageWriter<NetworkCommand>,
) {
    input.interrupted_this_frame = false;
    let Ok((entity, stats, utility)) = local.single() else {
        *input = RecallInput::default();
        return;
    };
    if !context.running || !stats.is_alive() {
        *input = RecallInput::default();
        return;
    }
    if !context.gameplay_allowed() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let active = input.active(utility.state.recall_remaining_secs, now);
    if !active {
        input.cancelled = false;
    }
    let mobile = mobile
        .as_deref()
        .filter(|m| m.enabled && m.focused && m.landscape);
    let key_action = keys.just_pressed(KeyCode::KeyB).then_some(if active {
        UtilityAction::CancelRecall
    } else {
        UtilityAction::Recall
    });
    if let Some(action) = key_action {
        network.write(NetworkCommand::Utility {
            action,
            direction: Vec2::ZERO,
        });
    }
    let start = key_action == Some(UtilityAction::Recall)
        || mobile.is_some_and(|m| m.utilities.iter().any(|(a, _)| *a == UtilityAction::Recall));
    if start {
        commands
            .entity(entity)
            .remove::<(MovementTarget, MovementRoute)>();
        basic.cancel_for_movement();
        pending.cancel();
        input.begin(now);
    }
    // Read raw touch intents before combat drains them: an empty attack, a
    // rejected cast, or pushing into a wall must still interrupt a channel.
    let interrupts = mobile.is_some_and(|m| {
        m.movement.length_squared() > 0.001
            || m.attack_pressed()
            || !m.attacks.is_empty()
            || !m.category_attacks.is_empty()
            || !m.casts.is_empty()
            || m.utilities
                .iter()
                .any(|(a, _)| matches!(a, UtilityAction::Dash | UtilityAction::Haste))
    }) || keys.any_just_pressed([
        KeyCode::KeyQ,
        KeyCode::KeyW,
        KeyCode::KeyE,
        KeyCode::KeyR,
        KeyCode::KeyA,
    ]);
    input.interrupted_this_frame = interrupts;
    if key_action == Some(UtilityAction::CancelRecall) {
        input.cancelled = true;
    }
}

fn cancel_for_new_movement(
    time: Res<Time<Real>>,
    local: Query<(&PlayerUtility, Option<Ref<MovementTarget>>), With<Player>>,
    mut input: ResMut<RecallInput>,
    mut network: ParamSet<(MessageReader<NetworkCommand>, MessageWriter<NetworkCommand>)>,
) {
    let actions: Vec<_> = network.p0().read().cloned().collect();
    let Ok((utility, target)) = local.single() else {
        return;
    };
    let interrupts = input.interrupted_this_frame
        || target.is_some_and(|t| t.is_changed())
        || actions.iter().any(|a| {
            matches!(
                a,
                NetworkCommand::BasicAttack { .. }
                    | NetworkCommand::Cast { .. }
                    | NetworkCommand::CastSkill { .. }
                    | NetworkCommand::Utility {
                        action: UtilityAction::Dash | UtilityAction::Haste,
                        ..
                    }
            )
        });
    if !input.cancelled
        && interrupts
        && input.active(utility.state.recall_remaining_secs, time.elapsed_secs_f64())
    {
        network.p1().write(NetworkCommand::Utility {
            action: UtilityAction::CancelRecall,
            direction: Vec2::ZERO,
        });
        input.cancelled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recall_pending_window_is_bounded_and_restarts_after_local_cancel() {
        let mut state = RecallInput::default();
        assert!(!state.active(0.0, 5.0));
        state.begin(5.0);
        assert!(state.active(0.0, 6.0));
        assert!(!state.active(0.0, 7.0));
        assert!(state.active(0.2, 8.0));
        state.cancelled = true;
        state.begin(10.0);
        assert!(!state.cancelled);
    }
    fn fixture() -> (App, Entity) {
        let mut app = App::new();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        app.init_resource::<Time<Real>>()
            .init_resource::<GameplayInputContext>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<RecallInput>()
            .init_resource::<BasicAttackState>()
            .init_resource::<PendingCast>()
            .insert_resource(mobile)
            .add_message::<NetworkCommand>()
            .add_systems(
                Update,
                (
                    prepare_recall_input,
                    forward_touch_utility,
                    cancel_for_new_movement,
                )
                    .chain(),
            );
        let hero = app
            .world_mut()
            .spawn((Player, CombatStats::default(), PlayerUtility::default()))
            .id();
        (app, hero)
    }
    fn forward_touch_utility(
        mut mobile: ResMut<MobileControls>,
        mut out: MessageWriter<NetworkCommand>,
    ) {
        for (action, _) in mobile.utilities.drain(..) {
            out.write(NetworkCommand::Utility {
                action,
                direction: Vec2::ZERO,
            });
        }
    }
    fn actions(app: &mut App) -> Vec<UtilityAction> {
        app.world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .filter_map(|message| {
                if let NetworkCommand::Utility { action, .. } = message {
                    Some(action)
                } else {
                    None
                }
            })
            .collect()
    }
    #[test]
    fn recall_touch_cancels_existing_route_and_same_frame_stick_cancels_after_start() {
        let (mut app, hero) = fixture();
        app.world_mut()
            .entity_mut(hero)
            .insert(MovementTarget { target: Vec3::X });
        let mut mobile = app.world_mut().resource_mut::<MobileControls>();
        mobile.utilities.push((UtilityAction::Recall, None));
        mobile.movement = Vec2::X;
        app.update();
        assert!(app.world().get::<MovementTarget>(hero).is_none());
        assert_eq!(
            actions(&mut app),
            [UtilityAction::Recall, UtilityAction::CancelRecall]
        );
    }
    #[test]
    fn recall_empty_attack_cancels_authoritative_channel_once_without_target_packet() {
        let (mut app, hero) = fixture();
        app.world_mut()
            .get_mut::<PlayerUtility>(hero)
            .unwrap()
            .state
            .recall_remaining_secs = 6.5;
        app.world_mut()
            .resource_mut::<MobileControls>()
            .start_attack_hold_for_test();
        app.update();
        assert_eq!(actions(&mut app), [UtilityAction::CancelRecall]);
        app.update();
        assert!(
            actions(&mut app).is_empty(),
            "held attack does not spam cancellation"
        );
    }
    #[test]
    fn recall_visual_tracks_channel_and_disappears_on_cancel_death_or_fog() {
        assert_eq!(recall_progress(7.0, true, true), Some(0.0));
        assert_eq!(recall_progress(3.5, true, true), Some(0.5));
        assert!(recall_progress(0.1, true, true).unwrap() > 0.98);
        for remaining in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(recall_progress(remaining, true, true), None);
        }
        assert_eq!(recall_progress(4.0, false, true), None);
        assert_eq!(recall_progress(4.0, true, false), None);
    }
}
