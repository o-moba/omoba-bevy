use super::*;

fn fixture() -> (ServerRuntime, Instant) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut runtime = ServerRuntime::new(socket, MatchConfig::dev());
    runtime.game_state = GameState::Running;
    for structure in runtime.structures.values_mut() {
        structure.state.hp = if structure.state.kind == StructureKind::BaseTower {
            5.0
        } else {
            0.0
        };
    }
    (runtime, Instant::now())
}

fn base_id(runtime: &ServerRuntime, team: Team) -> u64 {
    runtime
        .structures
        .values()
        .find(|structure| {
            structure.state.team == team && structure.state.kind == StructureKind::BaseTower
        })
        .unwrap()
        .state
        .id
}

fn impact(
    runtime: &mut ServerRuntime,
    now: Instant,
    projectile_id: u64,
    target: TargetId,
    team: Team,
    position: Vec3f,
    damage: f32,
) {
    runtime.projectiles.insert(
        projectile_id,
        Projectile {
            state: ProjectileState {
                id: projectile_id,
                owner_id: 90 + projectile_id,
                owner_team: team,
                source_kind: CombatEntityKind::Player,
                style: ProjectileStyle::Arcane,
                action_slot: Some(0),
                direction: [0.0; 3],
                x: position.x,
                y: position.y,
                z: position.z,
            },
            target,
            velocity: Vec3f::new(0.0, 0.0, 0.0),
            homing: true,
            guaranteed_hit: true,
            damage,
            radius: 0.1,
            expires_at: now + Duration::from_secs(1),
        },
    );
}

fn base_impact(runtime: &mut ServerRuntime, now: Instant, projectile_id: u64, team: Team) {
    let defending_team = if team == Team::Green {
        Team::Blue
    } else {
        Team::Green
    };
    let target_id = base_id(runtime, defending_team);
    let target = &runtime.structures[&target_id].state;
    let position = Vec3f::new(target.x, target.y, target.z);
    impact(
        runtime,
        now,
        projectile_id,
        TargetId {
            kind: TargetKind::Structure,
            id: target_id,
        },
        team,
        position,
        999.0,
    );
}

fn simulate_impacts(runtime: &mut ServerRuntime, now: Instant) -> Vec<CombatEvent> {
    simulate_projectiles(
        &mut runtime.players,
        &mut runtime.minions,
        &mut runtime.structures,
        &mut runtime.neutrals,
        &mut runtime.team_buffs,
        &mut runtime.projectiles,
        &mut runtime.game_state,
        0.01,
        now,
    )
}

#[test]
fn first_base_kill_freezes_winner_and_retains_exact_final_blow() {
    let (mut runtime, _) = fixture();
    let green_base = base_id(&runtime, Team::Green);
    let blue_base = base_id(&runtime, Team::Blue);
    let final_blow = apply_structure_damage(
        &mut runtime.structures,
        blue_base,
        999.0,
        Team::Green,
        &mut runtime.game_state,
    )
    .unwrap();
    assert_eq!(final_blow.amount, 5.0);
    assert!(final_blow.killed);
    assert_eq!(final_blow.target.id, blue_base);
    assert!(
        apply_structure_damage(
            &mut runtime.structures,
            green_base,
            999.0,
            Team::Blue,
            &mut runtime.game_state,
        )
        .is_none()
    );
    assert!(matches!(
        runtime.game_state,
        GameState::Victory {
            winner: Team::Green
        }
    ));
    assert_eq!(runtime.structures[&green_base].state.hp, 5.0);
}

#[test]
fn structure_damage_is_rejected_in_every_non_running_phase() {
    let (mut runtime, _) = fixture();
    let blue_base = base_id(&runtime, Team::Blue);
    for mut state in [
        GameState::Lobby,
        GameState::Forming {
            ready: 1,
            needed: 2,
        },
        GameState::Starting { countdown_ms: 100 },
        GameState::Victory { winner: Team::Blue },
    ] {
        assert!(
            apply_structure_damage(
                &mut runtime.structures,
                blue_base,
                999.0,
                Team::Green,
                &mut state,
            )
            .is_none()
        );
        assert_eq!(runtime.structures[&blue_base].state.hp, 5.0);
    }
}

#[test]
fn simultaneous_base_projectiles_resolve_by_creation_id_for_either_team() {
    for first_team in [Team::Green, Team::Blue] {
        let second_team = if first_team == Team::Green {
            Team::Blue
        } else {
            Team::Green
        };
        for reverse_insertion in [false, true] {
            // Fresh randomized HashMaps exercise iteration order independently of insertion.
            for _ in 0..16 {
                let (mut runtime, now) = fixture();
                let order = if reverse_insertion { [2, 1] } else { [1, 2] };
                for id in order {
                    base_impact(
                        &mut runtime,
                        now,
                        id,
                        if id == 1 { first_team } else { second_team },
                    );
                }
                let events = simulate_impacts(&mut runtime, now);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].source.id, 91);
                assert_eq!(events[0].target.id, base_id(&runtime, second_team));
                assert_eq!(events[0].amount, 5.0);
                assert!(events[0].killed);
                assert!(
                    matches!(runtime.game_state, GameState::Victory { winner } if winner == first_team)
                );
                assert_eq!(
                    runtime.structures[&base_id(&runtime, first_team)].state.hp,
                    5.0
                );
                assert!(simulate_impacts(&mut runtime, now).is_empty());
            }
        }
    }
}

