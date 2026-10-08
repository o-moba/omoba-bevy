//! Reusable staged techniques. State belongs to the skill/passive, not the avatar.
use super::*;
use shared::loadout::Technique;

#[derive(Debug, Clone, PartialEq)]
pub struct Recast {
    pub(super) skill: SkillId,
    pub until: Instant,
    target: Option<TargetId>,
    victims: Vec<TargetId>,
    pub uses: u8,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Orb {
    pub pos: [f32; 2],
    end: [f32; 2],
    pub(super) attached: Option<u64>,
    moving: bool,
    hits: BTreeSet<TargetKey>,
    skill: SkillId,
    slot: u8,
    scale: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Challenge {
    pub(super) target: TargetId,
    pub(super) sides: u8,
    until: Instant,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeroState {
    pub recasts: [Option<Recast>; 4],
    pub speed_until: Option<Instant>,
    pub parry_until: Option<Instant>,
    pub parried_control: bool,
    pub untargetable_until: Option<Instant>,
    pub unstoppable_until: Option<Instant>,
    pub defense_until: Option<Instant>,
    pub defense: f32,
    pub weakened_until: Option<Instant>,
    pub concussion_stacks: u8,
    pub brittle_until: Option<Instant>,
    pub intercept_until: Option<Instant>,
    pub intercept_direction: [f32; 2],
    pub intercepted: bool,
    pub last_combat: Option<Instant>,
    pub sustain_until: Option<Instant>,
    pub empowered_until: Option<Instant>,
    pub empowered: u8,
    pub flow_attacks: u8,
    pub flow_until: Option<Instant>,
    pub stacks: u8,
    pub stacks_until: Option<Instant>,
    pub last_target: Option<TargetId>,
    pub last_attack: Option<Instant>,
    pub souls: u16,
    pub essence: u8,
    pub orb: Option<Orb>,
    pub challenge: Option<Challenge>,
    pub charm: Option<([f32; 2], Instant)>,
    pub forged: bool,
    pub forge_ready: bool,
    pub forge_since: Option<Instant>,
}
impl HeroState {
    pub fn reset(&mut self) {
        *self = Self {
            souls: self.souls,
            forged: self.forged,
            ..Self::default()
        };
    }
    pub fn immune(&self, now: Instant) -> bool {
        remaining(self.parry_until, now) > 0.0 || remaining(self.untargetable_until, now) > 0.0
    }
    pub fn attack_rate(&self, now: Instant) -> f32 {
        1.0 + if remaining(self.flow_until, now) > 0.0 && self.flow_attacks > 0 {
            0.4
        } else {
            0.0
        } + if remaining(self.empowered_until, now) > 0.0 && self.empowered > 0 {
            0.5
        } else {
            0.0
        } + if remaining(self.stacks_until, now) > 0.0 {
            self.stacks as f32 * 0.08
        } else {
            0.0
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkKind {
    Seal,
    Curse,
    Concussion,
    Immunity,
    Brittle,
    Spikes,
}
#[derive(Clone)]
struct StatusMark {
    owner: u64,
    slot: u8,
    target: TargetId,
    kind: MarkKind,
    since: Instant,
    until: Instant,
    stacks: u8,
    amount: f32,
}
#[derive(Clone)]
struct Pillar {
    owner: u64,
    pos: [f32; 2],
    armed: Instant,
    until: Instant,
}
#[derive(Default)]
pub struct WorldState {
    npc_charms: Vec<(TargetId, [f32; 2], Instant)>,
    seen_deaths: BTreeSet<TargetKey>,
    healing: Vec<(u64, Team, [f32; 2], Instant)>,
    anchors: Vec<(u64, [f32; 2], Instant)>,
    marks: Vec<StatusMark>,
    pillars: Vec<Pillar>,
    souls: Vec<(u64, [f32; 2], Instant)>,
}
fn actor(w: &GameWorld, owner: u64) -> Option<&ConnectedPlayer> {
    w.players
        .values()
        .find(|p| p.joined && p.hero.identity.id == owner && p.hero.hp > 0.0)
}
fn actor_mut(w: &mut GameWorld, owner: u64) -> Option<&mut ConnectedPlayer> {
    w.players
        .values_mut()
        .find(|p| p.joined && p.hero.identity.id == owner && p.hero.hp > 0.0)
}
fn position(w: &GameWorld, owner: u64) -> Option<[f32; 2]> {
    actor(w, owner).map(|p| [p.hero.x, p.hero.z])
}
fn target(w: &GameWorld, id: TargetId) -> Option<Candidate> {
    candidates(w).into_iter().find(|c| c.target == id)
}
fn direction(from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
    let d = distance(from, to);
    if d < 0.001 {
        [0.0, 1.0]
    } else {
        [(to[0] - from[0]) / d, (to[1] - from[1]) / d]
    }
}
/// Which kinds a targeted technique may acquire near the aim point. The pick is
/// the nearest eligible candidate, so an ineligible one never shadows it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// Every kind the side admits.
    Any,
    /// Heroes only.
    Hero,
    /// Anything that can be moved: no structures.
    Unit,
}
fn select(
    w: &GameWorld,
    owner: u64,
    aim: [f32; 2],
    range: f32,
    ally: bool,
    pick: Pick,
    now: Instant,
) -> Option<Candidate> {
    let p = actor(w, owner)?;
    let origin = [p.hero.x, p.hero.z];
    candidates(w)
        .into_iter()
        .filter(|c| {
            (c.team == Some(p.hero.identity.team)) == ally
                && (!ally || matches!(c.target.kind, TargetKind::Player | TargetKind::Minion))
                && match pick {
                    Pick::Any => true,
                    Pick::Hero => c.target.kind == TargetKind::Player,
                    Pick::Unit => c.target.kind != TargetKind::Structure,
                }
                && distance(c.pos, aim) <= 2.0 + c.radius
                && distance(c.pos, origin) <= range + c.radius
                && (p.modifiers.bypass_vision
                    || crate::vision::target_visible(p.hero.identity.team, c.target, w, now))
        })
        .min_by(|a, b| {
            distance(a.pos, aim)
                .total_cmp(&distance(b.pos, aim))
                .then_with(|| key(a.target).cmp(&key(b.target)))
        })
}
fn nearest(
    w: &GameWorld,
    owner: u64,
    center: [f32; 2],
    radius: f32,
    now: Instant,
) -> Option<Candidate> {
    let p = actor(w, owner)?;
    candidates(w)
        .into_iter()
        .filter(|c| {
            hostile(c, p.hero.identity.team)
                && c.target.kind != TargetKind::Structure
                && distance(c.pos, center) <= radius + c.radius
                && (p.modifiers.bypass_vision
                    || crate::vision::target_visible(p.hero.identity.team, c.target, w, now))
        })
        .min_by(|a, b| {
            let marked = |c: &Candidate| {
                w.skill_runtime.advanced.marks.iter().any(|m| {
                    m.owner == owner
                        && m.target == c.target
                        && m.kind == MarkKind::Seal
                        && m.until > now
                })
            };
            marked(b)
                .cmp(&marked(a))
                .then_with(|| distance(a.pos, center).total_cmp(&distance(b.pos, center)))
                .then_with(|| key(a.target).cmp(&key(b.target)))
        })
}
fn heal(w: &mut GameWorld, owner: u64, amount: f32) {
    if let Some(p) = actor_mut(w, owner) {
        p.hero.hp = (p.hero.hp + amount).min(p.hero.max_hp);
    }
}
fn mark(
    w: &mut GameWorld,
    owner: u64,
    c: Candidate,
    kind: MarkKind,
    secs: f32,
    amount: f32,
    slot: u8,
    now: Instant,
) {
    let marks = &mut w.skill_runtime.advanced.marks;
    marks
        .retain(|m| !(m.owner == owner && m.target == c.target && m.kind == kind) && m.until > now);
    if marks.len() < MAX_MARKS {
        marks.push(StatusMark {
            owner,
            slot,
            target: c.target,
            kind,
            since: now,
            until: now + duration(secs),
            stacks: 1,
            amount,
        });
    }
}
pub fn terrain(w: &GameWorld, now: Instant) -> Vec<shared::navigation::Disc> {
    w.skill_runtime
        .advanced
        .pillars
        .iter()
        .filter(|p| p.armed <= now && p.until > now)
        .map(|p| shared::navigation::Disc {
            center: p.pos,
            radius: 1.0,
        })
        .collect()
}
fn move_to(w: &mut GameWorld, id: TargetId, end: [f32; 2], now: Instant) -> Option<[f32; 2]> {
    travel(w, id, end, now, false)
}
fn dash_to(w: &mut GameWorld, owner: u64, end: [f32; 2], now: Instant) -> Option<[f32; 2]> {
    travel(w, player_id(owner), end, now, true)
}
fn travel(
    w: &mut GameWorld,
    id: TargetId,
    end: [f32; 2],
    now: Instant,
    voluntary: bool,
) -> Option<[f32; 2]> {
    let c = target(w, id)?;
    if let Some(p) = actor(w, id.id).filter(|_| id.kind == TargetKind::Player) {
        if !voluntary
            && (p.modifiers.god_mode
                || remaining(p.hero.skills.advanced.unstoppable_until, now) > 0.0
                || p.hero.skills.advanced.immune(now))
        {
            return Some(c.pos);
        }
    }
    let clipped = shared::navigation::world_navigation().clip_movement(c.pos, end);
    let clipped = crate::session::clip_live_structures(c.pos, clipped, &w.structures);
    let clipped = shared::navigation::clip_discs(c.pos, clipped, &terrain(w, now));
    match id.kind {
        TargetKind::Player => {
            if let Some(p) = actor_mut(w, id.id) {
                if distance(c.pos, clipped) > 0.3 {
                    p.hero.utility.dash_sequence = p.hero.utility.dash_sequence.saturating_add(1);
                }
                p.hero.x = clipped[0];
                p.hero.z = clipped[1];
                p.timers.last_movement_at = now;
                p.timers.movement_slack = 0.0;
            }
        }
        TargetKind::Minion => {
            if let Some(p) = w.minions.get_mut(&id.id) {
                p.state.x = clipped[0];
                p.state.z = clipped[1];
            }
        }
        TargetKind::Neutral => {
            if let Some(p) = w.neutrals.get_mut(&id.id) {
                p.state.x = clipped[0];
                p.state.z = clipped[1];
            }
        }
        _ => return None,
    }
    Some(clipped)
}
fn player_id(owner: u64) -> TargetId {
    TargetId {
        kind: TargetKind::Player,
        id: owner,
    }
}
fn hit(
    w: &mut GameWorld,
    e: &ActiveEffect,
    c: Candidate,
    amount: f32,
    now: Instant,
) -> Vec<CombatEvent> {
    apply_hit(
        w,
        c.target,
        amount,
        skill(e.skill).damage_type,
        source(e.owner, e.slot),
        e.team,
        false,
        true,
        false,
        now,
    )
}
fn area(
    w: &mut GameWorld,
    e: &ActiveEffect,
    center: [f32; 2],
    radius: f32,
    damage: f32,
    root: f32,
    slow: f32,
    secs: f32,
    now: Instant,
) -> Vec<CombatEvent> {
    let mut out = Vec::new();
    for c in candidates(w).into_iter().filter(|c| {
        hostile(c, e.team)
            && c.target.kind != TargetKind::Structure
            && distance(c.pos, center) <= radius + c.radius
    }) {
        out.extend(hit(w, e, c, damage, now));
        control(w, c, e.owner, e.team, root, slow, secs, 0.0, now);
    }
    out
}
fn recast(
    p: &mut ConnectedPlayer,
    slot: u8,
    skill: SkillId,
    now: Instant,
    secs: f32,
    uses: u8,
    target: Option<TargetId>,
    victims: Vec<TargetId>,
) {
    // An old in-flight projectile cannot grant a free cast to a replacement
    // skill installed in the same button by a new sandbox incarnation.
    if !p
        .hero
        .skills
        .loadout
        .is_some_and(|l| l.skills()[slot as usize] == skill)
    {
        return;
    }
    p.hero.skills.advanced.recasts[slot as usize] = Some(Recast {
        skill,
        until: now + duration(secs),
        target,
        victims,
        uses,
    });
}

pub fn cast(w: &mut GameWorld, addr: SocketAddr, slot: u8, aim: [f32; 2], now: Instant) {
    if let Some(p) = w.players.get_mut(&addr) {
        ensure_orb(p);
    }
    let Some(p) = w.players.get(&addr) else {
        return;
    };
    let Some(l) = p.hero.skills.loadout else {
        return;
    };
    let def = l.skill(SkillSlot::from_index(slot).unwrap());
    let SkillEffect::Technique {
        action,
        damage,
        radius,
        duration_secs,
        speed: _,
    } = def.effect
    else {
        return;
    };
    let owner = p.hero.identity.id;
    let team = p.hero.identity.team;
    let origin = [p.hero.x, p.hero.z];
    let range = def.ability.cast_range;
    let follow = p.hero.skills.advanced.recasts[slot as usize]
        .clone()
        .filter(|r| r.skill == def.id && r.until > now && r.uses > 0);
    if p.hero.skills.advanced.immune(now)
        || remaining(p.hero.skills.control.stun_until, now) > 0.0
        || p.hero
            .skills
            .advanced
            .charm
            .is_some_and(|(_, until)| until > now)
    {
        return;
    }
    if !p.modifiers.no_cooldowns && remaining(p.hero.skills.recovery_until, now) > 0.0 {
        return;
    }
    if follow.is_none()
        && !p.modifiers.no_cooldowns
        && p.timers.last_cast_at[slot as usize].is_some_and(|at| {
            now.saturating_duration_since(at)
                < hero_stats::ability_cooldown(p, SkillSlot::from_index(slot).unwrap())
        })
    {
        return;
    }
    let len = distance(origin, aim);
    let dir = direction(origin, aim);
    if def.ability.targeting == shared::TargetingMode::Direction && len < 0.001 {
        return;
    }
    if def.ability.targeting == shared::TargetingMode::Point && len > range + 0.001 {
        return;
    }
    if remaining(p.hero.skills.control.root_until, now) > 0.0
        && matches!(
            action,
            Technique::Lunge
                | Technique::CollisionCharge
                | Technique::SpiritDash
                | Technique::BlinkShot
                | Technique::GuardLeap
                | Technique::AllyLeap
                | Technique::ExecuteRetreat
                | Technique::Lash
        )
    {
        return;
    }
    if action == Technique::BlinkShot && !legal_landing(w, aim, now) {
        return;
    }
    let picked = match action {
        Technique::VitalChallenge | Technique::Curse => {
            select(w, owner, aim, range, false, Pick::Hero, now)
        }
        // A kick throws its target; a structure cannot be thrown.
        Technique::ChainKick => select(w, owner, aim, range, false, Pick::Unit, now),
        Technique::Lash => select(w, owner, aim, range, false, Pick::Any, now),
        Technique::BallGuard => select(w, owner, aim, range, true, Pick::Hero, now),
        Technique::AllyLeap | Technique::GuardLeap => {
            select(w, owner, aim, range, true, Pick::Any, now)
        }
        _ => None,
    };
    if matches!(
        action,
        Technique::VitalChallenge
            | Technique::Curse
            | Technique::Lash
            | Technique::ChainKick
            | Technique::AllyLeap
            | Technique::BallGuard
    ) && picked.is_none()
    {
        return;
    }
    if matches!(action, Technique::EchoStrike | Technique::Hook)
        && follow.as_ref().is_some_and(|r| {
            r.target.and_then(|t| target(w, t)).is_none_or(|c| {
                distance(c.pos, origin) > 30.0
                    || !crate::vision::target_visible(team, c.target, w, now)
            })
        })
    {
        return;
    }
    if action == Technique::ReturningColossus
        && follow.is_some()
        && !w
            .skill_runtime
            .effects
            .values()
            .any(|e| e.owner == owner && e.skill == def.id && distance(e.pos, origin) <= 4.0)
    {
        return;
    }
    if matches!(action, Technique::BallField | Technique::BallPull)
        && p.hero.skills.advanced.orb.is_none()
    {
        return;
    }
    if active_count(w) + usize::from(action == Technique::GuardLeap) >= MAX_EFFECTS
        || w.skill_runtime
            .effects
            .values()
            .filter(|e| e.owner == owner)
            .count()
            >= MAX_OWNER_EFFECTS
    {
        return;
    }
    let rank = p.hero.progress.ranks[slot as usize].clamp(1, def.ability.max_rank);
    let cost = def.mana_cost(rank, follow.is_some());
    if !p.modifiers.infinite_resource && p.hero.mana < cost {
        return;
    }
    let scale = shared::rank_effect_scale(rank)
        * shared::hero_balance::ability_power_multiplier(
            p.hero.identity.hero_class,
            p.hero.progress.level,
        )
        * hero_stats::combat_bonuses(p).damage_multiplier
        * w.team_buffs.damage_multiplier(team, now);
    let was_hidden = camouflaged(p, now);
    let request = p.hero.skills.request_id;
    let p = w.players.get_mut(&addr).unwrap();
    if !p.modifiers.infinite_resource {
        p.hero.mana -= cost;
    }
    if follow.is_none() {
        p.timers.last_cast_at[slot as usize] = Some(now);
    }
    p.hero.skills.recovery_until = Some(now + duration(def.windup_secs.max(0.15)));
    accepted_technique(p, now);
    if let Some(r) = p.hero.skills.advanced.recasts[slot as usize]
        .as_mut()
        .filter(|r| r.skill == def.id)
    {
        r.uses = r.uses.saturating_sub(1);
    }
    crate::sim::cast::record_player_action(p, SkillSlot::from_index(slot).unwrap());
    if def.ability.targeting != shared::TargetingMode::SelfTarget {
        let facing = picked.map_or(aim, |c| c.pos);
        crate::sim::cast::face_player_action(p, facing[0] - origin[0], facing[1] - origin[1]);
    }
    let mut e = ActiveEffect {
        id: w.skill_runtime.id(),
        owner,
        team,
        skill: def.id,
        slot,
        pos: origin,
        origin,
        end: if def.ability.targeting == shared::TargetingMode::Point {
            aim
        } else {
            add(origin, dir, range)
        },
        direction: dir,
        expires: now + duration(duration_secs.max(range / 22.0 + 0.2)),
        armed_at: now + duration(def.windup_secs),
        scale,
        traveled: 0.0,
        returning: false,
        hits: BTreeSet::new(),
        hit_count: 0,
        fired: false,
        cast_request: request,
    };
    let mut out = Vec::new();
    let mut persistent = false;
    match action {
        Technique::DaggerDeadlyBlow
        | Technique::DaggerBluff
        | Technique::DaggerBackstab
        | Technique::DaggerLethalBlow => unreachable!("dagger dispatcher"),
        Technique::OnHitBolt
        | Technique::DetonationMark
        | Technique::CharmBolt
        | Technique::ConcussiveBolt
        | Technique::PiercingWave
        | Technique::ReturnOrb => persistent = true,
        Technique::EchoStrike | Technique::Hook => {
            if let Some(c) = follow.and_then(|r| r.target).and_then(|t| target(w, t)) {
                let landing = add(c.pos, direction(c.pos, origin), 1.0);
                dash_to(w, owner, landing, now);
                if action == Technique::EchoStrike {
                    out.extend(hit(
                        w,
                        &e,
                        c,
                        damage * scale + (c.max_hp - c.hp) * 0.12,
                        now,
                    ));
                }
            } else {
                persistent = true;
            }
        }
        Technique::SpikeVolley => {
            if let Some(ref r) = follow {
                let priority = r.target.and_then(|id| target(w, id)).filter(|c| {
                    distance(c.pos, origin) <= range + c.radius
                        && crate::vision::target_visible(team, c.target, w, now)
                });
                if let Some(c) = priority.or_else(|| nearest(w, owner, origin, range, now)) {
                    out.extend(hit(w, &e, c, damage * scale, now));
                }
            } else {
                persistent = true;
                recast(
                    actor_mut(w, owner).unwrap(),
                    slot,
                    def.id,
                    now,
                    duration_secs,
                    3,
                    None,
                    vec![],
                );
            }
        }
        Technique::TerrainLine => {
            for c in hits(&candidates(w), origin, e.end, radius)
                .into_iter()
                .filter(|c| hostile(c, team))
            {
                out.extend(hit(w, &e, c, damage * scale, now));
                control(w, c, owner, team, 0.0, 0.65, 1.5, 0.0, now);
            }
            w.skill_runtime
                .advanced
                .pillars
                .retain(|p| p.owner != owner && p.until > now);
            if w.skill_runtime.advanced.pillars.len() < 32 {
                w.skill_runtime.advanced.pillars.push(Pillar {
                    owner,
                    pos: e.end,
                    armed: now + duration(0.65),
                    until: now + duration(duration_secs),
                });
            }
            e.pos = e.end;
            e.armed_at = now + duration(0.65);
            persistent = true;
        }
        Technique::ConeBrittle => {
            actor_mut(w, owner)
                .unwrap()
                .hero
                .skills
                .advanced
                .unstoppable_until = Some(now + duration(duration_secs));
            e.armed_at = now + duration(duration_secs * 0.8);
            persistent = true;
        }
        Technique::CollisionCharge => {
            let actual = dash_to(w, owner, e.end, now).unwrap_or(origin);
            let collision = distance(actual, e.end) > 0.2;
            out.extend(area(
                w,
                &e,
                actual,
                radius,
                damage * scale,
                if collision { 1.0 } else { 0.0 },
                1.0,
                0.0,
                now,
            ));
            if collision {
                w.skill_runtime
                    .advanced
                    .pillars
                    .retain(|p| p.owner != owner || distance(p.pos, actual) > 3.0);
            }
        }
        Technique::ReturningColossus => {
            if follow.is_some() {
                if let Some(old) = w
                    .skill_runtime
                    .effects
                    .values_mut()
                    .find(|e| e.owner == owner && e.skill == def.id)
                {
                    old.direction = dir;
                    old.end = add(old.pos, dir, range);
                    old.returning = true;
                    old.hits.clear();
                    old.expires = now + duration(duration_secs);
                }
            } else {
                e.pos = e.end;
                e.end = origin;
                e.direction = direction(e.pos, origin);
                persistent = true;
                recast(
                    actor_mut(w, owner).unwrap(),
                    slot,
                    def.id,
                    now,
                    duration_secs,
                    1,
                    None,
                    vec![],
                );
            }
        }
        Technique::Lunge => {
            dash_to(w, owner, aim, now);
            if let Some(c) = nearest(w, owner, position(w, owner).unwrap_or(origin), radius, now) {
                out.extend(hit(w, &e, c, damage * scale, now));
                let p = actor_mut(w, owner).unwrap();
                p.timers.last_cast_at[slot as usize] =
                    Some(now - duration(def.ability.base_cooldown_secs * 0.5));
            }
        }
        Technique::Parry => {
            let p = actor_mut(w, owner).unwrap();
            p.hero.skills.advanced.parry_until = Some(now + duration(duration_secs));
            p.hero.skills.advanced.parried_control = false;
            e.armed_at = now + duration(duration_secs);
            e.expires = e.armed_at + duration(0.2);
            persistent = true;
        }
        Technique::DoubleStrike => {
            let p = actor_mut(w, owner).unwrap();
            p.hero.skills.advanced.empowered = 2;
            p.hero.skills.advanced.empowered_until = Some(now + duration(duration_secs));
        }
        Technique::VitalChallenge => {
            actor_mut(w, owner).unwrap().hero.skills.advanced.challenge = Some(Challenge {
                target: picked.unwrap().target,
                sides: 0,
                until: now + duration(duration_secs),
            });
        }
        Technique::GuardLeap | Technique::AllyLeap => {
            if follow.is_some() {
                actor_mut(w, owner)
                    .unwrap()
                    .hero
                    .skills
                    .advanced
                    .sustain_until = Some(now + duration(3.0));
            } else {
                let destination = picked.map_or(aim, |c| c.pos);
                if action == Technique::GuardLeap && picked.is_none() {
                    w.skill_runtime
                        .advanced
                        .anchors
                        .retain(|(id, _, until)| *id != owner && *until > now);
                    if w.skill_runtime.advanced.anchors.len() < 32 {
                        w.skill_runtime
                            .advanced
                            .anchors
                            .push((owner, aim, now + duration(5.0)));
                    }
                }
                dash_to(w, owner, destination, now);
                shield(w, owner, owner, damage * scale, duration_secs, now);
                if let Some(c) = picked.filter(|c| c.target.kind == TargetKind::Player) {
                    shield(w, c.target.id, owner, damage * scale, duration_secs, now);
                    let p = actor_mut(w, c.target.id).unwrap();
                    p.hero.skills.advanced.defense = 25.0;
                    p.hero.skills.advanced.defense_until = Some(now + duration(duration_secs));
                }
                if action == Technique::GuardLeap {
                    recast(
                        actor_mut(w, owner).unwrap(),
                        slot,
                        def.id,
                        now,
                        duration_secs,
                        1,
                        None,
                        vec![],
                    );
                } else {
                    let p = actor_mut(w, owner).unwrap();
                    p.hero.skills.advanced.defense = 25.0;
                    p.hero.skills.advanced.defense_until = Some(now + duration(duration_secs));
                }
            }
        }
        Technique::RevealPulse => {
            if let Some(r) = follow {
                for id in r.victims {
                    if let Some(c) = target(w, id) {
                        control(w, c, owner, team, 0.0, 0.5, 2.0, 0.0, now);
                    }
                }
            } else {
                let victims: Vec<_> = candidates(w)
                    .into_iter()
                    .filter(|c| {
                        hostile(c, team)
                            && distance(c.pos, origin) <= radius + c.radius
                            && c.target.kind != TargetKind::Structure
                    })
                    .collect();
                for c in &victims {
                    out.extend(hit(w, &e, *c, damage * scale, now));
                    control(w, *c, owner, team, 0.0, 1.0, 0.0, duration_secs, now);
                }
                recast(
                    actor_mut(w, owner).unwrap(),
                    slot,
                    def.id,
                    now,
                    duration_secs,
                    1,
                    None,
                    victims.iter().map(|c| c.target).collect(),
                );
            }
        }
        Technique::ChainKick => {
            let c = picked.unwrap();
            let end = add(c.pos, direction(origin, c.pos), 10.0);
            let actual = move_to(w, c.target, end, now).unwrap_or(c.pos);
            out.extend(hit(w, &e, c, damage * scale, now));
            control(w, c, owner, team, 1.0, 1.0, 0.0, 0.0, now);
            for other in hits(&candidates(w), c.pos, actual, radius)
                .into_iter()
                .filter(|v| v.target != c.target && hostile(v, team))
            {
                out.extend(hit(w, &e, other, damage * scale, now));
                control(w, other, owner, team, 1.0, 1.0, 0.0, 0.0, now);
            }
        }
        Technique::Curse => mark(
            w,
            owner,
            picked.unwrap(),
            MarkKind::Curse,
            duration_secs,
            0.0,
            slot,
            now,
        ),
        Technique::Lash => {
            let c = picked.unwrap();
            if was_hidden {
                dash_to(w, owner, add(c.pos, direction(c.pos, origin), 1.0), now);
            }
            out.extend(hit(
                w,
                &e,
                c,
                damage * scale * if was_hidden { 1.5 } else { 1.0 },
                now,
            ));
            actor_mut(w, owner)
                .unwrap()
                .hero
                .skills
                .advanced
                .speed_until = Some(now + duration(duration_secs));
        }
        Technique::ExecuteRetreat => {
            for c in candidates(w).into_iter().filter(|c| {
                hostile(c, team) && distance(c.pos, origin) <= range + c.radius && {
                    let d = direction(origin, c.pos);
                    d[0] * dir[0] + d[1] * dir[1] > 0.2
                }
            }) {
                out.extend(hit(
                    w,
                    &e,
                    c,
                    damage * scale * if c.hp / c.max_hp < 0.3 { 2.0 } else { 1.0 },
                    now,
                ));
            }
            dash_to(w, owner, add(origin, dir, -7.0), now);
            actor_mut(w, owner)
                .unwrap()
                .hero
                .skills
                .advanced
                .untargetable_until = Some(now + duration(duration_secs));
        }
        Technique::GuidedFires | Technique::BlinkShot | Technique::SpiritDash => {
            if action == Technique::BlinkShot {
                blink_to(w, owner, aim, now);
            } else if action == Technique::SpiritDash {
                dash_to(w, owner, aim, now);
            }
            let center = position(w, owner).unwrap_or(origin);
            for _ in 0..if action == Technique::GuidedFires {
                3
            } else {
                1
            } {
                if let Some(c) = nearest(
                    w,
                    owner,
                    center,
                    if action == Technique::GuidedFires {
                        range
                    } else {
                        radius
                    },
                    now,
                ) {
                    out.extend(hit(w, &e, c, damage * scale, now));
                }
            }
            if action == Technique::GuidedFires {
                actor_mut(w, owner)
                    .unwrap()
                    .hero
                    .skills
                    .advanced
                    .speed_until = Some(now + duration(duration_secs));
            }
            if action == Technique::SpiritDash && follow.is_none() {
                recast(
                    actor_mut(w, owner).unwrap(),
                    slot,
                    def.id,
                    now,
                    duration_secs,
                    2,
                    None,
                    vec![],
                );
            }
        }
        Technique::BallMove | Technique::BallGuard => {
            let p = actor_mut(w, owner).unwrap();
            let from = p
                .hero
                .skills
                .advanced
                .orb
                .as_ref()
                .map_or(origin, |b| b.pos);
            p.hero.skills.advanced.orb = Some(Orb {
                pos: from,
                end: if action == Technique::BallGuard {
                    picked.unwrap().pos
                } else {
                    aim
                },
                attached: picked
                    .filter(|c| c.target.kind == TargetKind::Player)
                    .map(|c| c.target.id),
                moving: true,
                hits: BTreeSet::new(),
                skill: def.id,
                slot,
                scale,
            });
        }
        Technique::BallField | Technique::BallPull => {
            let center = actor(w, owner)
                .unwrap()
                .hero
                .skills
                .advanced
                .orb
                .as_ref()
                .unwrap()
                .pos;
            e.pos = center;
            e.end = center;
            e.armed_at = now
                + duration(if action == Technique::BallPull {
                    duration_secs
                } else {
                    0.0
                });
            e.expires = e.armed_at
                + duration(if action == Technique::BallField {
                    duration_secs
                } else {
                    0.2
                });
            persistent = true;
        }
        Technique::Lantern => {
            w.skill_runtime.effects.retain(|_, old| {
                !(old.owner == owner
                    && matches!(
                        skill(old.skill).effect,
                        SkillEffect::Technique {
                            action: Technique::Lantern,
                            ..
                        }
                    ))
            });
            e.pos = aim;
            e.end = aim;
            persistent = true;
        }
        Technique::Sweep => {
            for c in candidates(w).into_iter().filter(|c| {
                hostile(c, team)
                    && c.target.kind != TargetKind::Structure
                    && distance(c.pos, origin) <= range + c.radius
            }) {
                out.extend(hit(w, &e, c, damage * scale, now));
                move_to(w, c.target, add(c.pos, dir, 3.0), now);
                control(w, c, owner, team, 0.0, 0.55, duration_secs, 0.0, now);
            }
        }
        Technique::SegmentCage => {
            e.pos = origin;
            e.end = origin;
            persistent = true;
        }
        Technique::InterceptShield => {
            let p = actor_mut(w, owner).unwrap();
            p.hero.skills.advanced.intercept_until = Some(now + duration(duration_secs));
            p.hero.skills.advanced.intercept_direction = dir;
            p.hero.skills.advanced.intercepted = false;
            e.end = add(origin, dir, radius);
            persistent = true;
        }
        Technique::GlacialFissure => persistent = true,
    }
    if persistent {
        w.skill_runtime.effects.insert(e.id, e);
    }
    w.skill_runtime.pending.extend(out);
}

pub(super) fn effect_tick(
    w: &mut GameWorld,
    e: &mut ActiveEffect,
    t: TickCtx,
    out: &mut Vec<CombatEvent>,
) -> bool {
    let now = t.now;
    let def = skill(e.skill);
    let SkillEffect::Technique {
        action,
        damage,
        radius,
        duration_secs,
        speed,
    } = def.effect
    else {
        return false;
    };
    // A pillar blocks for its whole timer whoever is alive, so the effect that
    // replicates it lives exactly as long as the pillar does.
    if action == Technique::TerrainLine {
        return w
            .skill_runtime
            .advanced
            .pillars
            .iter()
            .any(|p| p.owner == e.owner && p.pos == e.pos && p.until > now);
    }
    let Some(owner_pos) = position(w, e.owner) else {
        return false;
    };
    // The blast is resolved around the caster, so its telegraph stays on the caster.
    if action == Technique::ConeBrittle {
        e.pos = owner_pos;
        e.end = add(owner_pos, e.direction, def.ability.cast_range);
    }
    if now < e.armed_at {
        return true;
    }
    match action {
        Technique::InterceptShield => {
            e.pos = owner_pos;
            e.end = add(owner_pos, e.direction, radius);
            return true;
        }
        Technique::Lantern => {
            for c in candidates(w).into_iter().filter(|c| {
                c.team == Some(e.team)
                    && c.target.kind == TargetKind::Player
                    && distance(c.pos, e.pos) <= radius + c.radius
            }) {
                if e.hits.insert(key(c.target)) {
                    shield(
                        w,
                        c.target.id,
                        e.owner,
                        damage * e.scale,
                        duration_secs,
                        now,
                    );
                }
            }
            return true;
        }
        Technique::SegmentCage => {
            for c in candidates(w)
                .into_iter()
                .filter(|c| hostile(c, e.team) && c.target.kind != TargetKind::Structure)
            {
                for side in 0..5 {
                    let angle = side as f32 * std::f32::consts::TAU / 5.0;
                    let next = (side + 1) as f32 * std::f32::consts::TAU / 5.0;
                    let a = add(e.pos, [angle.cos(), angle.sin()], radius);
                    let b = add(e.pos, [next.cos(), next.sin()], radius);
                    if e.hit_count & (1 << side) == 0 && !hits(&[c], a, b, 0.4).is_empty() {
                        let amount = if e.hits.contains(&key(c.target)) {
                            0.0
                        } else {
                            damage * e.scale
                        };
                        out.extend(hit(w, e, c, amount, now));
                        control(w, c, e.owner, e.team, 0.0, 0.3, 2.0, 0.0, now);
                        e.hits.insert(key(c.target));
                        e.hit_count |= 1 << side;
                    }
                }
            }
            return e.hit_count != 31;
        }
        Technique::BallField => {
            if !e.fired {
                out.extend(area(
                    w,
                    e,
                    e.pos,
                    radius,
                    damage * e.scale,
                    0.0,
                    0.6,
                    0.2,
                    now,
                ));
                e.fired = true;
            }
            for c in candidates(w)
                .into_iter()
                .filter(|c| distance(c.pos, e.pos) <= radius + c.radius)
            {
                if hostile(&c, e.team) {
                    control(w, c, e.owner, e.team, 0.0, 0.6, 0.2, 0.0, now);
                } else if c.target.kind == TargetKind::Player {
                    if let Some(p) = actor_mut(w, c.target.id) {
                        p.hero.skills.advanced.speed_until = Some(now + duration(0.2));
                    }
                }
            }
            return true;
        }
        Technique::BallPull => {
            for c in candidates(w).into_iter().filter(|c| {
                hostile(c, e.team)
                    && c.target.kind != TargetKind::Structure
                    && distance(c.pos, e.pos) <= radius + c.radius
            }) {
                out.extend(hit(w, e, c, damage * e.scale, now));
                move_to(w, c.target, add(e.pos, direction(e.pos, c.pos), 1.0), now);
                control(w, c, e.owner, e.team, 0.7, 1.0, 0.0, 0.0, now);
            }
            return false;
        }
        Technique::ConeBrittle => {
            for c in candidates(w).into_iter().filter(|c| {
                hostile(c, e.team)
                    && distance(c.pos, owner_pos) <= def.ability.cast_range + c.radius
                    && {
                        let d = direction(owner_pos, c.pos);
                        d[0] * e.direction[0] + d[1] * e.direction[1] > 0.6
                    }
            }) {
                out.extend(hit(w, e, c, damage * e.scale, now));
                mark(w, e.owner, c, MarkKind::Brittle, 4.0, 0.07, e.slot, now);
            }
            return false;
        }
        Technique::Parry => {
            let empowered =
                actor(w, e.owner).is_some_and(|p| p.hero.skills.advanced.parried_control);
            if let Some(c) = hits(&candidates(w), e.pos, e.end, radius)
                .into_iter()
                .find(|c| hostile(c, e.team) && c.target.kind != TargetKind::Structure)
            {
                out.extend(hit(w, e, c, damage * e.scale, now));
                control(
                    w,
                    c,
                    e.owner,
                    e.team,
                    if empowered { 1.5 } else { 0.0 },
                    0.5,
                    1.5,
                    0.0,
                    now,
                );
            }
            return false;
        }
        Technique::GlacialFissure => {
            for c in hits(&candidates(w), e.origin, e.end, radius)
                .into_iter()
                .filter(|c| hostile(c, e.team) && c.target.kind != TargetKind::Structure)
            {
                if e.hits.insert(key(c.target)) {
                    out.extend(hit(w, e, c, damage * e.scale, now));
                    control(w, c, e.owner, e.team, 1.0, 0.5, 0.2, 0.0, now);
                } else {
                    control(w, c, e.owner, e.team, 0.0, 0.5, 0.2, 0.0, now);
                }
            }
            return true;
        }
        _ => {}
    }
    let mut end = e.end;
    if action == Technique::ReturnOrb && e.returning {
        end = owner_pos;
        e.direction = direction(e.pos, end);
    }
    let travel = (speed * t.dt).min(distance(e.pos, end));
    let to = add(e.pos, e.direction, travel);
    if let Some((holder, factor)) = intercept(w, e.team, e.pos, to, radius, now) {
        if let Some(c) = target(w, player_id(holder)) {
            out.extend(hit(w, e, c, damage * e.scale * factor, now));
        }
        return false;
    }
    let cs = hits(&candidates(w), e.pos, to, radius);
    for c in cs.into_iter().filter(|c| hostile(c, e.team)) {
        if e.hits.contains(&key(c.target)) {
            continue;
        }
        if action == Technique::DetonationMark
            && !matches!(c.target.kind, TargetKind::Player | TargetKind::Structure)
        {
            continue;
        }
        if c.target.kind == TargetKind::Structure && action != Technique::DetonationMark {
            continue;
        }
        e.hits.insert(key(c.target));
        e.hit_count = e.hit_count.saturating_add(1);
        match action {
            Technique::DetonationMark => mark(
                w,
                e.owner,
                c,
                MarkKind::Seal,
                duration_secs,
                damage * e.scale,
                e.slot,
                now,
            ),
            Technique::OnHitBolt => {
                out.extend(apply_hit(
                    w,
                    c.target,
                    damage * e.scale,
                    def.damage_type,
                    source(e.owner, e.slot),
                    e.team,
                    true,
                    true,
                    false,
                    now,
                ));
                if let Some(p) = actor_mut(w, e.owner) {
                    for at in p.timers.last_cast_at.iter_mut().flatten() {
                        *at = at.checked_sub(duration(1.0)).unwrap_or(*at);
                    }
                }
            }
            Technique::ReturnOrb if e.returning => out.extend(apply_hit(
                w,
                c.target,
                damage * e.scale,
                DamageType::True,
                source(e.owner, e.slot),
                e.team,
                false,
                true,
                false,
                now,
            )),
            Technique::PiercingWave => out.extend(hit(
                w,
                e,
                c,
                damage
                    * e.scale
                    * if c.target.kind == TargetKind::Player {
                        1.0
                    } else {
                        0.5
                    },
                now,
            )),
            _ => out.extend(hit(w, e, c, damage * e.scale, now)),
        }
        match action {
            Technique::SpikeVolley => {
                mark(
                    w,
                    e.owner,
                    c,
                    MarkKind::Spikes,
                    duration_secs,
                    8.0 * e.scale,
                    e.slot,
                    now,
                );
                if let Some(m) = w.skill_runtime.advanced.marks.iter_mut().find(|m| {
                    m.owner == e.owner && m.target == c.target && m.kind == MarkKind::Spikes
                }) {
                    m.stacks = 3;
                }
                if let Some(r) = actor_mut(w, e.owner)
                    .and_then(|p| p.hero.skills.advanced.recasts[e.slot as usize].as_mut())
                    .filter(|r| r.skill == e.skill)
                {
                    r.target = Some(c.target);
                }
            }
            Technique::CharmBolt => charm(w, c, owner_pos, duration_secs, now),
            Technique::ConcussiveBolt => {
                concussion(w, e.owner, e.slot, c, now, out);
                control(w, c, e.owner, e.team, 0.0, 0.55, duration_secs, 0.0, now);
            }
            Technique::EchoStrike | Technique::Hook => {
                if let Some(p) = actor_mut(w, e.owner) {
                    recast(
                        p,
                        e.slot,
                        e.skill,
                        now,
                        duration_secs,
                        1,
                        Some(c.target),
                        vec![],
                    );
                }
                control(
                    w,
                    c,
                    e.owner,
                    e.team,
                    if action == Technique::Hook {
                        duration_secs
                    } else {
                        0.0
                    },
                    1.0,
                    0.0,
                    duration_secs,
                    now,
                );
                if action == Technique::Hook {
                    move_to(
                        w,
                        c.target,
                        add(c.pos, direction(c.pos, owner_pos), 2.0),
                        now,
                    );
                }
            }
            Technique::ReturningColossus => {
                control(
                    w,
                    c,
                    e.owner,
                    e.team,
                    if e.returning { 1.0 } else { 0.0 },
                    0.6,
                    1.5,
                    0.0,
                    now,
                );
                mark(w, e.owner, c, MarkKind::Brittle, 4.0, 0.07, e.slot, now);
            }
            _ => {}
        }
        if !matches!(
            action,
            Technique::ReturnOrb | Technique::PiercingWave | Technique::ReturningColossus
        ) {
            return false;
        }
    }
    e.pos = to;
    e.traveled += travel;
    if distance(to, end) < 0.01 {
        if action == Technique::ReturnOrb && !e.returning {
            e.returning = true;
            e.hits.clear();
            e.direction = direction(e.pos, owner_pos);
            e.expires = now + duration(3.0);
            true
        } else {
            false
        }
    } else {
        true
    }
}
fn charm(w: &mut GameWorld, c: Candidate, toward: [f32; 2], secs: f32, now: Instant) {
    if !crowd_control::apply(
        w,
        c,
        0,
        c.team.unwrap_or(Team::Green),
        secs,
        1.0,
        0.0,
        0.0,
        now,
        crowd_control::Kind::Charm,
    ) {
        return;
    }
    if c.target.kind == TargetKind::Player {
        if let Some(p) = actor_mut(w, c.target.id) {
            p.hero.skills.advanced.charm = Some((toward, now + duration(secs)));
        }
    } else {
        w.skill_runtime
            .advanced
            .npc_charms
            .retain(|(t, _, until)| *t != c.target && *until > now);
        if w.skill_runtime.advanced.npc_charms.len() < MAX_STATUS_SOURCES {
            w.skill_runtime
                .advanced
                .npc_charms
                .push((c.target, toward, now + duration(secs)));
        }
    }
}
fn concussion(
    w: &mut GameWorld,
    owner: u64,
    slot: u8,
    c: Candidate,
    now: Instant,
    out: &mut Vec<CombatEvent>,
) {
    if c.target.kind == TargetKind::Structure {
        return;
    }
    if w.skill_runtime
        .advanced
        .marks
        .iter()
        .any(|m| m.target == c.target && m.kind == MarkKind::Immunity && m.until > now)
    {
        return;
    }
    if let Some(m) = w
        .skill_runtime
        .advanced
        .marks
        .iter_mut()
        .find(|m| m.target == c.target && m.kind == MarkKind::Concussion && m.until > now)
    {
        m.stacks += 1;
        if m.stacks < 4 {
            return;
        }
    } else {
        mark(w, owner, c, MarkKind::Concussion, 4.0, 20.0, slot, now);
        return;
    }
    w.skill_runtime
        .advanced
        .marks
        .retain(|m| !(m.target == c.target && m.kind == MarkKind::Concussion));
    mark(w, owner, c, MarkKind::Immunity, 7.0, 0.0, slot, now);
    let team = actor(w, owner).map_or(Team::Green, |p| p.hero.identity.team);
    out.extend(raw_damage(
        w,
        c.target,
        20.0,
        DamageType::Magic,
        source(owner, slot),
        team,
        now,
    ));
    control(w, c, owner, team, 1.25, 1.0, 0.0, 0.0, now);
}
/// Called by damage resolution once, never recursively by bonus damage.
pub(super) fn on_hit(
    w: &mut GameWorld,
    c: Candidate,
    amount: f32,
    src: HitSource,
    team: Team,
    basic: bool,
    now: Instant,
) -> Vec<CombatEvent> {
    let owner = src.entity.id;
    let Some(p) = actor(w, owner) else {
        return vec![];
    };
    let passive = p.hero.skills.loadout.map(|l| l.passive());
    let pos = [p.hero.x, p.hero.z];
    let mut out = Vec::new();
    let mut bonus = 0.0;
    let mut physical_bonus = 0.0;
    let charged_attack = p.hero.skills.loadout.is_some_and(|l| {
        l.skills().into_iter().any(|id| {
            matches!(
                skill(id).effect,
                SkillEffect::Technique {
                    action: Technique::Sweep,
                    ..
                }
            )
        })
    });
    let mut true_bonus = 0.0;
    let relevant: Vec<_> = w
        .skill_runtime
        .advanced
        .marks
        .iter()
        .filter(|m| {
            m.target == c.target
                && m.owner == owner
                && m.until > now
                && matches!(
                    m.kind,
                    MarkKind::Seal | MarkKind::Curse | MarkKind::Brittle | MarkKind::Spikes
                )
        })
        .cloned()
        .collect();
    for m in relevant {
        if m.kind == MarkKind::Brittle && !basic {
            continue;
        }
        w.skill_runtime
            .advanced
            .marks
            .retain(|old| !(old.target == m.target && old.owner == m.owner && old.kind == m.kind));
        match m.kind {
            MarkKind::Spikes => {
                bonus += m.amount;
                if m.stacks > 1 {
                    let mut next = m;
                    next.stacks -= 1;
                    w.skill_runtime.advanced.marks.push(next);
                }
            }
            MarkKind::Seal => {
                bonus += m.amount;
                if !basic {
                    if let Some(p) = actor_mut(w, owner) {
                        p.hero.mana = (p.hero.mana + 20.0).min(p.hero.max_mana);
                    }
                }
            }
            MarkKind::Brittle => {
                bonus += c.max_hp * m.amount;
                control(w, c, owner, team, 0.5, 1.0, 0.0, 0.0, now);
            }
            MarkKind::Curse => {
                if now.saturating_duration_since(m.since) >= duration(2.0) {
                    charm(w, c, pos, 1.5, now);
                    if let Some(p) =
                        actor_mut(w, c.target.id).filter(|_| c.target.kind == TargetKind::Player)
                    {
                        p.hero.skills.advanced.weakened_until = Some(now + duration(3.0));
                    }
                } else {
                    control(w, c, owner, team, 0.0, 0.5, 1.0, 0.0, now);
                }
            }
            _ => {}
        }
    }
    let ally_mark = w
        .skill_runtime
        .advanced
        .marks
        .iter()
        .find(|m| {
            m.target == c.target
                && m.kind == MarkKind::Concussion
                && m.until > now
                && actor(w, m.owner).is_some_and(|p| p.hero.identity.team == team)
        })
        .map(|m| (m.owner, m.slot));
    if basic
        && passive == Some(PassiveId::Concussion)
        && w.skill_runtime.advanced.marks.iter().any(|m| {
            m.owner == owner
                && m.target == c.target
                && m.kind == MarkKind::Immunity
                && m.until > now
        })
    {
        bonus += 8.0;
    }
    if basic && (passive == Some(PassiveId::Concussion) || ally_mark.is_some()) {
        let (mark_owner, mark_slot) =
            ally_mark.unwrap_or((owner, shared::BASIC_ATTACK_ACTION_SLOT));
        concussion(w, mark_owner, mark_slot, c, now, &mut out);
    }
    if let Some(p) = actor_mut(w, owner) {
        let s = &mut p.hero.skills.advanced;
        s.last_combat = Some(now);
        if passive == Some(PassiveId::Resonance) && src.action_slot.is_some_and(|s| s < 4) {
            if remaining(s.stacks_until, now) == 0.0 {
                s.stacks = 0;
            }
            s.stacks = (s.stacks + 1).min(5);
            s.stacks_until = Some(now + duration(5.0));
        }
        if remaining(s.sustain_until, now) > 0.0 {
            p.hero.hp = (p.hero.hp + amount * 0.2).min(p.hero.max_hp);
        }
        if basic {
            if passive == Some(PassiveId::Flow)
                && s.flow_attacks > 0
                && remaining(s.flow_until, now) > 0.0
            {
                s.flow_attacks -= 1;
                p.hero.mana = (p.hero.mana + 15.0).min(p.hero.max_mana);
            }
            if s.empowered > 0 && remaining(s.empowered_until, now) > 0.0 {
                if s.empowered == 1 {
                    physical_bonus += amount;
                }
                s.empowered -= 1;
            }
            if passive == Some(PassiveId::Clockwork) {
                s.stacks =
                    if s.last_target == Some(c.target) && remaining(s.stacks_until, now) > 0.0 {
                        (s.stacks + 1).min(3)
                    } else {
                        1
                    };
                s.stacks_until = Some(now + duration(4.0));
                s.last_target = Some(c.target);
                bonus += s.stacks as f32 * 4.0;
            }
            if charged_attack {
                bonus += s.last_attack.map_or(12.0, |at| {
                    now.saturating_duration_since(at).as_secs_f32().min(4.0) * 3.0
                }) + s.souls as f32 * 0.2;
            }
            s.last_attack = Some(now);
        }
    }
    if basic
        && actor(w, owner).is_some_and(|p| {
            p.hero.skills.advanced.empowered == 1
                && remaining(p.hero.skills.advanced.empowered_until, now) > 0.0
        })
    {
        control(w, c, owner, team, 0.0, 0.5, 1.0, 0.0, now);
    }
    if (passive == Some(PassiveId::Vitals)
        || actor(w, owner).is_some_and(|p| {
            p.hero
                .skills
                .advanced
                .challenge
                .as_ref()
                .is_some_and(|v| v.target == c.target && v.until > now)
        }))
        && c.target.kind == TargetKind::Player
    {
        let diff = [pos[0] - c.pos[0], pos[1] - c.pos[1]];
        let side = if diff[0].abs() > diff[1].abs() {
            if diff[0] > 0.0 { 0 } else { 2 }
        } else if diff[1] > 0.0 {
            1
        } else {
            3
        };
        let p = actor_mut(w, owner).unwrap();
        let s = &mut p.hero.skills.advanced;
        let challenge = s
            .challenge
            .as_mut()
            .filter(|v| v.target == c.target && v.until > now);
        let valid = if let Some(v) = challenge {
            let unseen = v.sides & (1 << side) == 0;
            if unseen {
                v.sides |= 1 << side;
            }
            unseen
        } else {
            let expected = (c.target.id as u8).wrapping_add(s.essence) % 4;
            if side == expected {
                s.essence = s.essence.wrapping_add(1);
                true
            } else {
                false
            }
        };
        if valid {
            true_bonus = c.max_hp * 0.035;
            p.hero.hp = (p.hero.hp + 12.0).min(p.hero.max_hp);
            s.speed_until = Some(now + duration(1.5));
        }
    }
    if bonus > 0.0 {
        out.extend(raw_damage(
            w,
            c.target,
            bonus,
            DamageType::Magic,
            src,
            team,
            now,
        ));
    }
    if physical_bonus > 0.0 {
        out.extend(raw_damage(
            w,
            c.target,
            physical_bonus,
            DamageType::Physical,
            src,
            team,
            now,
        ));
    }
    if true_bonus > 0.0 {
        out.extend(raw_damage(
            w,
            c.target,
            true_bonus,
            DamageType::True,
            src,
            team,
            now,
        ));
    }
    out
}
pub fn camouflaged(p: &ConnectedPlayer, now: Instant) -> bool {
    p.hero
        .skills
        .loadout
        .is_some_and(|l| l.passive() == PassiveId::Shroud)
        && p.hero.progress.level >= 6
        && p.hero.hp > 0.0
        && p.hero
            .skills
            .advanced
            .last_combat
            .is_none_or(|at| now.saturating_duration_since(at) >= duration(4.0))
}
/// Returns a shield holder and remaining damage multiplier for a swept projectile.
pub fn intercept(
    w: &mut GameWorld,
    team: Team,
    from: [f32; 2],
    to: [f32; 2],
    radius: f32,
    now: Instant,
) -> Option<(u64, f32)> {
    intercept_players(&mut w.players, team, from, to, radius, now)
}
pub fn intercept_players(
    players: &mut std::collections::HashMap<SocketAddr, ConnectedPlayer>,
    team: Team,
    from: [f32; 2],
    to: [f32; 2],
    radius: f32,
    now: Instant,
) -> Option<(u64, f32)> {
    let selected = players
        .values()
        .filter(|p| {
            p.joined
                && p.hero.hp > 0.0
                && p.hero.identity.team != team
                && remaining(p.hero.skills.advanced.intercept_until, now) > 0.0
        })
        .filter_map(|p| {
            let d = p.hero.skills.advanced.intercept_direction;
            let center = add([p.hero.x, p.hero.z], d, 1.0);
            let signed = |v: [f32; 2]| (v[0] - center[0]) * d[0] + (v[1] - center[1]) * d[1];
            let a = signed(from);
            let b = signed(to);
            if a < -radius || b > radius || b >= a {
                return None;
            }
            let fraction = (a / (a - b)).clamp(0.0, 1.0);
            let point = [
                from[0] + (to[0] - from[0]) * fraction,
                from[1] + (to[1] - from[1]) * fraction,
            ];
            let lateral = ((point[0] - center[0]) * (-d[1]) + (point[1] - center[1]) * d[0]).abs();
            (lateral <= 2.5 + radius).then_some((p.hero.identity.id, fraction))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)))?
        .0;
    let s = &mut players
        .values_mut()
        .find(|p| p.hero.identity.id == selected)?
        .hero
        .skills
        .advanced;
    let factor = if s.intercepted { 0.35 } else { 0.0 };
    s.intercepted = true;
    Some((selected, factor))
}

pub fn tick(w: &mut GameWorld, t: TickCtx, out: &mut Vec<CombatEvent>) {
    let now = t.now;
    w.skill_runtime
        .advanced
        .npc_charms
        .retain(|(_, _, until)| *until > now);
    for (id, toward, _) in w.skill_runtime.advanced.npc_charms.clone() {
        if let Some(c) = target(w, id) {
            move_to(
                w,
                id,
                add(
                    c.pos,
                    direction(c.pos, toward),
                    (crate::balance::PLAYER_SPEED * 0.45 * t.dt).min(distance(c.pos, toward)),
                ),
                now,
            );
        }
    }
    let alive: BTreeSet<_> = candidates(w).into_iter().map(|c| key(c.target)).collect();
    w.skill_runtime
        .advanced
        .seen_deaths
        .retain(|key| !alive.contains(key));
    w.skill_runtime
        .advanced
        .marks
        .retain(|m| m.until > now && alive.contains(&key(m.target)));
    for p in w.players.values_mut() {
        p.hero.skills.advanced.concussion_stacks = w
            .skill_runtime
            .advanced
            .marks
            .iter()
            .filter(|m| m.target == player_id(p.hero.identity.id) && m.kind == MarkKind::Concussion)
            .map(|m| m.stacks)
            .max()
            .unwrap_or(0);
        p.hero.skills.advanced.brittle_until = w
            .skill_runtime
            .advanced
            .marks
            .iter()
            .filter(|m| m.target == player_id(p.hero.identity.id) && m.kind == MarkKind::Brittle)
            .map(|m| m.until)
            .max();
    }
    w.skill_runtime.advanced.pillars.retain(|p| p.until > now);
    let owners: Vec<_> = w
        .players
        .values()
        .filter(|p| p.joined && p.hero.hp > 0.0)
        .map(|p| p.hero.identity.id)
        .collect();
    for owner in owners {
        if let Some(p) = actor_mut(w, owner) {
            ensure_orb(p);
        }
        let Some(p) = actor(w, owner) else { continue };
        let pos = [p.hero.x, p.hero.z];
        let team = p.hero.identity.team;
        let passive = p.hero.skills.loadout.map(|l| l.passive());
        if let Some((toward, until)) = p
            .hero
            .skills
            .advanced
            .charm
            .filter(|(_, until)| *until > now)
        {
            let step = (crate::balance::PLAYER_SPEED * 0.45 * t.dt).min(distance(pos, toward));
            move_to(
                w,
                player_id(owner),
                add(pos, direction(pos, toward), step),
                now,
            );
            let _ = until;
        }
        if passive == Some(PassiveId::Shroud)
            && actor(w, owner).is_some_and(|p| {
                p.hero
                    .skills
                    .advanced
                    .last_combat
                    .is_none_or(|at| now.saturating_duration_since(at) > duration(4.0))
                    && p.hero.hp < p.hero.max_hp * 0.5
            })
        {
            heal(w, owner, 6.0 * t.dt);
        }
        if let Some(mut orb) = actor_mut(w, owner).and_then(|p| p.hero.skills.advanced.orb.take()) {
            if let Some(ally) = orb.attached.and_then(|id| position(w, id)) {
                orb.end = ally;
            } else if orb.attached.is_some() {
                orb.attached = Some(owner);
                orb.end = pos;
                orb.moving = true;
            }
            if distance(orb.pos, pos) > 24.0 {
                orb.attached = Some(owner);
                orb.end = pos;
                orb.moving = true;
                orb.hits.clear();
            }
            if orb.moving {
                let SkillEffect::Technique {
                    speed,
                    damage,
                    radius,
                    ..
                } = skill(orb.skill).effect
                else {
                    unreachable!()
                };
                let to = add(
                    orb.pos,
                    direction(orb.pos, orb.end),
                    (speed * t.dt).min(distance(orb.pos, orb.end)),
                );
                for c in hits(&candidates(w), orb.pos, to, radius)
                    .into_iter()
                    .filter(|c| hostile(c, team) && c.target.kind != TargetKind::Structure)
                {
                    if orb.hits.insert(key(c.target)) {
                        let factor = (1.0 - 0.1 * (orb.hits.len() - 1) as f32).max(0.4);
                        out.extend(apply_hit(
                            w,
                            c.target,
                            damage * orb.scale * factor,
                            DamageType::Magic,
                            source(owner, orb.slot),
                            team,
                            false,
                            true,
                            false,
                            now,
                        ));
                    }
                }
                orb.pos = to;
                if distance(to, orb.end) < 0.01 {
                    orb.moving = false;
                    if let Some(ally) = orb.attached {
                        shield(w, ally, owner, damage * orb.scale, 4.0, now);
                    }
                }
            } else if let Some(id) = orb.attached {
                if let Some(ally) = position(w, id) {
                    orb.pos = ally;
                }
            }
            if let Some(ally) = orb.attached.filter(|_| !orb.moving) {
                if let Some(p) = actor_mut(w, ally) {
                    p.hero.skills.advanced.defense = 15.0;
                    p.hero.skills.advanced.defense_until = Some(now + duration(0.2));
                }
            }
            if let Some(p) = actor_mut(w, owner) {
                p.hero.skills.advanced.orb = Some(orb);
            }
        }
        let challenge = actor(w, owner).and_then(|p| p.hero.skills.advanced.challenge.clone());
        if let Some(c) = challenge {
            if c.sides == 15 || (c.sides > 0 && target(w, c.target).is_none()) {
                if w.skill_runtime.advanced.healing.len() < 32 && active_count(w) + 1 < MAX_EFFECTS
                {
                    w.skill_runtime
                        .advanced
                        .healing
                        .push((owner, team, pos, now + duration(4.0)));
                }
                actor_mut(w, owner).unwrap().hero.skills.advanced.challenge = None;
            } else if c.until <= now {
                actor_mut(w, owner).unwrap().hero.skills.advanced.challenge = None;
            }
        }
    }
    w.skill_runtime
        .advanced
        .healing
        .retain(|(_, _, _, until)| *until > now);
    for (_, team, pos, _) in w.skill_runtime.advanced.healing.clone() {
        for ally in candidates(w).into_iter().filter(|c| {
            c.team == Some(team)
                && c.target.kind == TargetKind::Player
                && distance(c.pos, pos) < 5.0
        }) {
            heal(w, ally.target.id, 10.0 * t.dt);
        }
    }
    w.skill_runtime
        .advanced
        .anchors
        .retain(|(_, _, until)| *until > now);
    forge_tick(w, now);
    let souls = std::mem::take(&mut w.skill_runtime.advanced.souls);
    for (owner, pos, until) in souls {
        if until <= now {
            continue;
        }
        if let Some(p) = actor_mut(w, owner) {
            if distance([p.hero.x, p.hero.z], pos) <= 3.0 {
                p.hero.skills.advanced.souls = (p.hero.skills.advanced.souls + 1).min(100);
            } else {
                w.skill_runtime.advanced.souls.push((owner, pos, until));
            }
        }
    }
}
pub fn observe(w: &mut GameWorld, events: &[CombatEvent], now: Instant) {
    let mut object_budget = MAX_EFFECTS.saturating_sub(active_count(w) + 1);
    for event in events {
        if event.killed {
            let kind = match event.target.kind {
                CombatEntityKind::Player => 0,
                CombatEntityKind::Minion => 1,
                CombatEntityKind::Structure => 2,
                CombatEntityKind::Neutral => 3,
                _ => continue,
            };
            if !w
                .skill_runtime
                .advanced
                .seen_deaths
                .insert((kind, event.target.id))
            {
                continue;
            }
            if w.skill_runtime.advanced.seen_deaths.len() > MAX_MARKS {
                w.skill_runtime.advanced.seen_deaths.pop_first();
            }
        }
        if event.target.kind == CombatEntityKind::Player && event.amount > 0.0 {
            if let Some(p) = actor_mut(w, event.target.id) {
                p.hero.skills.advanced.last_combat = Some(now);
            }
        }
        if !event.killed {
            continue;
        }
        let pos = [event.x, event.z];
        let participants = w
            .skill_runtime
            .contributions
            .get(&(0, event.target.id))
            .cloned()
            .unwrap_or_default();
        for p in w
            .players
            .values_mut()
            .filter(|p| p.joined && p.hero.hp > 0.0)
        {
            let passive = p.hero.skills.loadout.map(|l| l.passive());
            if passive == Some(PassiveId::Souls)
                && distance([p.hero.x, p.hero.z], pos) < 15.0
                && object_budget > 0
                && w.skill_runtime.advanced.souls.len() < 32
            {
                object_budget -= 1;
                w.skill_runtime.advanced.souls.push((
                    p.hero.identity.id,
                    pos,
                    now + duration(15.0),
                ));
            }
            if event.source.kind == CombatEntityKind::Player
                && (event.source.id == p.hero.identity.id
                    || event.target.kind == CombatEntityKind::Player
                        && participants
                            .get(&p.hero.identity.id)
                            .is_some_and(|at| now.saturating_duration_since(*at) <= duration(3.0)))
            {
                if passive == Some(PassiveId::Essence) {
                    if event.target.kind == CombatEntityKind::Player {
                        p.hero.hp = (p.hero.hp + 40.0).min(p.hero.max_hp);
                    } else {
                        p.hero.skills.advanced.essence += 1;
                        if p.hero.skills.advanced.essence >= 6 {
                            p.hero.skills.advanced.essence = 0;
                            p.hero.hp = (p.hero.hp + 18.0).min(p.hero.max_hp);
                        }
                    }
                }
                if event.target.kind == CombatEntityKind::Player {
                    for (i, r) in p.hero.skills.advanced.recasts.iter_mut().enumerate() {
                        if p.hero.skills.loadout.is_some_and(|l| {
                            matches!(
                                l.skill(SkillSlot::ALL[i]).effect,
                                SkillEffect::Technique {
                                    action: Technique::SpiritDash,
                                    ..
                                }
                            )
                        }) {
                            if let Some(r) = r.as_mut().filter(|r| {
                                r.until > now
                                    && p.hero
                                        .skills
                                        .loadout
                                        .is_some_and(|l| l.skills()[i] == r.skill)
                            }) {
                                r.uses = (r.uses + 1).min(3);
                            }
                        }
                    }
                }
            }
        }
    }
}
pub fn clear_actor(w: &mut GameWorld, id: u64) {
    w.skill_runtime
        .advanced
        .healing
        .retain(|(owner, _, _, _)| *owner != id);
    w.skill_runtime
        .advanced
        .anchors
        .retain(|(owner, _, _)| *owner != id);
    w.skill_runtime.advanced.seen_deaths.remove(&(0, id));
    w.skill_runtime
        .advanced
        .marks
        .retain(|m| m.owner != id && m.target != player_id(id));
    w.skill_runtime.advanced.pillars.retain(|p| p.owner != id);
    w.skill_runtime
        .advanced
        .souls
        .retain(|(owner, _, _)| *owner != id);
}

pub fn interact(w: &mut GameWorld, addr: SocketAddr, object: u64, request: u64, now: Instant) {
    let Some(p) = w.players.get_mut(&addr) else {
        return;
    };
    if !p.joined || request == 0 || request <= p.hero.skills.request_id {
        return;
    }
    p.hero.skills.request_id = request;
    if !matches!(w.game_state, GameState::Running)
        || p.hero.hp <= 0.0
        || p.hero.skills.control.movement(now) == 0.0
    {
        return;
    }
    let owner = p.hero.identity.id;
    let team = p.hero.identity.team;
    let origin = [p.hero.x, p.hero.z];
    let Some(e) = w
        .skill_runtime
        .effects
        .get(&object)
        .filter(|e| {
            e.owner != owner
                && e.team == team
                && e.expires > now
                && distance(e.pos, origin) <= 3.0
                && matches!(
                    skill(e.skill).effect,
                    SkillEffect::Technique {
                        action: Technique::Lantern,
                        ..
                    }
                )
        })
        .cloned()
    else {
        return;
    };
    let Some(destination) = position(w, e.owner).filter(|p| distance(*p, origin) <= 24.0) else {
        return;
    };
    dash_to(w, owner, destination, now);
    w.skill_runtime.effects.remove(&object);
}

fn forge_tick(w: &mut GameWorld, now: Instant) {
    let cs = candidates(w);
    let smiths: Vec<_> = w
        .players
        .values()
        .filter(|p| {
            p.joined
                && p.hero.hp > 0.0
                && p.hero
                    .skills
                    .loadout
                    .is_some_and(|l| l.passive() == PassiveId::Tempered)
        })
        .map(|p| {
            (
                p.hero.identity.id,
                p.hero.identity.team,
                [p.hero.x, p.hero.z],
                p.hero.progress.level,
            )
        })
        .collect();
    for p in w
        .players
        .values_mut()
        .filter(|p| p.joined && p.hero.hp > 0.0)
    {
        let s = &mut p.hero.skills.advanced;
        let pos = [p.hero.x, p.hero.z];
        let safe = s
            .last_combat
            .is_none_or(|at| now.saturating_duration_since(at) >= duration(4.0))
            && !cs.iter().any(|c| {
                c.team.is_some_and(|t| t != p.hero.identity.team) && distance(c.pos, pos) < 10.0
            });
        let own = smiths.iter().any(|(id, _, _, _)| *id == p.hero.identity.id);
        let upgrade = smiths.iter().any(|(_, team, at, level)| {
            *team == p.hero.identity.team && *level >= 6 && distance(*at, pos) < 5.0
        }) && p.hero.progress.level >= 6
            && !p.economy.inventory.is_empty();
        if safe && (own || upgrade) {
            let since = *s.forge_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= duration(3.0) {
                s.forge_ready = own;
                if upgrade {
                    s.forged = true;
                }
            }
        } else {
            s.forge_since = None;
            s.forge_ready = false;
        }
    }
}

// Advanced immobilizations are stuns/airborne/holds. Ordinary roots elsewhere
// retain their existing ability to cast and attack without moving.
fn control(
    w: &mut GameWorld,
    c: Candidate,
    id: u64,
    team: Team,
    root: f32,
    slow: f32,
    secs: f32,
    reveal: f32,
    now: Instant,
) {
    crowd_control::apply(
        w,
        c,
        id,
        team,
        root,
        slow,
        secs,
        reveal,
        now,
        crowd_control::Kind::Stun,
    );
}

pub fn ensure_orb(p: &mut ConnectedPlayer) {
    if p.hero.hp <= 0.0 || p.hero.skills.advanced.orb.is_some() {
        return;
    }
    let Some((slot, id)) = p.hero.skills.loadout.and_then(|l| {
        l.skills().into_iter().enumerate().find(|(_, id)| {
            matches!(
                skill(*id).effect,
                SkillEffect::Technique {
                    action: Technique::BallMove | Technique::BallGuard,
                    ..
                }
            )
        })
    }) else {
        return;
    };
    let pos = [p.hero.x, p.hero.z];
    p.hero.skills.advanced.orb = Some(Orb {
        pos,
        end: pos,
        attached: Some(p.hero.identity.id),
        moving: false,
        hits: BTreeSet::new(),
        skill: id,
        slot: slot as u8,
        scale: 1.0,
    });
}

pub(super) fn consume_brittle(w: &mut GameWorld, c: Candidate, now: Instant) {
    let marks: Vec<_> = w
        .skill_runtime
        .advanced
        .marks
        .iter()
        .filter(|m| m.target == c.target && m.kind == MarkKind::Brittle && m.until > now)
        .cloned()
        .collect();
    w.skill_runtime
        .advanced
        .marks
        .retain(|m| !(m.target == c.target && m.kind == MarkKind::Brittle));
    for m in marks {
        if let Some(team) = actor(w, m.owner).map(|p| p.hero.identity.team) {
            if let Some(event) = raw_damage(
                w,
                c.target,
                c.max_hp * m.amount,
                DamageType::Magic,
                source(m.owner, m.slot),
                team,
                now,
            ) {
                w.skill_runtime.pending.push(event);
            }
        }
    }
}

pub(super) fn visuals(w: &GameWorld, now: Instant) -> Vec<SkillEffectState> {
    let mut out = Vec::new();
    let mut push = |owner: u64,
                    pos: [f32; 2],
                    kind: EffectVisualKind,
                    skill: SkillId,
                    radius: f32,
                    secs: f32| {
        if let Some(p) = actor(w, owner) {
            out.push(SkillEffectState {
                id: u64::MAX - out.len() as u64,
                owner_id: owner,
                owner_team: p.hero.identity.team,
                skill,
                kind,
                position: pos,
                end: pos,
                radius,
                remaining_secs: secs,
                armed: true,
                consumed_segments: 0,
            });
        }
    };
    let mut actors: Vec<_> = w.players.values().collect();
    actors.sort_by_key(|p| p.hero.identity.id);
    for p in actors {
        if let Some(orb) = p
            .hero
            .skills
            .advanced
            .orb
            .as_ref()
            .filter(|_| p.hero.hp > 0.0)
        {
            push(
                p.hero.identity.id,
                orb.pos,
                EffectVisualKind::Orb,
                orb.skill,
                0.65,
                1.0,
            );
        }
    }
    for (id, _, pos, until) in &w.skill_runtime.advanced.healing {
        push(
            *id,
            *pos,
            EffectVisualKind::Healing,
            SkillId::FourfoldDuel,
            5.0,
            remaining(Some(*until), now),
        );
    }
    for (id, pos, until) in &w.skill_runtime.advanced.anchors {
        push(
            *id,
            *pos,
            EffectVisualKind::Anchor,
            SkillId::AnchorStep,
            0.5,
            remaining(Some(*until), now),
        );
    }
    for (id, pos, until) in &w.skill_runtime.advanced.souls {
        push(
            *id,
            *pos,
            EffectVisualKind::Soul,
            SkillId::IronHook,
            0.35,
            remaining(Some(*until), now),
        );
    }
    out
}

pub(super) fn active_count(w: &GameWorld) -> usize {
    w.skill_runtime.effects.len()
        + w.skill_runtime.advanced.souls.len()
        + w.skill_runtime.advanced.anchors.len()
        + w.skill_runtime.advanced.healing.len()
        + w.players
            .values()
            .filter(|p| p.hero.hp > 0.0 && p.hero.skills.advanced.orb.is_some())
            .count()
}

fn legal_landing(w: &GameWorld, pos: [f32; 2], now: Instant) -> bool {
    shared::navigation::world_navigation().point_clear(pos)
        && w.structures.values().filter(|s| s.state.hp > 0.0).all(|s| {
            distance(pos, [s.state.x, s.state.z])
                > crate::world::structure_collision_radius(s.state.kind)
        })
        && terrain(w, now)
            .iter()
            .all(|d| distance(pos, d.center) > d.radius + shared::navigation::HERO_RADIUS)
}
fn blink_to(w: &mut GameWorld, owner: u64, pos: [f32; 2], now: Instant) {
    if let Some(p) = actor_mut(w, owner) {
        p.hero.x = pos[0];
        p.hero.z = pos[1];
        p.hero.utility.dash_sequence = p.hero.utility.dash_sequence.saturating_add(1);
        p.timers.last_movement_at = now;
        p.timers.movement_slack = 0.0;
    }
}
