use crate::net::{NetworkCommand, SessionEvent};
use crate::player::MovementTarget;
use crate::targeting::BasicAttackState;
use bevy::prelude::*;

use super::cast::PendingCast;
use super::cooldown::LocalCastCooldown;
use super::feedback::ActionFeedback;
use super::selection::TargetState;

/// Drops the previous round's intents, cooldowns and queued gameplay commands
/// on [`SessionEvent::RoundChanged`]. `net` owns the round tracking: zero ids
/// are skipped and a reconnect to the same round (after the teardown gap) is
/// not a change. Runs after `ClientNetPipeline::ApplySnapshot`, whose flush
/// writes the event, and before input, so nothing from the old round is sent
/// in the frame the new one is applied.
pub(super) fn reset_round_input_state(
    mut commands: Commands,
    mut session_events: MessageReader<SessionEvent>,
    mut target: ResMut<TargetState>,
    orders: (ResMut<PendingCast>, ResMut<BasicAttackState>),
    mut cooldowns: ResMut<LocalCastCooldown>,
    mut feedback: ResMut<ActionFeedback>,
    moving: Query<Entity, With<MovementTarget>>,
    mut queued: ResMut<Messages<NetworkCommand>>,
) {
    let (mut pending, mut basic) = orders;
    let mut round_changed = false;
    for event in session_events.read() {
        round_changed |= matches!(event, SessionEvent::RoundChanged { .. });
    }
    if !round_changed {
        return;
    }
    target.selected_entity = None;
    target.selected_target = None;
    pending.cancel();
    *basic = default();
    *cooldowns = LocalCastCooldown::default();
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
                    | NetworkCommand::Utility { .. }
                    | NetworkCommand::BuyItem { .. }
            )
        })
        .collect();
    queued.write_batch(preserved);
}
