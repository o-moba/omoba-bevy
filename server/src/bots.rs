//! Practice-only server controllers. Actors use ordinary hero movement, attacks,
//! resources and receipts; internal addresses are never network endpoints.
use crate::*;
use std::collections::VecDeque;

const THINK_INTERVAL: Duration = Duration::from_millis(250);
const ROUTE_INTERVAL: Duration = Duration::from_millis(800);
const VISION: f32 = 15.0;

fn bot_avatar(class: HeroClass, slot: u16) -> Option<&'static str> {
    // Fixed bundled models only; runtime-synced/paid cosmetics are never selected.
    // Each class alternates two free appearances without changing its sprite kit.
    let preferred = match class {
        HeroClass::Warrior => ["good-knight", "bao-samurai"],
        HeroClass::Ranger => ["megan-the-fox", "cyberpal"],
        HeroClass::Mage => ["agnes", "stitch-witch"],
        HeroClass::Cleric => ["anna", "mega-angel"],
    };
    let variant = ((slot.saturating_sub(1) / 4) % 2) as usize;
    [preferred[variant], preferred[1 - variant], "agnes", "anna"]
        .into_iter()
        .find_map(|slug| {
            shared::avatar_definition(slug)
                .filter(|avatar| avatar.passport.is_none())
                .and_then(|_| shared::normalize_avatar_slug(Some(slug)))
        })
}

#[derive(Default)]
pub(crate) struct BotControllers {
    controllers: HashMap<SocketAddr, Controller>,
    defer_fill: bool,
}

impl BotControllers {
    pub(crate) fn clear(&mut self) {
        self.controllers.clear();
    }
}

struct Controller {
    lane: Lane,
    waypoint: usize,
    route: VecDeque<[f32; 2]>,
    goal: Option<[f32; 2]>,
    next_route: Instant,
    next_think: Instant,
    next_retreat: Instant,
    retreat_until: Option<Instant>,
    target: Option<TargetId>,
}

// The unspecified IPv6 source cannot identify a remote UDP peer. Still reject
// this whole namespace explicitly before processing ANY received packet.
pub(crate) fn is_bot_address(addr: SocketAddr) -> bool {
    matches!(addr, SocketAddr::V6(addr) if addr.ip().is_unspecified())
}

fn human_count(players: &HashMap<SocketAddr, ConnectedPlayer>, team: Team) -> u32 {
    players
        .values()
        .filter(|p| p.joined && !p.state.is_bot && p.state.team == team)
        .count() as u32
}

pub(crate) fn assign_human_team(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    team_size: u32,
) -> Option<Team> {
    let green = human_count(players, Team::Green);
    let blue = human_count(players, Team::Blue);
    if green >= team_size && blue >= team_size {
        None
    } else if green <= blue && green < team_size {
        Some(Team::Green)
    } else {
        Some(Team::Blue)
    }
}

pub(crate) fn remove_replaced_bot(
    players: &mut HashMap<SocketAddr, ConnectedPlayer>,
    bots: &mut BotControllers,
    team: Team,
) {
    let addr = players
        .iter()
        .filter(|(_, p)| p.state.is_bot && p.state.team == team)
        .min_by_key(|(_, p)| p.state.id)
        .map(|(addr, _)| *addr);
    if let Some(addr) = addr {
        players.remove(&addr);
        bots.controllers.remove(&addr);
    }
}

