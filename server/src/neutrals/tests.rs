use super::*;

fn player_fixture(
    class: HeroClass,
    now: Instant,
) -> (HashMap<SocketAddr, ConnectedPlayer>, SocketAddr) {
    let mut players = HashMap::new();
    let mut next_id = 1;
    let addr = "127.0.0.1:58401".parse().unwrap();
    let layout = build_map_layout();
    ensure_player_connected(&mut players, &layout, addr, &mut next_id, now);
    handle_join_request(
        players.get_mut(&addr).unwrap(),
        Team::Green,
        CharacterChoice::Ipfs,
        class,
        None,
        &layout,
        now,
    );
    (players, addr)
}

fn camps() -> HashMap<u64, Neutral> {
    build_neutral_camps(&mut 9_001)
}

fn assert_reset(neutral: &Neutral) {
    assert_eq!(neutral.state.hp, neutral.state.max_hp);
    assert_eq!(neutral.state.x, neutral.anchor.x);
    assert_eq!(neutral.state.y, neutral.anchor.y);
    assert_eq!(neutral.state.z, neutral.anchor.z);
    assert_eq!(neutral.state.yaw, 0.0);
    assert_eq!(neutral.state.ai_state, NeutralAiState::Idle);
    assert!(neutral.target_player_id.is_none());
    assert!(neutral.last_attack_at.is_none());
}

#[test]
fn six_camps_have_shared_symmetric_reachable_anchors_and_stable_ids() {
    let camps = camps();
    assert_eq!(camps.len(), 6);
    let navigation = shared::navigation::world_navigation();
    let layout = build_map_layout();
    for (index, (point, kind)) in shared::jungle::camp_layout(jungle_map_size())
        .into_iter()
        .enumerate()
    {
        let neutral = &camps[&(9_001 + index as u64)];
        assert_reset(neutral);
        assert_eq!([neutral.anchor.x, neutral.anchor.z], point);
        let expected_kind = match kind {
            shared::jungle::JungleCampKind::Skirmisher => NeutralCampType::Skirmisher,
            shared::jungle::JungleCampKind::Bruiser => NeutralCampType::Bruiser,
            shared::jungle::JungleCampKind::Spitter => NeutralCampType::Spitter,
        };
        assert_eq!(neutral.state.camp_type, expected_kind);
        assert!(navigation.point_clear(point));
        let mirror = camps.values().find(|other| {
            other.anchor.x == -neutral.anchor.x && other.anchor.z == -neutral.anchor.z
        });
        assert_eq!(mirror.unwrap().state.camp_type, expected_kind);
        for base in [layout.home, layout.away] {
            let start = [base.x, base.z];
            let route = navigation.plan_route(start, point, &[]).unwrap();
            let end = route.last().copied().unwrap_or(start);
            assert!((end[0] - point[0]).hypot(end[1] - point[1]) < 0.1);
        }
    }
}

#[test]
fn each_camp_pays_once_and_respawns_exactly_at_forty_seconds_only_when_running() {
    let now = Instant::now();
    for id in 9_001..=9_006 {
        let (mut players, addr) = player_fixture(HeroClass::Warrior, now);
        let killer = players[&addr].state.id;
        let mut camps = camps();
        let kind = camps[&id].state.camp_type;
        let template = neutral_template(kind);
        let mut buffs = TeamBuffs::default();
        for _ in 0..2 {
            apply_neutral_damage(&mut players, &mut camps, &mut buffs, id, 999.0, killer, now);
        }
        assert_eq!(
            players[&addr].state.gold,
            STARTING_GOLD + template.kill_gold
        );
        assert_eq!(players[&addr].state.xp, template.kill_xp);
        let deadline = now + Duration::from_secs(40);
        assert_eq!(camps[&id].dead_until, Some(deadline));
        assert!(camps[&id].target_player_id.is_none());
        simulate_neutrals(
            &mut players,
            &mut camps,
            &GameState::Running,
            0.1,
            deadline - Duration::from_nanos(1),
        );
        assert_eq!(camps[&id].state.hp, 0.0);
        for state in [
            GameState::Lobby,
            GameState::Victory {
                winner: Team::Green,
            },
        ] {
            simulate_neutrals(&mut players, &mut camps, &state, 0.1, deadline);
            assert_eq!(camps[&id].state.hp, 0.0);
        }
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, deadline);
        assert_reset(&camps[&id]);
        assert!(camps[&id].dead_until.is_none());
        apply_neutral_damage(
            &mut players,
            &mut camps,
            &mut buffs,
            id,
            999.0,
            killer,
            deadline,
        );
        assert_eq!(
            players[&addr].state.gold,
            STARTING_GOLD + 2 * template.kill_gold
        );
        assert_eq!(players[&addr].state.level, 2);
        assert_eq!(
            players[&addr].state.xp,
            2 * template.kill_xp - LEVEL_XP_THRESHOLDS[0]
        );
    }
}

