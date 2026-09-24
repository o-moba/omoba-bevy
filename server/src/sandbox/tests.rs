use super::*;
fn fixture() -> (ServerRuntime, SocketAddr, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    let now = Instant::now();
    rt.sandbox = Some(SandboxRuntime::new(now));
    rt.targeting_qa = true;
    let addr = "127.0.0.1:58241".parse().unwrap();
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Mage,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        },
        now,
    );
    (rt, addr, now)
}
fn command(rt: &mut ServerRuntime, a: SocketAddr, command: SandboxCommand) -> SandboxAck {
    let request_id = rt
        .sandbox
        .as_ref()
        .unwrap()
        .sequences
        .get(&rt.world.players[&a].hero.identity.id)
        .copied()
        .unwrap_or(0)
        + 1;
    rt.handle_sandbox(
        a,
        SandboxRequest {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            request_id,
            command,
        },
    );
    rt.sandbox.as_ref().unwrap().acks[&rt.world.players[&a].hero.identity.id].clone()
}
fn config(rt: &ServerRuntime) -> SandboxConfig {
    rt.sandbox.as_ref().unwrap().config.clone()
}
fn apply(rt: &mut ServerRuntime, a: SocketAddr, c: SandboxConfig) {
    let ack = command(rt, a, SandboxCommand::ApplyConfig { config: c });
    assert!(ack.accepted, "{}", ack.message);
}
fn advance(rt: &mut ServerRuntime, dt: f32) {
    let (now, dt) = rt.sandbox.as_mut().unwrap().advance(dt);
    rt.tick(now, dt);
}
#[test]
fn sandbox_defaults_valid_and_actor_edit_is_transactional() {
    let (mut rt, a, now) = fixture();
    assert!(validate_config(&config(&rt)).is_ok());
    let mut c = config(&rt);
    c.player.hero = HeroClass::Mage;
    c.player.level = 10;
    c.player.ranks = [3; 4];
    c.player.max_hp = 321.0;
    c.player.inventory = shared::shop::ITEMS.iter().map(|i| i.id).collect();
    c.player.damage_multiplier = 2.0;
    c.player.attack_speed = 3.0;
    rt.world.players.get_mut(&a).unwrap().hero.hp = 27.0;
    rt.world.players.get_mut(&a).unwrap().timers.last_cast_at[0] = Some(now);
    apply(&mut rt, a, c.clone());
    let p = &rt.world.players[&a];
    assert_eq!(p.hero.hp, 27.0);
    assert_eq!(p.timers.last_cast_at[0], Some(now));
    assert_eq!(p.hero.progress.level, 10);
    assert_eq!(p.hero.progress.ranks, [3; 4]);
    assert_eq!(p.economy.inventory.len(), 6);
    assert_eq!(p.hero.max_hp, 321.0 + 9.0 * 18.0 + 45.0);
    assert!(hero_stats::combat_bonuses(p).damage_multiplier > 2.0);
    assert!(hero_stats::combat_bonuses(p).attack_speed_multiplier > 3.0);
    c.player.max_hp = f32::NAN;
    assert!(!command(&mut rt, a, SandboxCommand::ApplyConfig { config: c }).accepted);
    assert_eq!(rt.world.players[&a].hero.hp, 27.0);
}
#[test]
fn sandbox_epoch_replay_and_mode_gates() {
    let (mut rt, a, _) = fixture();
    let request = SandboxRequest {
        server_epoch: rt.server_epoch,
        match_id: rt.match_id,
        request_id: 17,
        command: SandboxCommand::AddXp {
            actor: SandboxActor::Player,
            amount: 100,
        },
    };
    rt.handle_sandbox(a, request.clone());
    let level = rt.world.players[&a].hero.progress.level;
    let xp = rt.world.players[&a].hero.progress.xp;
    rt.handle_sandbox(a, request.clone());
    assert!(rt.sandbox.as_ref().unwrap().acks[&rt.world.players[&a].hero.identity.id].accepted);
    assert_eq!(
        (level, xp),
        (
            rt.world.players[&a].hero.progress.level,
            rt.world.players[&a].hero.progress.xp
        )
    );
    let mut stale = request;
    stale.server_epoch += 1;
    stale.request_id = 18;
    rt.handle_sandbox(a, stale);
    assert!(!rt.sandbox.as_ref().unwrap().acks[&rt.world.players[&a].hero.identity.id].accepted);
    for mode in [MatchMode::Release, MatchMode::Practice] {
        rt.rules = MatchRules::for_mode(mode, rt.rules.team_size);
        assert!(!rt.sandbox_allowed());
        command(
            &mut rt,
            a,
            SandboxCommand::AddXp {
                actor: SandboxActor::Player,
                amount: 10000,
            },
        );
        assert_eq!(rt.world.players[&a].hero.progress.level, level);
    }
    rt.rules = MatchRules::for_mode(MatchMode::Dev, rt.rules.team_size);
    rt.sandbox = None;
    assert!(!rt.sandbox_allowed());
}
#[test]
fn sandbox_resources_god_toggle_and_cooldowns_restore() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.player.god_mode = true;
    c.player.infinite_resource = true;
    c.player.no_cooldowns = true;
    apply(&mut rt, a, c.clone());
    let id = rt.world.players[&a].hero.identity.id;
    assert!(apply_player_damage(&mut rt.world.players, id, 10.0, now).is_none());
    rt.world.players.get_mut(&a).unwrap().hero.mana = 1.0;
    rt.world
        .players
        .get_mut(&a)
        .unwrap()
        .timers
        .last_basic_attack_at = Some(now);
    rt.simulate_sandbox(now, 0.01);
    assert_eq!(
        rt.world.players[&a].hero.mana,
        rt.world.players[&a].hero.max_mana
    );
    assert!(rt.world.players[&a].timers.last_basic_attack_at.is_none());
    c.player.god_mode = false;
    c.player.infinite_resource = false;
    c.player.no_cooldowns = false;
    apply(&mut rt, a, c);
    assert!(apply_player_damage(&mut rt.world.players, id, 10.0, now).is_some());
    assert!(
        command(
            &mut rt,
            a,
            SandboxCommand::Refill {
                actor: SandboxActor::Player
            }
        )
        .accepted
    );
    assert_eq!(
        rt.world.players[&a].hero.hp,
        rt.world.players[&a].hero.max_hp
    );
}
#[test]
fn sandbox_real_basic_and_ability_hits_mitigate_and_accumulate_without_duplicate() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.player.hero = HeroClass::Mage;
    c.player.position = [-1.0, 0.0];
    c.player.unlock_all = true;
    c.dummy.enabled = true;
    c.dummy.position = [1.0, 0.0];
    c.dummy.armor = 100.0;
    c.dummy.resistance = 300.0;
    apply(&mut rt, a, c);
    let target = TargetId {
        kind: TargetKind::Player,
        id: rt.world.players[&DUMMY_ADDR].hero.identity.id,
    };
    for (request, slot) in [(1, None), (2, Some(0))] {
        if let Some(slot) = slot {
            handle_cast_request(&mut rt.world, a, target, slot, now);
        } else {
            handle_basic_attack_request(&mut rt.world, a, target, request, now);
        }
        let raw = rt
            .world
            .projectiles
            .values()
            .next()
            .expect("attack launched")
            .damage;
        let events = simulate_projectiles(
            &mut rt.world,
            TickCtx {
                now: now + Duration::from_secs(1),
                dt: 1.0,
            },
        );
        assert_eq!(events.len(), 1);
        assert!((events[0].amount - raw / if slot.is_some() { 4.0 } else { 2.0 }).abs() < 0.001);
        assert!(!events[0].killed);
        rt.combat_log.extend(now, events);
    }
    assert_eq!(rt.world.players[&DUMMY_ADDR].hero.hp, 10000.0);
    let stats = rt
        .combat_log
        .sandbox_analytics(now + Duration::from_secs(2));
    assert_eq!(stats.hits, 2);
    assert_eq!(stats.breakdown.len(), 2);
    assert_eq!(stats.dps, stats.damage / 2.0);
    rt.combat_log.snapshot(now);
    assert_eq!(rt.combat_log.sandbox_analytics(now).hits, 2);
    command(&mut rt, a, SandboxCommand::ResetAnalytics);
    assert_eq!(rt.combat_log.sandbox_analytics(now).hits, 0);
}
#[test]
fn sandbox_finite_dummy_overkill_and_respawn() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.dummy.enabled = true;
    c.dummy.infinite_hp = false;
    c.dummy.max_hp = 10.0;
    apply(&mut rt, a, c);
    let id = rt.world.players[&DUMMY_ADDR].hero.identity.id;
    let event = apply_player_damage(&mut rt.world.players, id, 500.0, now).unwrap();
    assert_eq!(event.amount, 10.0);
    assert!(event.killed);
    rt.simulate_sandbox(now + RESPAWN_DELAY, 0.01);
    assert_eq!(rt.world.players[&DUMMY_ADDR].hero.hp, 10.0);
}
#[test]
fn sandbox_progression_unlock_items_teleport_reset_and_hero_swap() {
    let (mut rt, a, _) = fixture();
    let mut c = config(&rt);
    c.player.hero = HeroClass::Ranger;
    c.player.unlock_all = true;
    c.enemy.enabled = true;
    c.enemy.actor.hero = HeroClass::Cleric;
    c.enemy.actor.level = 10;
    c.enemy.actor.ranks = [3; 4];
    apply(&mut rt, a, c);
    assert_eq!(rt.world.players.len(), 2);
    for item in shared::shop::ITEMS {
        assert!(
            command(
                &mut rt,
                a,
                SandboxCommand::GrantItem {
                    actor: SandboxActor::Player,
                    item: item.id
                }
            )
            .accepted
        );
    }
    assert!(
        !command(
            &mut rt,
            a,
            SandboxCommand::GrantItem {
                actor: SandboxActor::Player,
                item: ItemId::EmberBlade
            }
        )
        .accepted
    );
    assert!(
        command(
            &mut rt,
            a,
            SandboxCommand::AddXp {
                actor: SandboxActor::Player,
                amount: i32::MAX
            }
        )
        .accepted
    );
    assert_eq!(rt.world.players[&a].hero.progress.level, 10);
    command(
        &mut rt,
        a,
        SandboxCommand::AddXp {
            actor: SandboxActor::Player,
            amount: i32::MIN,
        },
    );
    assert_eq!(rt.world.players[&a].hero.progress.level, 1);
    let seq = rt.world.players[&a].hero.utility.dash_sequence;
    command(
        &mut rt,
        a,
        SandboxCommand::Teleport {
            actor: SandboxActor::Player,
            position: [-2.0, 1.0],
        },
    );
    assert_eq!(rt.world.players[&a].hero.x, -2.0);
    assert!(rt.world.players[&a].hero.utility.dash_sequence > seq);
    command(&mut rt, a, SandboxCommand::ResetDuel);
    assert_eq!(rt.world.players[&a].hero.x, -3.0);
    assert_eq!(rt.world.players[&ENEMY_ADDR].hero.progress.level, 10);
    assert_eq!(rt.world.players[&ENEMY_ADDR].hero.progress.ranks, [3; 4]);
    assert_eq!(rt.world.players[&a].economy.inventory.len(), 6);
    assert!(rt.world.players[&a].modifiers.unlock_all);
}
#[test]
fn sandbox_ai_modes_forced_cast_and_all_heroes() {
    for hero in HeroClass::ALL {
        let (mut rt, a, now) = fixture();
        let mut c = config(&rt);
        c.enemy.enabled = true;
        c.enemy.actor.hero = hero;
        c.enemy.actor.unlock_all = true;
        c.enemy.actor.position = [1.0, 0.0];
        c.player.position = [-1.0, 0.0];
        apply(&mut rt, a, c.clone());
        let original = rt.world.players[&ENEMY_ADDR].hero.x;
        rt.simulate_sandbox(now + Duration::from_secs(1), 0.1);
        assert_eq!(rt.world.players[&ENEMY_ADDR].hero.x, original);
        c.enemy.behavior = BotBehavior::Flee;
        apply(&mut rt, a, c.clone());
        rt.simulate_sandbox(now + Duration::from_secs(2), 0.1);
        assert!(rt.world.players[&ENEMY_ADDR].hero.x > original);
        c.enemy.behavior = BotBehavior::Attack;
        apply(&mut rt, a, c.clone());
        rt.simulate_sandbox(now + Duration::from_secs(3), 0.1);
        assert!(!rt.world.projectiles.is_empty());
        rt.world.projectiles.clear();
        assert!(
            command(
                &mut rt,
                a,
                SandboxCommand::ForceCast {
                    target_id: None,
                    actor: SandboxActor::Enemy,
                    slot: 0
                }
            )
            .accepted
        );
        assert!(!rt.world.projectiles.is_empty());
        rt.world.projectiles.clear();
        c.enemy.behavior = BotBehavior::Fight;
        c.enemy.actor.no_cooldowns = true;
        c.enemy.actor.infinite_resource = true;
        apply(&mut rt, a, c);
        rt.simulate_sandbox(now + Duration::from_secs(4), 0.1);
        assert!(rt.world.players[&ENEMY_ADDR].hero.last_action.sequence > 2);
    }
}
#[test]
fn sandbox_clock_scales_pause_frame_and_keepalive() {
    let (mut rt, a, now) = fixture();
    for scale in TIME_SCALES {
        let s = rt.sandbox.as_mut().unwrap();
        s.config.environment.time_scale = scale;
        let before = s.now;
        let (after, dt) = s.advance(0.1);
        assert!((dt - 0.1 * scale).abs() < 0.0001);
        assert!((after.duration_since(before).as_secs_f32() - dt).abs() < 0.0001);
    }
    rt.sandbox.as_mut().unwrap().config.environment.paused = true;
    let sim = rt.sandbox.as_ref().unwrap().now;
    let frame = rt.sandbox.as_ref().unwrap().frame;
    for seconds in 1..20 {
        rt.handle_packet(a, ClientPacket::Ping, now + Duration::from_secs(seconds));
        let (s, dt) = rt.sandbox.as_mut().unwrap().advance(1.0);
        assert_eq!(s, sim);
        assert_eq!(dt, 0.0);
        rt.maintain_roster(now + Duration::from_secs(seconds));
        assert!(rt.world.players.contains_key(&a));
    }
    assert_eq!(rt.sandbox.as_ref().unwrap().frame, frame);
    command(&mut rt, a, SandboxCommand::FrameStep);
    let (after, dt) = rt.sandbox.as_mut().unwrap().advance(9.0);
    assert!(after > sim);
    assert!((dt - 1.0 / 60.0).abs() < 0.0001);
    assert_eq!(rt.sandbox.as_ref().unwrap().frame, frame + 1);
}
#[test]
fn sandbox_minion_enable_spawn_pause_disable() {
    let (mut rt, a, _) = fixture();
    rt.targeting_qa = false;
    assert!(!command(&mut rt, a, SandboxCommand::SpawnWave).accepted);
    let mut c = config(&rt);
    c.environment.minions = true;
    c.environment.minions_paused = true;
    apply(&mut rt, a, c.clone());
    assert!(command(&mut rt, a, SandboxCommand::SpawnWave).accepted);
    let positions: HashMap<_, _> = rt
        .world
        .minions
        .iter()
        .map(|(id, m)| (*id, [m.state.x, m.state.z]))
        .collect();
    assert!(!positions.is_empty());
    advance(&mut rt, 0.1);
    for (id, m) in &rt.world.minions {
        assert_eq!(positions[id], [m.state.x, m.state.z]);
    }
    c.environment.minions_paused = false;
    apply(&mut rt, a, c.clone());
    advance(&mut rt, 0.1);
    assert!(rt.world.minions.iter().any(|(id, m)| {
        positions
            .get(id)
            .is_some_and(|p| *p != [m.state.x, m.state.z])
    }));
    c.environment.minions = false;
    apply(&mut rt, a, c);
    assert!(rt.world.minions.is_empty());
    advance(&mut rt, 50.0);
    assert!(rt.world.minions.is_empty());
}
#[test]
fn sandbox_second_human_config_is_independent() {
    let (mut rt, a, now) = fixture();
    let b = "127.0.0.1:58242".parse().unwrap();
    rt.handle_packet(
        b,
        ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Cleric,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        },
        now,
    );
    assert_eq!(rt.world.players[&b].hero.identity.team, Team::Blue);
    assert_eq!(
        assign_human_team(&rt.world.players, &rt.world.disconnected_sessions),
        None
    );
    let mut c = config(&rt);
    c.player.level = 10;
    apply(&mut rt, a, c);
    assert_eq!(rt.world.players[&b].hero.progress.level, 1);
    let snap = rt
        .sandbox
        .as_ref()
        .unwrap()
        .snapshot(b, &rt.world.players, &rt.combat_log);
    assert_eq!(snap.config.player.hero, HeroClass::Cleric);
    assert_eq!(snap.config.player.level, 1);
    assert_eq!(
        rt.world
            .players
            .values()
            .filter(|p| !p.hero.identity.is_bot)
            .count(),
        2
    );
}

