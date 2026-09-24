use crate::domain::RoundId;
use crate::net::{GameStateSnapshot, NetworkCommand};
use crate::player::MovementTarget;
use crate::targeting::BasicAttackState;
use bevy::prelude::*;

use super::cast::PendingCast;
use super::cooldown::LocalCastCooldown;
use super::feedback::ActionFeedback;
use super::selection::TargetState;

/// Keep valid identity across a transient disconnect (whose snapshot is empty).
#[derive(Resource, Default)]
pub(super) struct CombatRoundIdentity(Option<RoundId>);

pub(super) fn reset_round_input_state(
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
    let Some(identity) = RoundId::from_meta(&snapshot.meta) else {
        return;
    };
    let changed = previous.0.is_some_and(|last| last != identity);
    previous.0 = Some(identity);
    if !changed {
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
