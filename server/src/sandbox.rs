//! Local-only authoritative combat lab, using ordinary actors and hit resolution.
use crate::*;
use shared::sandbox::*;

const ENEMY_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 40001);
const DUMMY_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 40002);

pub(crate) struct SandboxRuntime {
    pub config: SandboxConfig,
    pub now: Instant,
    pub elapsed: f64,
    pub frame: u64,
    step: bool,
    acks: HashMap<u64, SandboxAck>,
    sequences: HashMap<u64, u64>,
    initialized: HashSet<u64>,
}
impl SandboxRuntime {
    pub fn new(now: Instant) -> Self {
        Self {
            config: Default::default(),
            now,
            elapsed: 0.0,
            frame: 0,
            step: false,
            acks: HashMap::new(),
            sequences: HashMap::new(),
            initialized: HashSet::new(),
        }
    }
    pub fn advance(&mut self, wall_dt: f32) -> (Instant, f32) {
        let dt = if self.step {
            1.0 / 60.0
        } else if self.config.environment.paused {
            0.0
        } else {
            wall_dt * self.config.environment.time_scale
        };
        self.step = false;
        self.now += Duration::from_secs_f32(dt);
        self.elapsed += dt as f64;
        if dt > 0.0 {
            self.frame += 1;
        }
        (self.now, dt)
    }
    pub fn snapshot(
        &self,
        requester: SocketAddr,
        players: &HashMap<SocketAddr, ConnectedPlayer>,
        log: &CombatLog,
    ) -> SandboxSnapshot {
        let actors = [
            (SandboxActor::Player, requester),
            (SandboxActor::Enemy, ENEMY_ADDR),
            (SandboxActor::Dummy, DUMMY_ADDR),
        ]
        .into_iter()
        .filter_map(|(actor, addr)| {
            let p = players.get(&addr)?;
            let c = p.sandbox.as_ref()?;
            Some(ActorTelemetry {
                actor,
                id: p.state.id,
                position: [p.state.x, p.state.z],
                hp: p.state.hp,
                mana: p.state.mana,
                armor: c.armor,
                resistance: c.resistance,
                move_speed: PLAYER_SPEED
                    * p.speed_mult
                    * p.state.item_bonuses.move_speed_multiplier,
                attack_speed: p.state.item_bonuses.attack_speed_multiplier,
                attack_damage: effective_basic_attack_damage(p),
                cooldowns: std::array::from_fn(|i| {
                    let d = effective_ability_cooldown(p, SkillSlot::from_index(i as u8).unwrap());
                    if c.no_cooldowns {
                        0.0
                    } else {
                        p.last_cast_at[i]
                            .map(|at| {
                                d.saturating_sub(self.now.saturating_duration_since(at))
                                    .as_secs_f32()
                            })
                            .unwrap_or(0.0)
                    }
                }),
                unlocked: if c.unlock_all {
                    [true; 4]
                } else {
                    unlocked_slots_for_level(p.state.level)
                },
            })
        })
        .collect();
        let mut config = self.config.clone();
        if let Some(c) = players.get(&requester).and_then(|p| p.sandbox.as_ref()) {
            config.player = c.clone();
        }
        SandboxSnapshot {
            config,
            ack: players
                .get(&requester)
                .and_then(|p| self.acks.get(&p.state.id))
                .cloned(),
            last_request_id: players
                .get(&requester)
                .and_then(|p| self.sequences.get(&p.state.id))
                .copied()
                .unwrap_or(0),
            actors,
            analytics: log.sandbox_analytics(self.now),
            simulation_secs: self.elapsed,
            frame: self.frame,
        }
    }
}
fn bounded(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}
fn validate_position(position: [f32; 2]) -> Result<(), String> {
    if !position.iter().all(|v| bounded(*v, -500.0, 500.0)) {
        return Err("Position must be finite and inside the map".into());
    }
    if !shared::navigation::world_navigation().point_clear(position) {
        return Err("Position is not on walkable terrain".into());
    }
    Ok(())
}
fn validate_actor(c: &ActorConfig) -> Result<(), String> {
    if c.avatar
        .as_ref()
        .is_some_and(|a| shared::avatar_definition(a).is_none_or(|d| d.passport.is_some()))
    {
        return Err("Avatar must be a shipped free roster model".into());
    }
    if !(1..=MAX_LEVEL).contains(&c.level) || c.xp >= xp_threshold_for_level(c.level).max(1) {
        return Err("Level must be 1..10; XP must fit the current level".into());
    }
    for (i, r) in c.ranks.iter().enumerate() {
        if !(1..=c.hero.abilities()[i].max_rank).contains(r) {
            return Err("Skill rank outside ability bounds".into());
        }
    }
    if !bounded(c.max_hp, 1.0, 1_000_000.0)
        || !bounded(c.armor, 0.0, 10000.0)
        || !bounded(c.resistance, 0.0, 10000.0)
        || !bounded(c.move_speed, 0.1, 10.0)
        || !bounded(c.attack_speed, 0.1, 10.0)
        || !bounded(c.damage_multiplier, 0.0, 100.0)
    {
        return Err("Combat statistic outside finite supported bounds".into());
    }
    if c.inventory.len() > shared::shop::INVENTORY_CAPACITY
        || c.inventory.iter().collect::<HashSet<_>>().len() != c.inventory.len()
    {
        return Err("Inventory allows six distinct catalog items".into());
    }
    validate_position(c.position)
}
fn validate_config(c: &SandboxConfig) -> Result<(), String> {
    if c.version != PRESET_VERSION {
        return Err("Unsupported sandbox preset version".into());
    }
    validate_actor(&c.player)?;
    validate_actor(&c.enemy.actor)?;
    validate_position(c.dummy.position)?;
    if !bounded(c.dummy.max_hp, 1.0, 1_000_000.0)
        || !bounded(c.dummy.armor, 0.0, 10000.0)
        || !bounded(c.dummy.resistance, 0.0, 10000.0)
        || !bounded(c.enemy.aggression_range, 0.0, 100.0)
        || !bounded(c.enemy.attack_distance, 0.1, 100.0)
        || !TIME_SCALES.contains(&c.environment.time_scale)
    {
        return Err("Invalid dummy, AI distance, or time scale".into());
    }
    Ok(())
}
fn clear_cooldowns(p: &mut ConnectedPlayer) {
    p.last_cast_at = [None; 4];
    p.last_basic_attack_at = None;
    p.dash_ready_at = None;
    p.haste_ready_at = None;
    p.state.utility.dash_remaining_secs = 0.0;
    p.state.utility.haste_remaining_secs = 0.0;
    p.state.basic_attack_remaining_secs = 0.0;
}
pub(crate) fn apply_actor(p: &mut ConnectedPlayer, c: &ActorConfig, reset: bool, now: Instant) {
    let changed_hero = p.state.hero_class != c.hero;
    let changed_avatar = p.sandbox.as_ref().is_some_and(|old| old.avatar != c.avatar);
    let moved = p
        .sandbox
        .as_ref()
        .is_some_and(|old| old.position != c.position);
    p.sandbox = Some(c.clone());
    p.god_mode = c.god_mode;
    p.speed_mult = c.move_speed;
    p.state.hero_class = c.hero;
    if changed_hero || changed_avatar || c.avatar.is_some() {
        p.state.avatar = c
            .avatar
            .clone()
            .or_else(|| bots::bot_avatar(c.hero, 1).map(str::to_owned));
        p.state.sprite_character = None;
    }
    if moved {
        p.state.utility.dash_sequence = p.state.utility.dash_sequence.saturating_add(1);
        p.state.x = c.position[0];
        p.state.z = c.position[1];
        p.last_movement_at = now;
    }
    p.state.level = c.level;
    p.state.xp = c.xp;
    p.state.next_level_xp = xp_threshold_for_level(c.level);
    p.state.ranks = c.ranks;
    p.state.skill_points = c.level.saturating_sub(1);
    p.state.inventory = c.inventory.clone();
    let mut bonuses = shared::shop::item_bonuses(&c.inventory);
    bonuses.damage_multiplier *= c.damage_multiplier;
    bonuses.attack_speed_multiplier *= c.attack_speed;
    p.state.item_bonuses = bonuses;
    p.state.max_hp = c.max_hp + (c.level - 1) as f32 * LEVEL_UP_HP_BONUS + bonuses.max_hp;
    p.state.max_mana = MAX_MANA + (c.level - 1) as f32 * LEVEL_UP_MANA_BONUS + bonuses.max_mana;
    p.state.hp = p.state.hp.min(p.state.max_hp);
    p.state.mana = p.state.mana.min(p.state.max_mana);
    if reset {
        p.state.utility.dash_sequence = p.state.utility.dash_sequence.saturating_add(1);
        p.state.x = c.position[0];
        p.state.z = c.position[1];
        p.state.y = PLAYER_GROUND_Y;
        p.state.hp = p.state.max_hp;
        p.state.mana = p.state.max_mana;
        p.respawn_at = None;
        p.haste_expires_at = None;
        p.state.utility.haste_active_secs = 0.0;
        p.last_movement_at = now;
        clear_cooldowns(p);
        p.state.action_kind = PlayerActionKind::None;
    }
    if c.no_cooldowns {
        clear_cooldowns(p);
    }
}
impl ServerRuntime {
    pub(crate) fn sandbox_allowed(&self) -> bool {
        self.sandbox.is_some()
            && self.match_config.mode == MatchMode::Dev
            && !self.match_service.is_public()
            && self.match_service.worker().is_none()
    }
    pub(crate) fn initialize_sandbox_players(&mut self) {
        if !self.sandbox_allowed() {
            return;
        }
        let s = self.sandbox.as_mut().unwrap();
        for p in self
            .players
            .values_mut()
            .filter(|p| p.joined && !p.state.is_bot)
        {
            if s.initialized.insert(p.state.id) || p.sandbox.is_none() {
                let mut c = s.config.player.clone();
                c.hero = p.state.hero_class;
                c.avatar = p.state.avatar.clone();
                if p.state.team == Team::Blue {
                    c.position = [3.0, 0.0];
                }
                apply_actor(p, &c, true, s.now);
            }
        }
        self.combat_log.enable_sandbox(s.now);
    }
    fn sandbox_addr(
        &self,
        requester: SocketAddr,
        actor: SandboxActor,
    ) -> Result<SocketAddr, String> {
        let addr = match actor {
            SandboxActor::Player => requester,
            SandboxActor::Enemy => ENEMY_ADDR,
            SandboxActor::Dummy => DUMMY_ADDR,
        };
        self.players
            .get(&addr)
            .filter(|p| p.joined)
            .map(|_| addr)
            .ok_or_else(|| "Selected actor is not spawned".into())
    }
    pub(crate) fn handle_sandbox(&mut self, addr: SocketAddr, request: SandboxRequest) {
        if !self.sandbox_allowed() {
            return;
        }
        let Some(player) = self
            .players
            .get(&addr)
            .filter(|p| p.joined && !p.state.is_bot && p.protocol_compatible)
        else {
            return;
        };
        let id = player.state.id;
        let s = self.sandbox.as_mut().unwrap();
        // UDP retries must repeat the original acknowledgement, never convert
        // a successful non-idempotent command into a false rejection or rerun it.
        if request.server_epoch == self.server_epoch
            && request.match_id == self.match_id
            && request.request_id > 0
            && request.request_id <= *s.sequences.get(&id).unwrap_or(&0)
        {
            return;
        }
        let result =
            if request.server_epoch != self.server_epoch || request.match_id != self.match_id {
                Err("Stale server epoch or match".into())
            } else if request.request_id == 0
                || request.request_id <= *s.sequences.get(&id).unwrap_or(&0)
            {
                Err("Duplicate or stale request".into())
            } else {
                s.sequences.insert(id, request.request_id);
                self.apply_sandbox_command(addr, request.command)
            };
        let accepted = result.is_ok();
        self.sandbox.as_mut().unwrap().acks.insert(
            id,
            SandboxAck {
                request_id: request.request_id,
                accepted,
                message: result.err().unwrap_or_else(|| "Applied".into()),
            },
        );
    }
    fn apply_sandbox_command(
        &mut self,
        addr: SocketAddr,
        command: SandboxCommand,
    ) -> Result<(), String> {
        let now = self.sandbox.as_ref().unwrap().now;
        match command {
            SandboxCommand::ApplyConfig { config } => {
                validate_config(&config)?;
                for position in [
                    config.player.position,
                    config.enemy.actor.position,
                    config.dummy.position,
                ] {
                    self.validate_sandbox_destination(position)?;
                }
                let old = self.sandbox.as_ref().unwrap().config.clone();
                let p = self.players.get_mut(&addr).unwrap();
                let reset = p.state.hero_class != config.player.hero;
                let id = p.state.id;
                let moved = p
                    .sandbox
                    .as_ref()
                    .is_some_and(|c| c.position != config.player.position);
                apply_actor(p, &config.player, reset, now);
                if reset || moved {
                    self.projectiles.retain(|_, p| {
                        p.state.owner_id != id
                            && p.target
                                != TargetId {
                                    kind: TargetKind::Player,
                                    id,
                                }
                    });
                }
                self.sandbox.as_mut().unwrap().config = config.clone();
                self.sync_sandbox_actor(
                    ENEMY_ADDR,
                    config.enemy.enabled,
                    &config.enemy.actor,
                    old.enemy.actor.hero != config.enemy.actor.hero,
                    now,
                );
                let c = dummy_actor(&config.dummy);
                self.sync_sandbox_actor(DUMMY_ADDR, config.dummy.enabled, &c, false, now);
                if let Some(p) = self.players.get_mut(&DUMMY_ADDR) {
                    p.sandbox_infinite_hp = config.dummy.infinite_hp;
                }
                if !config.environment.minions {
                    self.minions.clear();
                    self.projectiles.retain(|_, p| {
                        p.state.source_kind != CombatEntityKind::Minion
                            && p.target.kind != TargetKind::Minion
                    });
                }
                if old.environment.minions != config.environment.minions
                    || old.environment.minions_paused != config.environment.minions_paused
                {
                    self.last_wave_spawn_at = now;
                }
            }
            SandboxCommand::Refill { actor } => {
                let a = self.sandbox_addr(addr, actor)?;
                let p = self.players.get_mut(&a).unwrap();
                p.state.hp = p.state.max_hp;
                p.state.mana = p.state.max_mana;
                p.respawn_at = None;
            }
            SandboxCommand::ResetCooldowns { actor } => {
                let a = self.sandbox_addr(addr, actor)?;
                clear_cooldowns(self.players.get_mut(&a).unwrap());
            }
            SandboxCommand::Teleport { actor, position } => {
                self.validate_sandbox_destination(position)?;
                let a = self.sandbox_addr(addr, actor)?;
                let p = self.players.get_mut(&a).unwrap();
                p.state.utility.dash_sequence = p.state.utility.dash_sequence.saturating_add(1);
                p.state.x = position[0];
                p.state.z = position[1];
                p.last_movement_at = now;
                let id = p.state.id;
                self.projectiles.retain(|_, p| {
                    p.state.owner_id != id
                        && p.target
                            != TargetId {
                                kind: TargetKind::Player,
                                id,
                            }
                });
            }
            SandboxCommand::ResetActor { actor } => {
                let a = self.sandbox_addr(addr, actor)?;
                self.reset_sandbox_actor(a, now);
            }
            SandboxCommand::ResetDuel => self.reset_sandbox_duel(now),
            SandboxCommand::AddXp { actor, amount } => {
                let a = self.sandbox_addr(addr, actor)?;
                let p = &self.players[&a];
                let mut c = p.sandbox.clone().ok_or("Actor has no sandbox config")?;
                let total: u32 = (1..c.level).map(xp_threshold_for_level).sum::<u32>() + c.xp;
                let mut left = (total as i64 + amount as i64).max(0).min(u32::MAX as i64) as u32;
                c.level = 1;
                while c.level < MAX_LEVEL && left >= xp_threshold_for_level(c.level) {
                    left -= xp_threshold_for_level(c.level);
                    c.level += 1;
                }
                c.xp = if c.level == MAX_LEVEL { 0 } else { left };
                apply_actor(self.players.get_mut(&a).unwrap(), &c, false, now);
                self.save_actor_config(actor, c);
            }
            SandboxCommand::GrantItem { actor, item } => {
                let a = self.sandbox_addr(addr, actor)?;
                let mut c = self.players[&a]
                    .sandbox
                    .clone()
                    .ok_or("Actor has no sandbox config")?;
                c.inventory.push(item);
                validate_actor(&c)?;
                apply_actor(self.players.get_mut(&a).unwrap(), &c, false, now);
                self.save_actor_config(actor, c);
            }
            SandboxCommand::ResetAnalytics => self.combat_log.reset_sandbox(now),
            SandboxCommand::FrameStep => {
                let s = self.sandbox.as_mut().unwrap();
                if !s.config.environment.paused {
                    return Err("Pause simulation before stepping".into());
                }
                s.step = true;
            }
            SandboxCommand::SpawnWave => {
                if !self.sandbox.as_ref().unwrap().config.environment.minions {
                    return Err("Enable minions before spawning a wave".into());
                }
                for team in [Team::Green, Team::Blue] {
                    for lane in [Lane::Top, Lane::Mid, Lane::Bot] {
                        spawn_minion_wave_for_team_lane(
                            &self.map_layout,
                            &mut self.minions,
                            &mut self.next_minion_id,
                            team,
                            lane,
                        );
                    }
                }
                self.last_wave_spawn_at = now;
            }
            SandboxCommand::ForceCast {
                actor,
                slot,
                target_id,
            } => {
                let a = self.sandbox_addr(addr, actor)?;
                self.sandbox_cast(a, slot, target_id, now)?;
            }
        }
        Ok(())
    }
    pub(crate) fn reset_sandbox_duel(&mut self, now: Instant) {
        let addresses: Vec<_> = self
            .players
            .iter()
            .filter(|(_, p)| p.joined)
            .map(|(a, _)| *a)
            .collect();
        for a in addresses {
            self.reset_sandbox_actor(a, now);
        }
        self.projectiles.clear();
        self.minions.clear();
        self.structures = build_configured_structures(&self.map_config);
        let mut next_neutral_id = 9001;
        self.neutrals = build_neutral_camps(&mut next_neutral_id);
        self.neutrals
            .extend(build_boss_neutrals(&mut next_neutral_id));
        schedule_boss_spawns(&mut self.neutrals, now);
        self.team_buffs = TeamBuffs::default();
        self.combat_log.reset_sandbox(now);
        self.game_state = GameState::Running;
        self.victory_at = None;
        self.last_wave_spawn_at = now;
    }
    fn validate_sandbox_destination(&self, position: [f32; 2]) -> Result<(), String> {
        validate_position(position)?;
        if self.structures.values().any(|s| {
            s.state.hp > 0.0
                && (s.state.x - position[0]).hypot(s.state.z - position[1])
                    < structure_collision_radius(s.state.kind) + shared::navigation::HERO_RADIUS
        }) {
            return Err("Destination overlaps a living structure".into());
        }
        Ok(())
    }
    fn save_actor_config(&mut self, actor: SandboxActor, c: ActorConfig) {
        let s = self.sandbox.as_mut().unwrap();
        match actor {
            SandboxActor::Player => s.config.player = c,
            SandboxActor::Enemy => s.config.enemy.actor = c,
            SandboxActor::Dummy => {}
        }
    }
    fn sync_sandbox_actor(
        &mut self,
        addr: SocketAddr,
        enabled: bool,
        c: &ActorConfig,
        reset: bool,
        now: Instant,
    ) {
        if !enabled {
            if let Some(p) = self.players.remove(&addr) {
                self.projectiles.retain(|_, b| {
                    b.state.owner_id != p.state.id
                        && b.target
                            != TargetId {
                                kind: TargetKind::Player,
                                id: p.state.id,
                            }
                });
            }
            return;
        }
        let new = !self.players.contains_key(&addr);
        if new {
            ensure_player_connected(
                &mut self.players,
                &self.map_layout,
                addr,
                &mut self.next_player_id,
                now,
            );
            let p = self.players.get_mut(&addr).unwrap();
            p.state.is_bot = true;
            handle_join_request_with_sprite(
                p,
                Team::Blue,
                default_character_choice(),
                c.hero,
                bots::bot_avatar(c.hero, 1),
                None,
                &self.map_layout,
                now,
            );
        }
        if reset {
            let id = self.players[&addr].state.id;
            self.projectiles.retain(|_, p| {
                p.state.owner_id != id
                    && p.target
                        != TargetId {
                            kind: TargetKind::Player,
                            id,
                        }
            });
        }
        apply_actor(self.players.get_mut(&addr).unwrap(), c, new || reset, now);
    }
    fn reset_sandbox_actor(&mut self, addr: SocketAddr, now: Instant) {
        if let Some(p) = self.players.get_mut(&addr) {
            if let Some(c) = p.sandbox.clone() {
                let id = p.state.id;
                apply_actor(p, &c, true, now);
                self.projectiles.retain(|_, p| {
                    p.state.owner_id != id
                        && p.target
                            != TargetId {
                                kind: TargetKind::Player,
                                id,
                            }
                });
            }
        }
    }
    fn sandbox_cast(
        &mut self,
        addr: SocketAddr,
        slot: u8,
        target_id: Option<u64>,
        now: Instant,
    ) -> Result<(), String> {
        let skill = SkillSlot::from_index(slot).ok_or("Skill slot must be 0..3")?;
        let p = &self.players[&addr];
        let target = if ability_for_class_slot(p.state.hero_class, skill).targeting
            == TargetingMode::SelfTarget
        {
            TargetId {
                kind: TargetKind::Player,
                id: p.state.id,
            }
        } else {
            self.players
                .values()
                .filter(|t| {
                    t.joined
                        && t.state.hp > 0.0
                        && t.state.team != p.state.team
                        && target_id.is_none_or(|id| id == t.state.id)
                })
                .min_by(|a, b| {
                    ((a.state.x - p.state.x).hypot(a.state.z - p.state.z))
                        .total_cmp(&((b.state.x - p.state.x).hypot(b.state.z - p.state.z)))
                })
                .map(|t| TargetId {
                    kind: TargetKind::Player,
                    id: t.state.id,
                })
                .ok_or("No living hostile target")?
        };
        let before = p.state.action_sequence;
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
        if self.players[&addr].state.action_sequence == before {
            Err("Ability rejected: check range, unlock, health, mana and cooldown".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn simulate_sandbox(&mut self, now: Instant, dt: f32) {
        if !self.sandbox_allowed() {
            return;
        }
        self.initialize_sandbox_players();
        let config = self.sandbox.as_ref().unwrap().config.clone();
        for p in self.players.values_mut() {
            if let Some(c) = &p.sandbox {
                if c.infinite_resource {
                    p.state.mana = p.state.max_mana;
                }
                if c.no_cooldowns {
                    clear_cooldowns(p);
                }
            }
        }
        if dt <= 0.0 {
            return;
        }
        for addr in [ENEMY_ADDR, DUMMY_ADDR] {
            let Some(p) = self.players.get(&addr) else {
                continue;
            };
            if p.state.hp <= 0.0 {
                if addr == DUMMY_ADDR || config.enemy.auto_respawn {
                    if p.respawn_at.is_some_and(|t| now >= t) {
                        self.reset_sandbox_actor(addr, now);
                    } else if p.respawn_at.is_none() {
                        self.players.get_mut(&addr).unwrap().respawn_at = Some(now + RESPAWN_DELAY);
                    }
                } else {
                    self.players.get_mut(&addr).unwrap().respawn_at = None;
                }
                continue;
            }
            let origin = [p.state.x, p.state.z];
            let team = p.state.team;
            let target = self
                .players
                .values()
                .filter(|p| p.joined && !p.state.is_bot && p.state.hp > 0.0 && p.state.team != team)
                .min_by_key(|p| p.state.id)
                .map(|p| (p.state.id, [p.state.x, p.state.z]));
            if addr == DUMMY_ADDR {
                if config.dummy.moving {
                    let phase = self.sandbox.as_ref().unwrap().elapsed as f32;
                    self.move_sandbox_actor(
                        addr,
                        [
                            config.dummy.position[0] + phase.sin() * 3.0,
                            config.dummy.position[1],
                        ],
                        now,
                        dt,
                    );
                }
                continue;
            }
            let Some((id, position)) = target else {
                continue;
            };
            let distance = (position[0] - origin[0]).hypot(position[1] - origin[1]);
            if distance > config.enemy.aggression_range {
                continue;
            }
            match config.enemy.behavior {
                BotBehavior::Stationary => {}
                BotBehavior::Flee => self.move_sandbox_actor(
                    addr,
                    [
                        origin[0] + (origin[0] - position[0]) * 2.0,
                        origin[1] + (origin[1] - position[1]) * 2.0,
                    ],
                    now,
                    dt,
                ),
                BotBehavior::Attack | BotBehavior::Fight => {
                    let reach =
                        shared::basic_attack_for_class(self.players[&addr].state.hero_class).range
                            + PLAYER_HIT_RADIUS;
                    let desired = config.enemy.attack_distance.min(reach - 0.1);
                    if distance > desired {
                        self.move_sandbox_actor(addr, position, now, dt);
                    }
                    if distance <= reach {
                        let request = self.players[&addr]
                            .state
                            .basic_attack_request_id
                            .saturating_add(1);
                        handle_basic_attack_request(
                            &mut self.players,
                            &mut self.projectiles,
                            &self.minions,
                            &self.structures,
                            &self.neutrals,
                            &self.team_buffs,
                            addr,
                            TargetId {
                                kind: TargetKind::Player,
                                id,
                            },
                            request,
                            &mut self.next_projectile_id,
                            &self.game_state,
                            now,
                        );
                    }
                    if config.enemy.behavior == BotBehavior::Fight {
                        for slot in 0..4 {
                            let _ = self.sandbox_cast(addr, slot, None, now);
                        }
                    }
                }
            }
        }
    }
    fn move_sandbox_actor(&mut self, addr: SocketAddr, goal: [f32; 2], now: Instant, dt: f32) {
        let p = &self.players[&addr];
        let origin = [p.state.x, p.state.z];
        let discs: Vec<_> = self
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| shared::navigation::Disc {
                center: [s.state.x, s.state.z],
                radius: structure_collision_radius(s.state.kind),
            })
            .collect();
        let route = shared::navigation::world_navigation()
            .plan_route(origin, goal, &discs)
            .unwrap_or_default();
        let Some(next) = route
            .into_iter()
            .find(|x| (x[0] - origin[0]).hypot(x[1] - origin[1]) > 0.1)
        else {
            return;
        };
        let dx = next[0] - origin[0];
        let dz = next[1] - origin[1];
        let distance = dx.hypot(dz).max(0.001);
        let step = (PLAYER_SPEED * p.speed_mult * p.state.item_bonuses.move_speed_multiplier * dt)
            .min(distance);
        handle_transform_request_with_structures(
            self.players.get_mut(&addr).unwrap(),
            &self.map_layout,
            &self.structures,
            origin[0] + dx / distance * step,
            PLAYER_GROUND_Y,
            origin[1] + dz / distance * step,
            dx.atan2(dz),
            now,
        );
    }
}
fn dummy_actor(c: &DummyConfig) -> ActorConfig {
    ActorConfig {
        max_hp: c.max_hp,
        armor: c.armor,
        resistance: c.resistance,
        position: c.position,
        ..Default::default()
    }
}

/// Ordinary item helpers deliberately floor beneficial item multipliers at one.
/// Sandbox values are validated independently and intentionally support zero
/// damage and slower attacks. Keep that exception local to sandbox actors.
pub(crate) fn effective_basic_attack_damage(player: &ConnectedPlayer) -> f32 {
    let definition = shared::basic_attack_for_class(player.state.hero_class);
    if player.sandbox.is_some() {
        definition.damage * player.state.item_bonuses.damage_multiplier.max(0.0)
    } else {
        shared::shop::basic_attack_damage(definition, player.state.item_bonuses)
    }
}

pub(crate) fn effective_basic_attack_cooldown(player: &ConnectedPlayer) -> Duration {
    let definition = shared::basic_attack_for_class(player.state.hero_class);
    if player.sandbox.is_some() {
        Duration::from_secs_f32(definition.cooldown_secs)
            .div_f32(player.state.item_bonuses.attack_speed_multiplier.max(0.1))
    } else {
        shared::shop::basic_attack_cooldown(definition, player.state.item_bonuses)
    }
}

pub(crate) fn effective_ability_cooldown(player: &ConnectedPlayer, slot: SkillSlot) -> Duration {
    let definition = ability_for_class_slot(player.state.hero_class, slot);
    let rank = player.state.ranks[slot.index()].clamp(1, definition.max_rank);
    if player.sandbox.is_some() && slot == SkillSlot::Q {
        shared::scaled_cooldown(definition, rank)
            .div_f32(player.state.item_bonuses.attack_speed_multiplier.max(0.1))
    } else {
        shared::shop::item_cooldown(definition, rank, slot, player.state.item_bonuses)
    }
}

#[cfg(test)]
mod tests;

/// Two human seats, assigned automatically regardless of the client's preferred
/// side. Internal training actors never consume a human seat.
pub(crate) fn assign_human_team(
    players: &HashMap<SocketAddr, ConnectedPlayer>,
    disconnected: &HashMap<String, DisconnectedSession>,
) -> Option<Team> {
    let mut green = 0;
    let mut blue = 0;
    for p in players
        .values()
        .chain(disconnected.values().map(|s| &s.player))
        .filter(|p| p.joined && !p.state.is_bot)
    {
        match p.state.team {
            Team::Green => green += 1,
            Team::Blue => blue += 1,
        }
    }
    if green == 0 {
        Some(Team::Green)
    } else if blue == 0 {
        Some(Team::Blue)
    } else {
        None
    }
}