#[test]
fn sandbox_invalid_values_and_foreign_actor_do_not_partially_mutate() {
    let (mut rt, a, _) = fixture();
    let baseline = rt
        .sandbox
        .as_ref()
        .unwrap()
        .snapshot(a, &rt.world.players, &rt.combat_log)
        .config;
    let mut invalids = Vec::new();
    for level in [0, 11, u32::MAX] {
        let mut c = baseline.clone();
        c.player.level = level;
        invalids.push(c);
    }
    for value in [f32::NAN, f32::INFINITY, -1.0, 10001.0] {
        let mut c = baseline.clone();
        c.enemy.actor.armor = value;
        invalids.push(c);
    }
    let mut c = baseline.clone();
    c.enemy.actor.ranks[3] = 4;
    invalids.push(c);
    let mut c = baseline.clone();
    c.environment.time_scale = 3.0;
    invalids.push(c);
    let mut c = baseline.clone();
    c.version = 99;
    invalids.push(c);
    let mut c = baseline.clone();
    c.player.inventory = vec![ItemId::EmberBlade; 2];
    invalids.push(c);
    let mut c = baseline.clone();
    c.player.avatar = Some("does-not-exist".into());
    invalids.push(c);
    for mut c in invalids {
        c.player.god_mode = true;
        assert!(!command(&mut rt, a, SandboxCommand::ApplyConfig { config: c }).accepted);
        assert_eq!(
            rt.sandbox
                .as_ref()
                .unwrap()
                .actor_config(&rt.world.players[&a])
                .unwrap(),
            baseline.player
        );
        assert_eq!(rt.world.players.len(), 1);
    }
    assert!(
        !command(
            &mut rt,
            a,
            SandboxCommand::ForceCast {
                target_id: None,
                actor: SandboxActor::Enemy,
                slot: 0
            }
        )
        .accepted
    );
    assert!(
        !command(
            &mut rt,
            a,
            SandboxCommand::Teleport {
                actor: SandboxActor::Player,
                position: [500.0, 500.0]
            }
        )
        .accepted
    );
    let source = serde_json::to_value(&baseline).unwrap();
    let mut bad = source.clone();
    bad["player"]["hero"] = serde_json::json!("future_unknown_hero");
    assert!(serde_json::from_value::<SandboxConfig>(bad).is_err());
    let mut bad = source;
    bad["player"]["unexpected_admin_switch"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SandboxConfig>(bad).is_err());
}