#[test]
fn projectile_batch_keeps_pre_victory_hit_but_blocks_later_hero_and_jungle_damage() {
    let (mut runtime, now) = fixture();
    let addr: SocketAddr = "127.0.0.1:58190".parse().unwrap();
    ensure_player_connected(
        &mut runtime.players,
        &runtime.map_layout,
        addr,
        &mut runtime.next_player_id,
        now,
    );
    let player = runtime.players.get_mut(&addr).unwrap();
    player.joined = true;
    let player_id = player.state.id;
    let hero_hp = player.state.hp;
    let hero_pos = Vec3f::new(player.state.x, player.state.y + AIM_HEIGHT, player.state.z);
    let rewards = (player.state.gold, player.state.xp);
    let neutral_id = *runtime.neutrals.keys().min().unwrap();
    let neutral = &runtime.neutrals[&neutral_id].state;
    let neutral_hp = neutral.hp;
    let neutral_pos = Vec3f::new(neutral.x, neutral.y + NEUTRAL_RADIUS * 0.85, neutral.z);
    base_impact(&mut runtime, now, 2, Team::Green);
    for (id, damage) in [(1, 3.0), (3, 999.0)] {
        impact(
            &mut runtime,
            now,
            id,
            TargetId {
                kind: TargetKind::Player,
                id: player_id,
            },
            Team::Blue,
            hero_pos,
            damage,
        );
    }
    impact(
        &mut runtime,
        now,
        4,
        TargetId {
            kind: TargetKind::Neutral,
            id: neutral_id,
        },
        Team::Green,
        neutral_pos,
        999.0,
    );
    let events = simulate_impacts(&mut runtime, now);
    assert_eq!(events.len(), 2);
    assert_eq!(
        (events[0].target.kind, events[0].amount),
        (CombatEntityKind::Player, 3.0)
    );
    assert_eq!(events[1].target.kind, CombatEntityKind::Structure);
    assert!(events[1].killed);
    assert_eq!(runtime.players[&addr].state.hp, hero_hp - 3.0);
    assert_eq!(runtime.neutrals[&neutral_id].state.hp, neutral_hp);
    assert!(runtime.neutrals[&neutral_id].dead_until.is_none());
    assert_eq!(
        (
            runtime.players[&addr].state.gold,
            runtime.players[&addr].state.xp
        ),
        rewards
    );
}

#[test]
fn simultaneous_melee_base_kills_use_stable_minion_source_order() {
    for first_team in [Team::Green, Team::Blue] {
        let (mut runtime, now) = fixture();
        let other_team = if first_team == Team::Green {
            Team::Blue
        } else {
            Team::Green
        };
        for team in [first_team, other_team] {
            spawn_minion_wave_for_team_lane(
                &runtime.map_layout,
                &mut runtime.minions,
                &mut runtime.next_minion_id,
                team,
                Lane::Mid,
            );
        }
        runtime.minions.retain(|id, _| *id == 1 || *id == 4);
        let green_base = base_id(&runtime, Team::Green);
        let blue_base = base_id(&runtime, Team::Blue);
        for minion in runtime.minions.values_mut() {
            let target_id = if minion.state.team == Team::Green {
                blue_base
            } else {
                green_base
            };
            let target = &runtime.structures[&target_id].state;
            minion.state.x = target.x;
            minion.state.z = target.z;
        }
        let events = simulate_minions(
            &mut runtime.players,
            &mut runtime.minions,
            &mut runtime.structures,
            &mut runtime.projectiles,
            &mut runtime.next_projectile_id,
            &mut runtime.game_state,
            0.01,
            now,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].source,
            CombatEntity {
                kind: CombatEntityKind::Minion,
                id: 1
            }
        );
        assert_eq!(events[0].amount, 5.0);
        assert!(events[0].killed);
        assert!(
            matches!(runtime.game_state, GameState::Victory { winner } if winner == first_team)
        );
        assert_eq!(
            runtime.structures[&base_id(&runtime, first_team)].state.hp,
            5.0
        );
    }
}

#[test]
fn queued_ecs_damage_is_discarded_after_terminal_and_cannot_leak_into_next_round() {
    use gameplay::combat::{DamageEvent, apply_projectile_minion_damage_system};
    let (mut runtime, now) = fixture();
    spawn_minion_wave_for_team_lane(
        &runtime.map_layout,
        &mut runtime.minions,
        &mut runtime.next_minion_id,
        Team::Blue,
        Lane::Mid,
    );
    let hp = runtime.minions[&1].state.hp;
    runtime.game_state = GameState::Victory {
        winner: Team::Green,
    };
    let mut app = App::new();
    app.add_plugins(GameplayPlugin)
        .insert_resource(runtime)
        .insert_resource(TickContext {
            now: Some(now),
            dt: 0.01,
        })
        .add_systems(Update, apply_projectile_minion_damage_system);
    app.world_mut().write_message(DamageEvent {
        target_id: 1,
        amount: 999.0,
        attacker_team: Team::Green,
        source: HitSource::new(CombatEntityKind::Player, 99, ProjectileStyle::Arrow),
    });
    app.update();
    {
        let mut runtime = app.world_mut().resource_mut::<ServerRuntime>();
        assert_eq!(runtime.minions[&1].state.hp, hp);
        assert!(runtime.combat_log.snapshot(now).is_empty());
        runtime.game_state = GameState::Running;
    }
    app.update();
    let mut runtime = app.world_mut().resource_mut::<ServerRuntime>();
    assert_eq!(runtime.minions[&1].state.hp, hp);
    assert!(runtime.combat_log.snapshot(now).is_empty());
}