#[test]
fn aggro_resets_after_target_death_disconnect_leave_or_leash_escape() {
    let now = Instant::now();
    for cause in ["death", "disconnect", "leave", "leash"] {
        let (mut players, addr) = player_fixture(HeroClass::Warrior, now);
        let mut camps = camps();
        let neutral = camps.get_mut(&9_001).unwrap();
        let player = players.get_mut(&addr).unwrap();
        player.state.x = neutral.anchor.x + 2.0;
        player.state.z = neutral.anchor.z;
        neutral.target_player_id = Some(player.state.id);
        neutral.state.ai_state = NeutralAiState::Aggro;
        neutral.state.hp -= 20.0;
        neutral.state.x += 1.0;
        neutral.state.yaw = 1.0;
        neutral.last_attack_at = Some(now);
        match cause {
            "death" => player.state.hp = 0.0,
            "leave" => player.joined = false,
            "leash" => player.state.x = neutral.anchor.x + NEUTRAL_LEASH_DISTANCE + 0.1,
            "disconnect" => {
                players.remove(&addr);
            }
            _ => unreachable!(),
        }
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
        assert_reset(&camps[&9_001]);
    }
}

#[test]
fn ordinary_last_hit_recovery_is_bounded_once_only_and_never_revives_or_heals_boss_killers() {
    let now = Instant::now();
    for (kind, starting_hp, expected_hp) in [
        (NeutralCampType::Skirmisher, 10.0, 30.0),
        (NeutralCampType::Skirmisher, 95.0, 100.0),
        (NeutralCampType::Skirmisher, 0.0, 0.0),
        (NeutralCampType::WendigoBoss, 10.0, 10.0),
    ] {
        let (mut players, addr) = player_fixture(HeroClass::Mage, now);
        let state = &mut players.get_mut(&addr).unwrap().state;
        state.hp = starting_hp;
        // Isolate recovery from the separate HP increase granted by a level-up.
        state.level = MAX_LEVEL;
        state.next_level_xp = 0;
        let killer = state.id;
        let mut camps = camps();
        camps.get_mut(&9_001).unwrap().state.camp_type = kind;
        let mut buffs = TeamBuffs::default();
        for _ in 0..2 {
            apply_neutral_damage(
                &mut players,
                &mut camps,
                &mut buffs,
                9_001,
                999.0,
                killer,
                now,
            );
            assert_eq!(players[&addr].state.hp, expected_hp);
            assert_eq!(
                players[&addr].state.gold,
                STARTING_GOLD + neutral_template(kind).kill_gold
            );
        }
    }
}

#[test]
fn rematch_restores_all_six_camps_and_discards_old_deaths_and_targets() {
    let now = Instant::now();
    let (mut players, _) = player_fixture(HeroClass::Warrior, now);
    let mut camps = camps();
    for neutral in camps.values_mut() {
        neutral.state.hp = 0.0;
        neutral.state.x += 3.0;
        neutral.dead_until = Some(now + Duration::from_secs(40));
        neutral.target_player_id = Some(1);
        neutral.last_attack_at = Some(now);
    }
    let mut game_state = GameState::Victory {
        winner: Team::Green,
    };
    let mut last_wave = now;
    reset_match(
        &mut players,
        &mut HashMap::new(),
        &mut HashMap::new(),
        &mut HashMap::new(),
        &mut camps,
        &mut TeamBuffs::default(),
        &build_map_layout(),
        &mut last_wave,
        &mut game_state,
        now,
    );
    assert!(matches!(game_state, GameState::Lobby));
    assert_eq!(camps.len(), 8);
    for id in 9_001..=9_006 {
        assert_reset(&camps[&id]);
        assert!(camps[&id].dead_until.is_none());
    }
    for boss in camps
        .values()
        .filter(|neutral| neutral.state.camp_type.is_boss())
    {
        assert_eq!(boss.state.hp, 0.0);
        assert!(boss.dead_until.is_none());
    }
}

