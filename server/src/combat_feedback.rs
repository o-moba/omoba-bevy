//! Damage receipts are produced at HP mutation, then retained briefly for UDP loss.
use crate::*;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

pub(crate) const COMBAT_EVENT_CAPACITY: usize = 96;
pub(crate) const COMBAT_EVENT_RETENTION: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct CombatLog {
    next_id: u64,
    recent: VecDeque<(Instant, CombatEvent)>,
}

impl CombatLog {
    pub(crate) fn extend(&mut self, now: Instant, events: impl IntoIterator<Item = CombatEvent>) {
        self.prune(now);
        for mut event in events {
            self.next_id = self.next_id.saturating_add(1);
            event.id = self.next_id;
            self.recent.push_back((now, event));
            while self.recent.len() > COMBAT_EVENT_CAPACITY {
                self.recent.pop_front();
            }
        }
    }

    fn prune(&mut self, now: Instant) {
        while self
            .recent
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= COMBAT_EVENT_RETENTION)
        {
            self.recent.pop_front();
        }
    }

    pub(crate) fn snapshot(&mut self, now: Instant) -> Vec<CombatEvent> {
        self.prune(now);
        self.recent.iter().map(|(_, event)| event.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HitSource {
    pub(crate) entity: CombatEntity,
    pub(crate) style: ProjectileStyle,
    pub(crate) action_slot: Option<u8>,
}

impl HitSource {
    pub(crate) fn new(kind: CombatEntityKind, id: u64, style: ProjectileStyle) -> Self {
        Self {
            entity: CombatEntity { kind, id },
            style,
            action_slot: None,
        }
    }

    pub(crate) fn projectile(state: &ProjectileState) -> Self {
        Self {
            entity: CombatEntity {
                kind: state.source_kind,
                id: state.owner_id,
            },
            style: state.style,
            action_slot: state.action_slot,
        }
    }

    pub(crate) fn annotate(self, mut event: CombatEvent) -> CombatEvent {
        event.source = self.entity;
        event.style = self.style;
        event.action_slot = self.action_slot;
        event
    }
}

pub(crate) fn damage_receipt(
    kind: CombatEntityKind,
    id: u64,
    before: f32,
    after: f32,
    position: Vec3f,
) -> Option<CombatEvent> {
    let amount = before - after;
    (amount.is_finite() && amount > 0.0).then_some(CombatEvent {
        target: CombatEntity { kind, id },
        amount,
        x: position.x,
        y: position.y,
        z: position.z,
        killed: after <= 0.0,
        ..Default::default()
    })
}

pub(crate) fn apply_player_damage(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    target_id: u64,
    damage: f32,
    now: Instant,
) -> Option<CombatEvent> {
    if !damage.is_finite() || damage <= 0.0 {
        return None;
    }
    let player = players.values_mut().find(|player| {
        player.joined && player.state.id == target_id && player.state.hp > 0.0 && !player.god_mode
    })?;
    let before = player.state.hp;
    player.state.hp = (before - damage).max(0.0);
    if player.state.hp <= 0.0 && player.respawn_at.is_none() {
        player.respawn_at = Some(now + RESPAWN_DELAY);
    }
    damage_receipt(
        CombatEntityKind::Player,
        target_id,
        before,
        player.state.hp,
        Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z),
    )
}

pub(crate) fn minion_stats(kind: MinionKind) -> (f32, f32, f32, Duration) {
    match kind {
        MinionKind::Melee => (
            MINION_MAX_HP,
            MINION_ATTACK_DAMAGE,
            MINION_ATTACK_RANGE,
            MINION_ATTACK_COOLDOWN,
        ),
        MinionKind::Caster => (
            CASTER_MINION_MAX_HP,
            CASTER_MINION_ATTACK_DAMAGE,
            CASTER_MINION_ATTACK_RANGE,
            CASTER_MINION_ATTACK_COOLDOWN,
        ),
    }
}

pub(crate) fn spawn_caster_projectile(
    minion: &Minion,
    target: TargetId,
    position: Vec3f,
    projectiles: &mut HashMap<u64, Projectile>,
    next_id: &mut u64,
    now: Instant,
) {
    let origin = Vec3f::new(
        minion.state.x,
        minion.state.y + MINION_RADIUS * 0.8,
        minion.state.z,
    );
    let direction = Vec3f::new(
        position.x - origin.x,
        position.y - origin.y,
        position.z - origin.z,
    )
    .normalize_or_zero();
    let id = *next_id;
    *next_id += 1;
    projectiles.insert(
        id,
        Projectile {
            state: ProjectileState {
                id,
                owner_id: minion.state.id,
                owner_team: minion.state.team,
                source_kind: CombatEntityKind::Minion,
                style: ProjectileStyle::CasterBolt,
                action_slot: None,
                direction: [direction.x, direction.y, direction.z],
                x: origin.x,
                y: origin.y,
                z: origin.z,
            },
            target,
            velocity: Vec3f::new(
                direction.x * PROJECTILE_SPEED,
                direction.y * PROJECTILE_SPEED,
                direction.z * PROJECTILE_SPEED,
            ),
            homing: true,
            guaranteed_hit: true,
            damage: minion_stats(minion.state.kind).1,
            radius: PROJECTILE_RADIUS,
            expires_at: now + PROJECTILE_LIFETIME,
        },
    );
}