#[test]
fn sandbox_simulation_time_controls_real_movement_and_cooldown() {
    let mut distances = Vec::new();
    for scale in TIME_SCALES {
        let (mut rt, a, wall) = fixture();
        rt.sandbox.as_mut().unwrap().config.environment.time_scale = scale;
        let start = rt.world.players[&a].hero.x;
        let now = rt.sandbox.as_ref().unwrap().now;
        rt.world
            .players
            .get_mut(&a)
            .unwrap()
            .timers
            .last_basic_attack_at = Some(now);
        let (sim, dt) = rt.sandbox.as_mut().unwrap().advance(0.1);
        let seq = rt.world.players[&a].hero.utility.dash_sequence;
        rt.handle_packet(
            a,
            ClientPacket::Transform {
                dash_sequence: seq,
                x: 0.0,
                y: PLAYER_GROUND_Y,
                z: 0.0,
                yaw: 0.0,
            },
            wall + Duration::from_millis(100),
        );
        let distance = rt.world.players[&a].hero.x - start;
        assert!((distance - (PLAYER_SPEED * dt + MOVEMENT_POSITION_TOLERANCE)).abs() < 0.001);
        distances.push(distance);
        let view = rt.player_view(a, sim);
        assert!(
            (view.basic_attack_remaining_secs - (view.basic_attack_cooldown_secs - dt).max(0.0))
                .abs()
                < 0.001
        );
        rt.sandbox.as_mut().unwrap().config.environment.paused = true;
        let position = rt.world.players[&a].hero.x;
        rt.handle_packet(
            a,
            ClientPacket::Transform {
                dash_sequence: seq,
                x: 0.0,
                y: PLAYER_GROUND_Y,
                z: 0.0,
                yaw: 0.0,
            },
            wall + Duration::from_secs(1),
        );
        assert_eq!(rt.world.players[&a].hero.x, position);
    }
    assert!(distances.windows(2).all(|w| w[1] > w[0]));
}