#[test]
fn ordinary_chase_evades_obstacles_without_tunneling_and_boss_chase_is_unchanged() {
    use shared::navigation::{Bounds, NavigationMap, Obstacle};
    let navigation = NavigationMap::new(
        Bounds {
            min: [-20.0, -20.0],
            max: [20.0, 20.0],
        },
        vec![Obstacle {
            id: "wall".into(),
            kind: "test".into(),
            vertices: vec![[0.0, -3.0], [1.0, -3.0], [1.0, 3.0], [0.0, 3.0]],
        }],
    )
    .unwrap();
    let mut camps = camps();
    let neutral = camps.get_mut(&9_001).unwrap();
    neutral.anchor = Vec3f::new(-4.0, NEUTRAL_SPAWN_HEIGHT, 0.0);
    reset_neutral_at_anchor(neutral);
    neutral.target_player_id = Some(1);
    neutral.state.ai_state = NeutralAiState::Aggro;
    neutral.state.hp -= 10.0;
    chase_neutral(neutral, [-2.0, 0.0], 0.1, &navigation);
    assert!(neutral.state.x > -4.0 && neutral.state.x < -2.0);
    assert!(navigation.point_clear([neutral.state.x, neutral.state.z]));
    // A large step ending beyond the wall must still be rejected by the sweep.
    chase_neutral(neutral, [4.0, 0.0], 10.0, &navigation);
    assert_reset(neutral);
    neutral.state.camp_type = NeutralCampType::WendigoBoss;
    chase_neutral(neutral, [4.0, 0.0], 10.0, &navigation);
    assert_eq!(neutral.state.x, 4.0);
}

#[test]
fn fresh_level_one_heroes_can_clear_three_camps_with_basic_attacks_and_unlocked_skills() {
    for class in [
        HeroClass::Warrior,
        HeroClass::Mage,
        HeroClass::Ranger,
        HeroClass::Cleric,
    ] {
        let start = Instant::now();
        let (mut players, addr) = player_fixture(class, start);
        let mut camps = camps();
        let mut projectiles = HashMap::new();
        let mut minions = HashMap::new();
        let mut structures = HashMap::new();
        let mut buffs = TeamBuffs::default();
        let mut next_projectile = 1;
        let mut tick = 0_u64;
        for id in [9_001, 9_005, 9_003] {
            let anchor = camps[&id].anchor;
            let player = &mut players.get_mut(&addr).unwrap().state;
            player.x = anchor.x + 2.0;
            player.z = anchor.z;
            let target = TargetId {
                kind: TargetKind::Neutral,
                id,
            };
            let fight_start = tick;
            while camps[&id].state.hp > 0.0 && tick - fight_start < 300 {
                tick += 1;
                let now = start + Duration::from_millis(100 * tick);
                let player = &mut players.get_mut(&addr).unwrap().state;
                player.mana = (player.mana + MANA_REGEN_PER_SECOND * 0.1).min(player.max_mana);
                handle_basic_attack_request(
                    &mut players,
                    &mut projectiles,
                    &minions,
                    &structures,
                    &camps,
                    &buffs,
                    addr,
                    target,
                    tick,
                    &mut next_projectile,
                    &GameState::Running,
                    now,
                );
                handle_cast_request(
                    &mut players,
                    &mut projectiles,
                    &mut minions,
                    &mut structures,
                    &mut camps,
                    &buffs,
                    addr,
                    target,
                    0,
                    &mut next_projectile,
                    &GameState::Running,
                    now,
                );
                // The second kill unlocks W. Use each class's recovery skill
                // naturally: mana surge for Mage, a self-heal for the others.
                let player = &players[&addr].state;
                let needs_recovery = if class == HeroClass::Mage {
                    player.mana < player.max_mana * 0.5
                } else {
                    player.hp < player.max_hp * 0.8
                };
                if needs_recovery {
                    handle_cast_request(
                        &mut players,
                        &mut projectiles,
                        &mut minions,
                        &mut structures,
                        &mut camps,
                        &buffs,
                        addr,
                        target,
                        1,
                        &mut next_projectile,
                        &GameState::Running,
                        now,
                    );
                }
                simulate_projectiles(
                    &mut players,
                    &mut minions,
                    &mut structures,
                    &mut camps,
                    &mut buffs,
                    &mut projectiles,
                    &mut GameState::Running,
                    0.1,
                    now,
                );
                simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
                assert!(
                    players[&addr].state.hp > 0.0,
                    "{class:?} died clearing camp {id}"
                );
            }
            assert_eq!(
                camps[&id].state.hp, 0.0,
                "{class:?} failed to kill camp {id}"
            );
        }
        let hero = &players[&addr].state;
        assert_eq!(hero.level, 2, "{class:?}");
        assert_eq!(hero.xp, 100);
        assert_eq!(hero.gold, STARTING_GOLD + 115);
        println!(
            "{class:?} three-camp clear: {:.1} seconds, {:.1}/{:.1} HP",
            tick as f32 * 0.1,
            hero.hp,
            hero.max_hp
        );
    }
}

