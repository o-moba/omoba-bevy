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
    /// The `ActorConfig` last applied to each hero (by hero id), kept only to
    /// echo it back in the telemetry and to seed the next edit or reset; the
    /// simulation reads `ConnectedPlayer.modifiers` and the hero's loadout.
    actors: HashMap<u64, ActorConfig>,
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
            actors: HashMap::new(),
        }
    }
    /// The actor's configuration as the telemetry echoes it: the applied
    /// config with the values the hero has owned since (ranks, inventory,
    /// the god-mode and speed toggles) read back from the hero.
    pub(crate) fn actor_config(&self, p: &ConnectedPlayer) -> Option<ActorConfig> {
        let mut c = self.actors.get(&p.hero.identity.id)?.clone();
        c.ranks = p.hero.progress.ranks;
        c.inventory = p.economy.inventory.clone();
        c.god_mode = p.modifiers.god_mode;
        c.move_speed = p.modifiers.move_speed_mult;
        Some(c)
    }
    /// Converts `c` into the hero's `StatModifiers` and loadout (class,
    /// avatar, level, XP, ranks, inventory, pools) once; `reset` also puts
    /// the actor back on its configured spot with full pools and clear
    /// clocks. The config itself is only remembered for the telemetry.
    pub(crate) fn apply_actor(
        &mut self,
        p: &mut ConnectedPlayer,
        c: &ActorConfig,
        reset: bool,
        now: Instant,
    ) {
        let old = self.actors.get(&p.hero.identity.id);
        let changed_hero = p.hero.identity.hero_class != c.hero;
        let changed_avatar = old.is_some_and(|old| old.avatar != c.avatar);
        let moved = old.is_some_and(|old| old.position != c.position);
        self.actors.insert(p.hero.identity.id, c.clone());
        p.modifiers = StatModifiers {
            damage_mult: c.damage_multiplier,
            attack_speed_mult: c.attack_speed,
            move_speed_mult: c.move_speed,
            armor: c.armor,
            resistance: c.resistance,
            base_max_hp: Some(c.max_hp),
            god_mode: c.god_mode,
            // The dummy's infinite HP is a `DummyConfig` setting, applied
            // after the actor sync.
            infinite_hp: p.modifiers.infinite_hp,
            infinite_resource: c.infinite_resource,
            no_cooldowns: c.no_cooldowns,
            unlock_all: c.unlock_all,
            bypass_vision: true,
            grant_xp: false,
            respawns: !p.hero.identity.is_bot,
        };
        p.hero.identity.hero_class = c.hero;
        if changed_hero || changed_avatar || c.avatar.is_some() {
            p.hero.identity.avatar = c
                .avatar
                .clone()
                .or_else(|| bots::bot_avatar(c.hero, 1).map(str::to_owned));
            p.hero.identity.sprite_character = None;
        }
        if moved {
            p.hero.utility.dash_sequence = p.hero.utility.dash_sequence.saturating_add(1);
            p.hero.x = c.position[0];
            p.hero.z = c.position[1];
            p.timers.last_movement_at = now;
        }
        p.hero.progress.level = c.level;
        p.hero.progress.xp = c.xp;
        p.hero.progress.next_level_xp = xp_threshold_for_level(c.level);
        p.hero.progress.ranks = c.ranks;
        p.hero.progress.skill_points = c.level.saturating_sub(1);
        p.economy.inventory = c.inventory.clone();
        p.economy.item_bonuses = shared::shop::item_bonuses(&c.inventory);
        hero_stats::resize_pools(p);
        if reset {
            p.hero.utility.dash_sequence = p.hero.utility.dash_sequence.saturating_add(1);
            p.hero.x = c.position[0];
            p.hero.z = c.position[1];
            p.hero.y = PLAYER_GROUND_Y;
            p.hero.hp = p.hero.max_hp;
            p.hero.mana = p.hero.max_mana;
            p.timers.respawn_at = None;
            p.timers.haste_expires_at = None;
            p.timers.last_movement_at = now;
            p.timers.clear_cooldowns();
            p.hero.last_action.kind = PlayerActionKind::None;
        }
        if c.no_cooldowns {
            p.timers.clear_cooldowns();
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
            if !self.actors.contains_key(&p.hero.identity.id) {
                return None;
            }
            Some(ActorTelemetry {
                actor,
                id: p.hero.identity.id,
                position: [p.hero.x, p.hero.z],
                hp: p.hero.hp,
                mana: p.hero.mana,
                armor: p.modifiers.armor,
                resistance: p.modifiers.resistance,
                move_speed: hero_stats::move_speed(p),
                attack_speed: hero_stats::attack_speed(p),
                attack_damage: hero_stats::basic_attack_damage(p),
                cooldowns: std::array::from_fn(|i| {
                    if p.modifiers.no_cooldowns {
                        0.0
                    } else {
                        hero_timers::skill_cooldown_left(p, SkillSlot::ALL[i], self.now)
                    }
                }),
                unlocked: if p.modifiers.unlock_all {
                    [true; 4]
                } else {
                    unlocked_slots_for_level(p.hero.progress.level)
                },
            })
        })
        .collect();
        let mut config = self.config.clone();
        if let Some(c) = players.get(&requester).and_then(|p| self.actor_config(p)) {
            config.player = c;
        }
        SandboxSnapshot {
            config,
            ack: players
                .get(&requester)
                .and_then(|p| self.acks.get(&p.hero.identity.id))
                .cloned(),
            last_request_id: players
                .get(&requester)
                .and_then(|p| self.sequences.get(&p.hero.identity.id))
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
impl ServerRuntime {
    pub(crate) fn sandbox_allowed(&self) -> bool {
        self.sandbox.is_some()
            && self.rules.combat_sandbox_allowed
            && !self.match_service.is_public()
            && self.match_service.worker().is_none()
    }
    pub(crate) fn initialize_sandbox_players(&mut self) {
        if !self.sandbox_allowed() {
            return;
        }
        let s = self.sandbox.as_mut().unwrap();
        for p in self
            .world
            .players
            .values_mut()
            .filter(|p| p.joined && !p.hero.identity.is_bot)
        {
            if s.initialized.insert(p.hero.identity.id)
                || !s.actors.contains_key(&p.hero.identity.id)
            {
                let mut c = s.config.player.clone();
                c.hero = p.hero.identity.hero_class;
                c.avatar = p.hero.identity.avatar.clone();
                if p.hero.identity.team == Team::Blue {
                    c.position = [3.0, 0.0];
                }
                let now = s.now;
                s.apply_actor(p, &c, true, now);
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
        self.world
            .players
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
            .world
            .players
            .get(&addr)
            .filter(|p| p.joined && !p.hero.identity.is_bot && p.protocol_compatible)
        else {
            return;
        };
        let id = player.hero.identity.id;
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
                let s = self.sandbox.as_mut().unwrap();
                let old = s.config.clone();
                let p = self.world.players.get_mut(&addr).unwrap();
                let reset = p.hero.identity.hero_class != config.player.hero;
                let id = p.hero.identity.id;
                let moved = s
                    .actors
                    .get(&id)
                    .is_some_and(|c| c.position != config.player.position);
                s.apply_actor(p, &config.player, reset, now);
                if reset || moved {
                    self.world.projectiles.retain(|_, p| {
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
                if let Some(p) = self.world.players.get_mut(&DUMMY_ADDR) {
                    p.modifiers.infinite_hp = config.dummy.infinite_hp;
                }
                if !config.environment.minions {
                    self.world.minions.clear();
                    self.world.projectiles.retain(|_, p| {
                        p.state.source_kind != CombatEntityKind::Minion
                            && p.target.kind != TargetKind::Minion
                    });
                }
                if old.environment.minions != config.environment.minions
                    || old.environment.minions_paused != config.environment.minions_paused
                {
                    self.world.last_wave_spawn_at = now;
                }
            }
            SandboxCommand::Refill { actor } => {
                let a = self.sandbox_addr(addr, actor)?;
                let p = self.world.players.get_mut(&a).unwrap();
                p.hero.hp = p.hero.max_hp;
                p.hero.mana = p.hero.max_mana;
                p.timers.respawn_at = None;
            }
            SandboxCommand::ResetCooldowns { actor } => {
                let a = self.sandbox_addr(addr, actor)?;
                self.world
                    .players
                    .get_mut(&a)
                    .unwrap()
                    .timers
                    .clear_cooldowns();
            }
            SandboxCommand::Teleport { actor, position } => {
                self.validate_sandbox_destination(position)?;
                let a = self.sandbox_addr(addr, actor)?;
                let p = self.world.players.get_mut(&a).unwrap();
                p.hero.utility.dash_sequence = p.hero.utility.dash_sequence.saturating_add(1);
                p.hero.x = position[0];
                p.hero.z = position[1];
                p.timers.last_movement_at = now;
                let id = p.hero.identity.id;
                self.world.projectiles.retain(|_, p| {
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
                let s = self.sandbox.as_mut().unwrap();
                let p = self.world.players.get_mut(&a).unwrap();
                let mut c = s.actor_config(p).ok_or("Actor has no sandbox config")?;
                let total: u32 = (1..c.level).map(xp_threshold_for_level).sum::<u32>() + c.xp;
                let mut left = (total as i64 + amount as i64).max(0).min(u32::MAX as i64) as u32;
                c.level = 1;
                while c.level < MAX_LEVEL && left >= xp_threshold_for_level(c.level) {
                    left -= xp_threshold_for_level(c.level);
                    c.level += 1;
                }
                c.xp = if c.level == MAX_LEVEL { 0 } else { left };
                s.apply_actor(p, &c, false, now);
                self.save_actor_config(actor, c);
            }
            SandboxCommand::GrantItem { actor, item } => {
                let a = self.sandbox_addr(addr, actor)?;
                let s = self.sandbox.as_mut().unwrap();
                let p = self.world.players.get_mut(&a).unwrap();
                let mut c = s.actor_config(p).ok_or("Actor has no sandbox config")?;
                c.inventory.push(item);
                validate_actor(&c)?;
                s.apply_actor(p, &c, false, now);
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
                            &self.world.map_layout,
                            &mut self.world.minions,
                            &mut self.world.next_minion_id,
                            team,
                            lane,
                        );
                    }
                }
                self.world.last_wave_spawn_at = now;
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
            .world
            .players
            .iter()
            .filter(|(_, p)| p.joined)
            .map(|(a, _)| *a)
            .collect();
        for a in addresses {
            self.reset_sandbox_actor(a, now);
        }
        self.world.projectiles.clear();
        self.world.minions.clear();
        self.world.structures = build_configured_structures(&self.world.map_config);
        let mut next_neutral_id = 9001;
        self.world.neutrals = build_neutral_camps(&mut next_neutral_id);
        self.world
            .neutrals
            .extend(build_boss_neutrals(&mut next_neutral_id));
        schedule_boss_spawns(&mut self.world.neutrals, now);
        self.world.team_buffs = TeamBuffs::default();
        self.world.forest_pickups.reset_availability();
        self.combat_log.reset_sandbox(now);
        self.world.game_state = GameState::Running;
        self.victory_at = None;
        self.world.last_wave_spawn_at = now;
    }
    fn validate_sandbox_destination(&self, position: [f32; 2]) -> Result<(), String> {
        validate_position(position)?;
        if self.world.structures.values().any(|s| {
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
            if let Some(p) = self.world.players.remove(&addr) {
                self.sandbox
                    .as_mut()
                    .unwrap()
                    .actors
                    .remove(&p.hero.identity.id);
                self.world.projectiles.retain(|_, b| {
                    b.state.owner_id != p.hero.identity.id
                        && b.target
                            != TargetId {
                                kind: TargetKind::Player,
                                id: p.hero.identity.id,
                            }
                });
            }
            return;
        }
        let new = !self.world.players.contains_key(&addr);
        if new {
            self.world.ensure_connected(addr, now);
            let p = self.world.players.get_mut(&addr).unwrap();
            p.hero.identity.is_bot = true;
            handle_join_request_with_sprite(
                p,
                Team::Blue,
                default_character_choice(),
                c.hero,
                bots::bot_avatar(c.hero, 1),
                None,
                &self.world.map_layout,
                now,
            );
        }
        if reset {
            let id = self.world.players[&addr].hero.identity.id;
            self.world.projectiles.retain(|_, p| {
                p.state.owner_id != id
                    && p.target
                        != TargetId {
                            kind: TargetKind::Player,
                            id,
                        }
            });
        }
        self.sandbox.as_mut().unwrap().apply_actor(
            self.world.players.get_mut(&addr).unwrap(),
            c,
            new || reset,
            now,
        );
    }
    fn reset_sandbox_actor(&mut self, addr: SocketAddr, now: Instant) {
        let s = self.sandbox.as_mut().unwrap();
        if let Some(p) = self.world.players.get_mut(&addr) {
            if let Some(c) = s.actor_config(p) {
                let id = p.hero.identity.id;
                s.apply_actor(p, &c, true, now);
                self.world.projectiles.retain(|_, p| {
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
        let p = &self.world.players[&addr];
        let target = if ability_for_class_slot(p.hero.identity.hero_class, skill).targeting
            == TargetingMode::SelfTarget
        {
            TargetId {
                kind: TargetKind::Player,
                id: p.hero.identity.id,
            }
        } else {
            self.world
                .players
                .values()
                .filter(|t| {
                    t.joined
                        && t.hero.hp > 0.0
                        && t.hero.identity.team != p.hero.identity.team
                        && target_id.is_none_or(|id| id == t.hero.identity.id)
                })
                .min_by(|a, b| {
                    ((a.hero.x - p.hero.x).hypot(a.hero.z - p.hero.z))
                        .total_cmp(&((b.hero.x - p.hero.x).hypot(b.hero.z - p.hero.z)))
                })
                .map(|t| TargetId {
                    kind: TargetKind::Player,
                    id: t.hero.identity.id,
                })
                .ok_or("No living hostile target")?
        };
        let before = p.hero.last_action.sequence;
        handle_cast_request(&mut self.world, addr, target, slot, now);
        if self.world.players[&addr].hero.last_action.sequence == before {
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
        for p in self.world.players.values_mut() {
            if p.modifiers.infinite_resource {
                p.hero.mana = p.hero.max_mana;
            }
            if p.modifiers.no_cooldowns {
                p.timers.clear_cooldowns();
            }
        }
        if dt <= 0.0 {
            return;
        }
        for addr in [ENEMY_ADDR, DUMMY_ADDR] {
            let Some(p) = self.world.players.get(&addr) else {
                continue;
            };
            if p.hero.hp <= 0.0 {
                if addr == DUMMY_ADDR || config.enemy.auto_respawn {
                    if p.timers.respawn_at.is_some_and(|t| now >= t) {
                        self.reset_sandbox_actor(addr, now);
                    } else if p.timers.respawn_at.is_none() {
                        self.world.players.get_mut(&addr).unwrap().timers.respawn_at =
                            Some(now + RESPAWN_DELAY);
                    }
                } else {
                    self.world.players.get_mut(&addr).unwrap().timers.respawn_at = None;
                }
                continue;
            }
            let origin = [p.hero.x, p.hero.z];
            let team = p.hero.identity.team;
            let target = self
                .world
                .players
                .values()
                .filter(|p| {
                    p.joined
                        && !p.hero.identity.is_bot
                        && p.hero.hp > 0.0
                        && p.hero.identity.team != team
                })
                .min_by_key(|p| p.hero.identity.id)
                .map(|p| (p.hero.identity.id, [p.hero.x, p.hero.z]));
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
                    let reach = shared::basic_attack_for_class(
                        self.world.players[&addr].hero.identity.hero_class,
                    )
                    .range
                        + PLAYER_HIT_RADIUS;
                    let desired = config.enemy.attack_distance.min(reach - 0.1);
                    if distance > desired {
                        self.move_sandbox_actor(addr, position, now, dt);
                    }
                    if distance <= reach {
                        let request = self.world.players[&addr]
                            .economy
                            .basic_attack_request_id
                            .saturating_add(1);
                        handle_basic_attack_request(
                            &mut self.world,
                            addr,
                            TargetId {
                                kind: TargetKind::Player,
                                id,
                            },
                            request,
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
        let p = &self.world.players[&addr];
        let origin = [p.hero.x, p.hero.z];
        let discs: Vec<_> = self
            .world
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
        let step = (hero_stats::move_speed(p) * dt).min(distance);
        handle_transform_request_with_structures(
            self.world.players.get_mut(&addr).unwrap(),
            &self.world.map_layout,
            &self.world.structures,
            origin[0] + dx / distance * step,
            PLAYER_GROUND_Y,
            origin[1] + dz / distance * step,
            // A sandbox actor is a hero model: -Z forward, unlike minions.
            shared::math::hero_yaw_towards(dx, dz),
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
        .filter(|p| p.joined && !p.hero.identity.is_bot)
    {
        match p.hero.identity.team {
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