#[test]
fn sandbox_reset_cooldowns_preserves_utility_replay_and_active_haste() {
    let (mut rt, a, now) = fixture();
    let mut c = rt
        .sandbox
        .as_ref()
        .unwrap()
        .snapshot(a, &rt.world.players, &rt.combat_log)
        .config;
    c.player.no_cooldowns = true;
    apply(&mut rt, a, c);
    let p = rt.world.players.get_mut(&a).unwrap();
    handle_utility_request(
        p,
        &rt.world.map_layout,
        &rt.world.structures,
        &rt.world.game_state,
        shared::utility::UtilityAction::Haste,
        [0.0, 0.0],
        77,
        now,
    );
    assert!(hero_timers::haste_active(p, now) > 0.0);
    rt.simulate_sandbox(now, 0.1);
    assert_eq!(rt.world.players[&a].hero.utility.last_request_id, 77);
    assert!(rt.player_view(a, now).utility.haste_active_secs > 0.0);
    command(&mut rt, a, SandboxCommand::ResetDuel);
    assert_eq!(rt.world.players[&a].hero.utility.last_request_id, 77);
    assert_eq!(rt.player_view(a, now).utility.haste_active_secs, 0.0);
}

#[test]
fn sandbox_analytics_keeps_source_kinds_distinct_and_latest_hit_identity() {
    let now = Instant::now();
    let mut log = CombatLog::default();
    log.enable_sandbox(now);
    for (kind, amount) in [
        (CombatEntityKind::Player, 10.0),
        (CombatEntityKind::Minion, 4.0),
        (CombatEntityKind::Player, 3.0),
    ] {
        log.extend(
            now,
            [CombatEvent {
                source: CombatEntity { kind, id: 1 },
                target: CombatEntity {
                    kind: CombatEntityKind::Player,
                    id: 5,
                },
                amount,
                ..Default::default()
            }],
        );
    }
    let stats = log.sandbox_analytics(now + Duration::from_secs(2));
    assert_eq!(stats.breakdown.len(), 2);
    let player = stats
        .breakdown
        .iter()
        .find(|b| b.source_kind == CombatEntityKind::Player)
        .unwrap();
    assert_eq!(player.damage, 13.0);
    assert_eq!(player.last_hit, 3.0);
    assert_eq!(player.hits, 2);
    assert_eq!(player.last_event_id, 3);
}