#[test]
fn orphan_chip_damage_resets_uncontested_camps_but_living_nearby_players_keep_aggro() {
    let now = Instant::now();
    for cause in ["death", "disconnect", "leave"] {
        let (mut players, addr) = player_fixture(HeroClass::Ranger, now);
        let killer = players[&addr].state.id;
        let mut camps = camps();
        let mut buffs = TeamBuffs::default();
        let anchor = camps[&9_001].anchor;
        // The first arrow acquires aggro; target loss then fully evades.
        apply_neutral_damage(
            &mut players,
            &mut camps,
            &mut buffs,
            9_001,
            9.0,
            killer,
            now,
        );
        match cause {
            "death" => players.get_mut(&addr).unwrap().state.hp = 0.0,
            "leave" => players.get_mut(&addr).unwrap().joined = false,
            "disconnect" => {
                players.remove(&addr);
            }
            _ => unreachable!(),
        }
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
        assert_reset(&camps[&9_001]);
        // A second arrow lands after that reset, with its owner now ineligible.
        apply_neutral_damage(
            &mut players,
            &mut camps,
            &mut buffs,
            9_001,
            9.0,
            killer,
            now,
        );
        assert!(camps[&9_001].target_player_id.is_none());
        assert!(camps[&9_001].state.hp < camps[&9_001].state.max_hp);
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
        assert_reset(&camps[&9_001]);

        let (mut new_players, new_addr) = player_fixture(HeroClass::Warrior, now);
        let mut nearby = new_players.remove(&new_addr).unwrap();
        nearby.state.id = killer + 1;
        nearby.state.x = anchor.x + 2.0;
        nearby.state.z = anchor.z;
        players.insert("127.0.0.1:58402".parse().unwrap(), nearby);
        apply_neutral_damage(
            &mut players,
            &mut camps,
            &mut buffs,
            9_001,
            9.0,
            killer,
            now,
        );
        let wounded_hp = camps[&9_001].state.hp;
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
        assert_eq!(camps[&9_001].state.hp, wounded_hp);
        assert_eq!(camps[&9_001].target_player_id, Some(killer + 1));
        assert_eq!(camps[&9_001].state.ai_state, NeutralAiState::Aggro);
    }
}

#[test]
fn lethal_posthumous_neutral_hit_still_pays_once_and_waits_for_respawn() {
    let now = Instant::now();
    let (mut players, addr) = player_fixture(HeroClass::Ranger, now);
    let killer = players[&addr].state.id;
    players.get_mut(&addr).unwrap().state.hp = 0.0;
    let mut camps = camps();
    let mut buffs = TeamBuffs::default();
    for _ in 0..2 {
        apply_neutral_damage(
            &mut players,
            &mut camps,
            &mut buffs,
            9_001,
            999.0,
            killer,
            now,
        );
        simulate_neutrals(&mut players, &mut camps, &GameState::Running, 0.1, now);
    }
    assert_eq!(
        players[&addr].state.gold,
        STARTING_GOLD + SKIRMISHER_KILL_GOLD
    );
    assert_eq!(players[&addr].state.xp, SKIRMISHER_KILL_XP);
    assert_eq!(players[&addr].state.hp, 0.0);
    assert_eq!(camps[&9_001].state.hp, 0.0);
    assert_eq!(
        camps[&9_001].dead_until,
        Some(now + NEUTRAL_RESPAWN_COOLDOWN)
    );
}