impl ServerRuntime {
    pub(crate) fn prepare_practice_join(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        now: Instant,
    ) -> bool {
        if self.match_config.mode != MatchMode::Practice {
            return true;
        }
        let ClientPacket::Join { session_id, .. } = packet else {
            return true;
        };
        if self.players.get(&addr).is_some_and(|p| p.joined) {
            return true;
        }
        let session = normalize_session_id(session_id.clone());
        if !self
            .players
            .get(&addr)
            .is_some_and(|p| p.protocol_compatible)
        {
            return false;
        }
        let retained = session
            .as_ref()
            .and_then(|session| self.disconnected_sessions.get(session));
        let available = retained.map_or_else(
            || assign_human_team(&self.players, self.match_config.team_size).is_some(),
            |p| human_count(&self.players, p.player.state.team) < self.match_config.team_size,
        );
        if !available {
            self.players.get_mut(&addr).unwrap().join_error =
                Some(shared::protocol::JoinRejection::MatchFull);
            return false;
        }
        let retained_id = retained.map(|p| p.player.state.id);
        let roster = self.combat_log.ledger.snapshot();
        let existing = roster.iter().any(|p| {
            Some(p.player_id) == retained_id
                || self
                    .players
                    .get(&addr)
                    .is_some_and(|a| a.state.id == p.player_id)
        });
        if matches!(self.game_state, GameState::Victory { .. })
            || (!existing && roster.len() >= shared::career::MAX_PARTICIPANTS)
        {
            // Admit the new human before refilling. A 16v16 arena otherwise
            // freezes 32 fresh identities and immediately overflows on this join.
            self.bots.defer_fill = true;
            self.restart_round(now);
            self.bots.defer_fill = false;
        }
        true
    }

    pub(crate) fn fill_practice_bots(&mut self, now: Instant) {
        if self.match_config.mode != MatchMode::Practice
            || self.bots.defer_fill
            || matches!(self.game_state, GameState::Victory { .. })
            || !self.players.values().any(|p| p.joined && !p.state.is_bot)
        {
            return;
        }
        // Reclaimed humans keep their own identity and gameplay state. Remove
        // their temporary replacement before another simulation or snapshot.
        for team in [Team::Green, Team::Blue] {
            while self
                .players
                .values()
                .filter(|p| p.joined && p.state.team == team)
                .count()
                > self.match_config.team_size as usize
            {
                let before = self.players.len();
                remove_replaced_bot(&mut self.players, &mut self.bots, team);
                if self.players.len() == before {
                    break;
                }
            }
        }
        let missing = self
            .match_config
            .roster_size()
            .saturating_sub(joined_count(&self.players)) as usize;
        if self.combat_log.ledger.is_started()
            && self.combat_log.ledger.snapshot().len() + missing > shared::career::MAX_PARTICIPANTS
        {
            // Immutable historical identities never get recycled to make room.
            // A fresh practice round retains connected humans and clears combat.
            self.restart_round(now);
            return;
        }
        for team in [Team::Green, Team::Blue] {
            while self
                .players
                .values()
                .filter(|p| p.joined && p.state.team == team)
                .count()
                < self.match_config.team_size as usize
            {
                let slot = (1..=32_u16)
                    .find(|slot| {
                        !self
                            .players
                            .contains_key(&SocketAddr::from(([0_u16; 8], *slot)))
                    })
                    .unwrap();
                let addr = SocketAddr::from(([0_u16; 8], slot));
                ensure_player_connected(
                    &mut self.players,
                    &self.map_layout,
                    addr,
                    &mut self.next_player_id,
                    now,
                );
                let player = self.players.get_mut(&addr).unwrap();
                player.state.is_bot = true;
                let class = [
                    HeroClass::Warrior,
                    HeroClass::Ranger,
                    HeroClass::Mage,
                    HeroClass::Cleric,
                ][(slot as usize - 1) % 4];
                handle_join_request_with_sprite(
                    player,
                    team,
                    default_character_choice(),
                    class,
                    bot_avatar(class, slot),
                    None,
                    &self.map_layout,
                    now,
                );
                self.bots.controllers.insert(
                    addr,
                    Controller {
                        lane: [Lane::Mid, Lane::Top, Lane::Bot][(slot as usize - 1) % 3],
                        waypoint: 1,
                        route: VecDeque::new(),
                        goal: None,
                        next_route: now,
                        next_think: now,
                        next_retreat: now,
                        retreat_until: None,
                        target: None,
                    },
                );
                self.register_career_participant(addr);
            }
        }
    }