#[test]
fn sandbox_enemy_respawn_policy_and_dummy_motion_are_authoritative() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.enemy.enabled = true;
    c.enemy.auto_respawn = false;
    c.dummy.enabled = true;
    c.dummy.moving = true;
    apply(&mut rt, a, c);
    let id = rt.world.players[&ENEMY_ADDR].hero.identity.id;
    apply_player_damage(&mut rt.world.players, id, 9999.0, now);
    rt.sandbox.as_mut().unwrap().elapsed = 1.0;
    rt.simulate_sandbox(now + Duration::from_secs(60), 0.1);
    handle_respawns(&mut rt.world, now + Duration::from_secs(60));
    assert_eq!(rt.world.players[&ENEMY_ADDR].hero.hp, 0.0);
    assert_ne!(rt.world.players[&DUMMY_ADDR].hero.x, 0.0);
    command(
        &mut rt,
        a,
        SandboxCommand::ResetActor {
            actor: SandboxActor::Enemy,
        },
    );
    assert!(rt.world.players[&ENEMY_ADDR].hero.hp > 0.0);
}

#[test]
fn sandbox_duel_restart_preserves_controls_and_restores_environment() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.player.level = 10;
    c.player.ranks = [3; 4];
    c.player.god_mode = true;
    c.enemy.enabled = true;
    c.enemy.actor.hero = HeroClass::Ranger;
    c.enemy.actor.level = 6;
    c.enemy.auto_respawn = false;
    apply(&mut rt, a, c.clone());
    for structure in rt.world.structures.values_mut() {
        structure.state.hp = 0.0;
    }
    rt.world.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.victory_at = Some(now);
    rt.restart_round(now);
    assert_eq!(rt.world.game_state, GameState::Running);
    assert!(
        rt.world
            .structures
            .values()
            .all(|s| s.state.hp == s.state.max_hp)
    );
    assert_eq!(rt.world.players[&a].hero.progress.level, 10);
    assert_eq!(rt.world.players[&a].hero.progress.ranks, [3; 4]);
    assert!(rt.world.players[&a].modifiers.god_mode);
    assert_eq!(
        rt.world.players[&ENEMY_ADDR].hero.identity.hero_class,
        HeroClass::Ranger
    );
    assert_eq!(rt.world.players[&ENEMY_ADDR].hero.progress.level, 6);
    let id = rt.world.players[&ENEMY_ADDR].hero.identity.id;
    apply_player_damage(&mut rt.world.players, id, 9999.0, now);
    rt.simulate_sandbox(now, 0.1);
    assert!(rt.world.players[&ENEMY_ADDR].timers.respawn_at.is_none());
    c.enemy.auto_respawn = true;
    apply(&mut rt, a, c);
    rt.simulate_sandbox(now, 0.1);
    rt.simulate_sandbox(now + RESPAWN_DELAY, 0.1);
    assert!(rt.world.players[&ENEMY_ADDR].hero.hp > 0.0);
}

