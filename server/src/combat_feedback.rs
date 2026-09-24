//! Damage receipts are produced at HP mutation, then retained briefly for UDP loss.
use crate::*;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

pub(crate) const COMBAT_EVENT_CAPACITY: usize = 96;
pub(crate) const COMBAT_EVENT_RETENTION: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct CombatLog {
    pub(crate) ledger: crate::match_stats::RoundLedger,
    next_id: u64,
    sandbox: Option<(Instant, shared::sandbox::DamageAnalytics)>,
    recent: VecDeque<(Instant, CombatEvent)>,
}

impl CombatLog {
    pub(crate) fn extend(&mut self, now: Instant, events: impl IntoIterator<Item = CombatEvent>) {
        self.prune(now);
        for mut event in events {
            self.next_id = self.next_id.saturating_add(1);
            event.id = self.next_id;
            self.ledger.record(now, &event);
            if let Some((_, stats)) = &mut self.sandbox {
                if event.target.kind == CombatEntityKind::Player {
                    stats.damage += event.amount as f64;
                    stats.hits += 1;
                    stats.last_hit = event.amount;
                    let index = stats.breakdown.iter().position(|b| {
                        b.source_kind == event.source.kind
                            && b.source_id == event.source.id
                            && b.target_id == event.target.id
                            && b.slot == event.action_slot
                    });
                    if let Some(index) = index {
                        let b = &mut stats.breakdown[index];
                        b.hits += 1;
                        b.damage += event.amount as f64;
                        b.last_hit = event.amount;
                        b.last_event_id = event.id;
                    } else if stats.breakdown.len() < 1024 {
                        stats.breakdown.push(shared::sandbox::DamageBreakdown {
                            source_kind: event.source.kind,
                            last_event_id: event.id,
                            source_id: event.source.id,
                            target_id: event.target.id,
                            slot: event.action_slot,
                            hits: 1,
                            damage: event.amount as f64,
                            last_hit: event.amount,
                        });
                    }
                }
            }
            self.recent.push_back((now, event));
            while self.recent.len() > COMBAT_EVENT_CAPACITY {
                self.recent.pop_front();
            }
        }
    }

    pub(crate) fn enable_sandbox(&mut self, now: Instant) {
        if self.sandbox.is_none() {
            self.reset_sandbox(now);
        }
    }
    pub(crate) fn reset_sandbox(&mut self, now: Instant) {
        self.sandbox = Some((now, Default::default()));
    }
    pub(crate) fn sandbox_analytics(&self, now: Instant) -> shared::sandbox::DamageAnalytics {
        self.sandbox
            .as_ref()
            .map(|(start, stats)| {
                let mut stats = stats.clone();
                stats.elapsed_secs = now.saturating_duration_since(*start).as_secs_f64();
                stats.dps = stats.damage / stats.elapsed_secs.max(1.0 / 60.0);
                stats
            })
            .unwrap_or_default()
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
    apply_player_damage_typed(players, target_id, damage, now, false)
}
pub(crate) fn apply_player_damage_typed(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    target_id: u64,
    damage: f32,
    now: Instant,
    magical: bool,
) -> Option<CombatEvent> {
    if !damage.is_finite() || damage <= 0.0 {
        return None;
    }
    let player = players.values_mut().find(|player| {
        player.joined
            && player.hero.identity.id == target_id
            && player.hero.hp > 0.0
            && !player.modifiers.god_mode
    })?;
    let before = player.hero.hp;
    let damage = hero_stats::mitigate(player, damage, magical);
    player.hero.hp = if player.modifiers.infinite_hp {
        before
    } else {
        (before - damage).max(0.0)
    };
    if player.hero.hp <= 0.0 && player.timers.respawn_at.is_none() {
        player.timers.respawn_at = Some(now + RESPAWN_DELAY);
        player.timers.haste_expires_at = None;
    }
    let mut receipt = damage_receipt(
        CombatEntityKind::Player,
        target_id,
        before,
        if player.modifiers.infinite_hp {
            before - damage
        } else {
            player.hero.hp
        },
        Vec3f::new(player.hero.x, player.hero.y + AIM_HEIGHT, player.hero.z),
    );
    if player.modifiers.infinite_hp {
        if let Some(event) = &mut receipt {
            event.killed = false;
        }
    }
    receipt
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