    pub(crate) fn simulate_bots(&mut self, now: Instant, dt: f32) {
        if self.match_config.mode != MatchMode::Practice
            || !matches!(self.game_state, GameState::Running)
            || dt <= 0.0
        {
            return;
        }
        let mut addresses: Vec<_> = self.bots.controllers.keys().copied().collect();
        addresses.sort_unstable();
        let discs: Vec<_> = self
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| shared::navigation::Disc {
                center: [s.state.x, s.state.z],
                radius: structure_radius(s.state.kind),
            })
            .collect();
        for addr in addresses {
            if !matches!(self.game_state, GameState::Running) {
                break;
            }
            let Some(mut controller) = self.bots.controllers.remove(&addr) else {
                continue;
            };
            let Some(player) = self.players.get(&addr) else {
                continue;
            };
            if player.state.hp <= 0.0 {
                controller.route.clear();
                controller.goal = None;
                controller.waypoint = 1;
                controller.target = None;
                self.bots.controllers.insert(addr, controller);
                continue;
            }
            let team = player.state.team;
            let origin = [player.state.x, player.state.z];
            let low_health = player.state.hp < player.state.max_hp * 0.28;
            if low_health && now >= controller.next_retreat {
                controller.retreat_until = Some(now + Duration::from_secs(6));
                controller.next_retreat = now + Duration::from_secs(18);
                controller.route.clear();
                controller.next_route = now;
            }
            let retreating = controller.retreat_until.is_some_and(|until| now < until);
            if now >= controller.next_think {
                controller.next_think = now + THINK_INTERVAL;
                apply_skill_upgrade(self.players.get_mut(&addr).unwrap(), 0);
                controller.target = if retreating {
                    None
                } else {
                    self.bot_target(team, origin, controller.lane)
                };
                if let Some(target) = controller.target {
                    handle_cast_request(
                        &mut self.players,
                        &mut self.projectiles,
                        &mut self.minions,
                        &mut self.structures,
                        &mut self.neutrals,
                        &self.team_buffs,
                        addr,
                        target,
                        0,
                        &mut self.next_projectile_id,
                        &self.game_state,
                        now,
                    );
                }
                if low_health {
                    for slot in 0..4 {
                        let p = &self.players[&addr];
                        if ability_for_class_slot(
                            p.state.hero_class,
                            SkillSlot::from_index(slot).unwrap(),
                        )
                        .targeting
                            == TargetingMode::SelfTarget
                        {
                            let target = TargetId {
                                kind: TargetKind::Player,
                                id: p.state.id,
                            };
                            handle_cast_request(
                                &mut self.players,
                                &mut self.projectiles,
                                &mut self.minions,
                                &mut self.structures,
                                &mut self.neutrals,
                                &self.team_buffs,
                                addr,
                                target,
                                slot,
                                &mut self.next_projectile_id,
                                &self.game_state,
                                now,
                            );
                        }
                    }
                }
            }
            let target = controller.target.and_then(|target| {
                basic_attack::resolve_hostile_target(
                    team,
                    target,
                    &self.players,
                    &self.minions,
                    &self.structures,
                    &self.neutrals,
                )
                .map(|(position, radius)| (target, position, radius))
            });
            let reach = shared::basic_attack_for_class(self.players[&addr].state.hero_class).range;
            let mut in_range = false;
            let destination = if retreating {
                let spawn = spawn_position_for_team(&self.map_layout, team);
                [spawn.x, spawn.z]
            } else if let Some((target, position, radius)) = target {
                let dx = position.x - origin[0];
                let dz = position.z - origin[1];
                let distance = dx.hypot(dz);
                in_range = distance <= reach + radius;
                if in_range {
                    self.players.get_mut(&addr).unwrap().state.yaw = dx.atan2(dz);
                    let request = self.players[&addr]
                        .state
                        .basic_attack_request_id
                        .saturating_add(1);
                    basic_attack::handle_basic_attack_request(
                        &mut self.players,
                        &mut self.projectiles,
                        &self.minions,
                        &self.structures,
                        &self.neutrals,
                        &self.team_buffs,
                        addr,
                        target,
                        request,
                        &mut self.next_projectile_id,
                        &self.game_state,
                        now,
                    );
                }
                let approach = (reach + radius - 0.35).max(0.5).min(distance);
                [
                    position.x - dx / distance.max(0.001) * approach,
                    position.z - dz / distance.max(0.001) * approach,
                ]
            } else {
                let path = build_minion_path(&self.map_layout, controller.lane, team);
                while controller.waypoint + 1 < path.len()
                    && (path[controller.waypoint].x - origin[0])
                        .hypot(path[controller.waypoint].z - origin[1])
                        < 1.5
                {
                    controller.waypoint += 1;
                }
                let point = path[controller.waypoint.min(path.len() - 1)];
                [point.x, point.z]
            };
            if !in_range {
                let goal_changed = controller
                    .goal
                    .is_none_or(|p| (p[0] - destination[0]).hypot(p[1] - destination[1]) > 2.0);
                if now >= controller.next_route && (goal_changed || controller.route.is_empty()) {
                    controller.next_route = now + ROUTE_INTERVAL;
                    controller.goal = Some(destination);
                    controller.route = shared::navigation::world_navigation()
                        .plan_route(origin, destination, &discs)
                        .unwrap_or_default()
                        .into();
                }
                while controller
                    .route
                    .front()
                    .is_some_and(|p| (p[0] - origin[0]).hypot(p[1] - origin[1]) < 0.2)
                {
                    controller.route.pop_front();
                }
                if let Some(point) = controller.route.front().copied() {
                    let dx = point[0] - origin[0];
                    let dz = point[1] - origin[1];
                    let distance = dx.hypot(dz);
                    let player = self.players.get_mut(&addr).unwrap();
                    let step =
                        (PLAYER_SPEED * player.state.item_bonuses.move_speed_multiplier * dt)
                            .min(distance);
                    handle_transform_request_with_structures(
                        player,
                        &self.map_layout,
                        &self.structures,
                        origin[0] + dx / distance.max(0.001) * step,
                        PLAYER_GROUND_Y,
                        origin[1] + dz / distance.max(0.001) * step,
                        dx.atan2(dz),
                        now,
                    );
                }
            } else {
                controller.route.clear();
                controller.goal = None;
            }
            self.bots.controllers.insert(addr, controller);
        }
    }

    fn bot_target(&self, team: Team, origin: [f32; 2], lane: Lane) -> Option<TargetId> {
        let distance = |x: f32, z: f32| (x - origin[0]).hypot(z - origin[1]);
        // Fight nearby lane units before diving a structure. Stable ID tie breaks
        // keep behavior independent of HashMap iteration order.
        let mut units: Vec<_> = self
            .players
            .values()
            .filter(|p| p.joined && p.state.hp > 0.0 && p.state.team != team)
            .map(|p| {
                (
                    distance(p.state.x, p.state.z),
                    TargetId {
                        kind: TargetKind::Player,
                        id: p.state.id,
                    },
                )
            })
            .collect();
        units.extend(
            self.minions
                .values()
                .filter(|p| p.state.hp > 0.0 && p.state.team != team)
                .map(|p| {
                    (
                        distance(p.state.x, p.state.z),
                        TargetId {
                            kind: TargetKind::Minion,
                            id: p.state.id,
                        },
                    )
                }),
        );
        if let Some((_, target)) = units
            .into_iter()
            .filter(|(d, _)| *d <= VISION)
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)))
        {
            return Some(target);
        }
        self.structures
            .values()
            .filter(|s| {
                s.state.hp > 0.0
                    && s.state.team != team
                    && !structure_is_protected(&self.structures, s.state.id)
                    && match s.role {
                        StructureRole::BaseTower => true,
                        StructureRole::LaneTower { lane: tower_lane } => tower_lane == lane,
                    }
            })
            .filter(|s| distance(s.state.x, s.state.z) <= VISION + s.attack_range)
            .min_by(|a, b| {
                distance(a.state.x, a.state.z)
                    .total_cmp(&distance(b.state.x, b.state.z))
                    .then_with(|| a.state.id.cmp(&b.state.id))
            })
            .map(|s| TargetId {
                kind: TargetKind::Structure,
                id: s.state.id,
            })
    }
}