#[test]
fn sandbox_subunit_damage_applies_to_resolved_basics_abilities_and_telemetry() {
    for inventory in [Vec::new(), vec![ItemId::EmberBlade]] {
        let (mut rt, a, now) = fixture();
        let mut c = config(&rt);
        c.player.hero = HeroClass::Mage;
        c.player.position = [-1.0, 0.0];
        c.player.infinite_resource = true;
        c.player.inventory = inventory.clone();
        c.dummy.enabled = true;
        c.dummy.infinite_hp = false;
        c.dummy.position = [1.0, 0.0];
        let item_bonus = shared::shop::item_bonuses(&inventory).damage_multiplier;
        for (index, multiplier) in [0.0, 0.5, 1.0].into_iter().enumerate() {
            c.player.damage_multiplier = multiplier;
            apply(&mut rt, a, c.clone());
            command(
                &mut rt,
                a,
                SandboxCommand::ResetCooldowns {
                    actor: SandboxActor::Player,
                },
            );
            let target = TargetId {
                kind: TargetKind::Player,
                id: rt.world.players[&DUMMY_ADDR].hero.identity.id,
            };
            let expected_basic =
                shared::basic_attack_for_class(HeroClass::Mage).damage * item_bonus * multiplier;
            let telemetry =
                rt.sandbox
                    .as_ref()
                    .unwrap()
                    .snapshot(a, &rt.world.players, &rt.combat_log);
            assert!(
                (telemetry
                    .actors
                    .iter()
                    .find(|actor| actor.actor == SandboxActor::Player)
                    .unwrap()
                    .attack_damage
                    - expected_basic)
                    .abs()
                    < 0.001
            );
            for ability in [false, true] {
                let expected = if ability {
                    HeroClass::Mage.abilities()[0].projectile_damage.unwrap()
                        * item_bonus
                        * multiplier
                } else {
                    expected_basic
                };
                let hp_before = rt.world.players[&DUMMY_ADDR].hero.hp;
                if ability {
                    handle_cast_request(&mut rt.world, a, target, 0, now);
                } else {
                    handle_basic_attack_request(&mut rt.world, a, target, index as u64 + 1, now);
                }
                assert_eq!(
                    rt.world.projectiles.len(),
                    1,
                    "accepted input must launch its ordinary projectile"
                );
                assert!(
                    (rt.world.projectiles.values().next().unwrap().damage - expected).abs() < 0.001
                );
                let events = simulate_projectiles(
                    &mut rt.world,
                    TickCtx {
                        now: now + Duration::from_secs(1),
                        dt: 1.0,
                    },
                );
                assert!(
                    (hp_before - rt.world.players[&DUMMY_ADDR].hero.hp - expected).abs() < 0.002
                );
                if multiplier == 0.0 {
                    assert!(
                        events.is_empty(),
                        "zero damage must not create a positive damage receipt"
                    );
                } else {
                    assert_eq!(events.len(), 1);
                    assert!((events[0].amount - expected).abs() < 0.002);
                }
            }
        }
    }
}

#[test]
fn sandbox_quarter_attack_speed_delays_real_basic_and_q_gates_then_can_be_disabled() {
    let (mut rt, a, now) = fixture();
    let mut c = config(&rt);
    c.player.hero = HeroClass::Mage;
    c.player.position = [-1.0, 0.0];
    c.player.infinite_resource = true;
    c.player.attack_speed = 0.25;
    c.player.inventory = vec![ItemId::SwiftGrip];
    c.dummy.enabled = true;
    c.dummy.position = [1.0, 0.0];
    apply(&mut rt, a, c.clone());
    let target = TargetId {
        kind: TargetKind::Player,
        id: rt.world.players[&DUMMY_ADDR].hero.identity.id,
    };
    let items = shared::shop::item_bonuses(&c.player.inventory);
    let ordinary_basic =
        shared::shop::basic_attack_cooldown(shared::basic_attack_for_class(c.player.hero), items);
    let ordinary_q =
        shared::shop::item_cooldown(&c.player.hero.abilities()[0], 1, SkillSlot::Q, items);
    let ordinary_w =
        shared::shop::item_cooldown(&c.player.hero.abilities()[1], 1, SkillSlot::W, items);
    let slow_basic = ordinary_basic.mul_f32(4.0);
    let slow_q = ordinary_q.mul_f32(4.0);
    assert!(
        (hero_stats::basic_attack_cooldown(&rt.world.players[&a]).as_secs_f32()
            - slow_basic.as_secs_f32())
        .abs()
            < 0.00001
    );
    assert!(
        (hero_stats::ability_cooldown(&rt.world.players[&a], SkillSlot::Q).as_secs_f32()
            - slow_q.as_secs_f32())
        .abs()
            < 0.00001
    );
    assert_eq!(
        hero_stats::ability_cooldown(&rt.world.players[&a], SkillSlot::W),
        ordinary_w
    );
    let basic = |rt: &mut ServerRuntime, request, at| {
        handle_basic_attack_request(&mut rt.world, a, target, request, at)
    };
    let cast = |rt: &mut ServerRuntime, at| handle_cast_request(&mut rt.world, a, target, 0, at);
    basic(&mut rt, 1, now);
    cast(&mut rt, now);
    assert_eq!(rt.world.projectiles.len(), 2);
    assert!(
        (rt.player_view(a, now).basic_attack_cooldown_secs - slow_basic.as_secs_f32()).abs()
            < 0.00001
    );
    let telemetry = rt
        .sandbox
        .as_ref()
        .unwrap()
        .snapshot(a, &rt.world.players, &rt.combat_log);
    let actor = telemetry
        .actors
        .iter()
        .find(|actor| actor.actor == SandboxActor::Player)
        .unwrap();
    assert!((actor.cooldowns[0] - slow_q.as_secs_f32()).abs() < 0.00001);
    assert!((actor.attack_speed - 0.25 * items.attack_speed_multiplier).abs() < 0.00001);
    rt.world.projectiles.clear();
    basic(&mut rt, 2, now + ordinary_basic);
    cast(&mut rt, now + ordinary_q);
    assert!(
        rt.world.projectiles.is_empty(),
        "ordinary ready times must remain blocked under a slower override"
    );
    basic(&mut rt, 3, now + slow_basic - Duration::from_millis(1));
    cast(&mut rt, now + slow_q - Duration::from_millis(1));
    assert!(rt.world.projectiles.is_empty());
    basic(&mut rt, 4, now + slow_basic + Duration::from_micros(1));
    cast(&mut rt, now + slow_q + Duration::from_micros(1));
    assert_eq!(
        rt.world.projectiles.len(),
        2,
        "both clocks become ready at their extended cooldowns"
    );
    rt.sandbox.as_mut().unwrap().now = now + slow_basic.max(slow_q) + Duration::from_secs(1);
    c.player.attack_speed = 1.0;
    apply(&mut rt, a, c);
    command(
        &mut rt,
        a,
        SandboxCommand::ResetCooldowns {
            actor: SandboxActor::Player,
        },
    );
    assert_eq!(
        hero_stats::basic_attack_cooldown(&rt.world.players[&a]),
        ordinary_basic
    );
    assert_eq!(
        hero_stats::ability_cooldown(&rt.world.players[&a], SkillSlot::Q),
        ordinary_q
    );
    let reset_at = rt.sandbox.as_ref().unwrap().now;
    rt.world.projectiles.clear();
    basic(&mut rt, 5, reset_at);
    cast(&mut rt, reset_at);
    rt.world.projectiles.clear();
    basic(&mut rt, 6, reset_at + ordinary_basic);
    cast(&mut rt, reset_at + ordinary_q);
    assert_eq!(
        rt.world.projectiles.len(),
        2,
        "disabling the override restores ordinary attack timing"
    );
}

#[test]
fn sandbox_low_multiplier_exception_does_not_change_ordinary_item_floors() {
    let (mut rt, a, _) = fixture();
    let player = rt.world.players.get_mut(&a).unwrap();
    player.modifiers = StatModifiers::default();
    player.economy.item_bonuses.damage_multiplier = 0.0;
    player.economy.item_bonuses.attack_speed_multiplier = 0.25;
    let def = shared::basic_attack_for_class(player.hero.identity.hero_class);
    assert_eq!(hero_stats::basic_attack_damage(player), def.damage);
    assert_eq!(
        hero_stats::basic_attack_cooldown(player),
        Duration::from_secs_f32(def.cooldown_secs)
    );
    assert_eq!(
        hero_stats::ability_cooldown(player, SkillSlot::Q),
        shared::scaled_cooldown(
            &player.hero.identity.hero_class.abilities()[0],
            player.hero.progress.ranks[0]
        )
    );
}

#[test]
fn reclaimed_actor_keeps_ack_and_sequence_after_address_change() {
    let (mut rt, old, _) = fixture();
    command(
        &mut rt,
        old,
        SandboxCommand::AddXp {
            actor: SandboxActor::Player,
            amount: 100,
        },
    );
    let player = rt.world.players.remove(&old).unwrap();
    let id = player.hero.identity.id;
    let new = "127.0.0.1:58242".parse().unwrap();
    rt.world.players.insert(new, player);
    let before = rt
        .sandbox
        .as_ref()
        .unwrap()
        .snapshot(new, &rt.world.players, &rt.combat_log);
    assert_eq!(before.last_request_id, 1);
    assert_eq!(before.ack.as_ref().unwrap().request_id, 1);
    assert!(before.ack.as_ref().unwrap().accepted);
    assert!(
        command(
            &mut rt,
            new,
            SandboxCommand::Refill {
                actor: SandboxActor::Player
            }
        )
        .accepted
    );
    assert_eq!(rt.sandbox.as_ref().unwrap().sequences[&id], 2);
}
