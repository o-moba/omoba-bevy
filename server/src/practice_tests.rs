use super::*;

fn runtime(size: u32) -> ServerRuntime {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut rt = ServerRuntime::new(
        socket,
        MatchConfig {
            mode: MatchMode::Practice,
            team_size: size,
        },
    );
    // Enabled but never acknowledges allocation: practice must still run.
    rt.career.backend = career_backend::CareerBackend::test_backend(rt.server_epoch);
    rt
}

fn addr(index: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 57000 + index))
}
fn join(session: &str) -> ClientPacket {
    ClientPacket::Join {
        prematch: false,
        team: Team::Green,
        character: CharacterChoice::Cube,
        hero_class: HeroClass::Mage,
        avatar: None,
        sprite_character: None,
        session_id: Some(session.into()),
        passport_ticket: None,
    }
}

#[test]
fn practice_solo_starts_with_labelled_heroes_without_database_ack_or_ranked_credit() {
    assert_eq!(parse_match_mode(Some("practice")), MatchMode::Practice);
    let mut rt = runtime(2);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("solo"), now);
    assert_eq!(rt.game_state, GameState::Running);
    assert_eq!(joined_count(&rt.players), 4);
    assert_eq!(joined_team_counts(&rt.players), (2, 2));
    let snapshot = build_players_snapshot(&rt.players);
    assert_eq!(snapshot.iter().filter(|p| p.is_bot).count(), 3);
    let allocation = rt.career_allocation_for_test().unwrap();
    assert_eq!(allocation.participants.len(), 4);
    assert_eq!(
        allocation.participants.iter().filter(|p| p.is_bot).count(),
        3
    );
    assert!(!allocation.rated);
    assert_eq!(allocation.ruleset, "practice-bots-v1");
    assert!(!rt.career.backend.started(&allocation.result_id));
    rt.game_state = GameState::Victory {
        winner: Team::Green,
    };
    rt.record_match_metrics(now + Duration::from_secs(1));
    let result = rt.career_view(addr(1), now).last_result.unwrap();
    assert!(!result.saved && !result.rated);
    assert!(
        result
            .participants
            .iter()
            .all(|p| p.rating.is_none() && p.progression_xp_gained == 0)
    );
    rt.handle_packet(
        addr(1),
        ClientPacket::RequestRematch,
        now + Duration::from_secs(2),
    );
    assert_eq!(rt.game_state, GameState::Running);
    assert_eq!(rt.match_id, 2);
}

#[test]
fn bot_models_are_distinct_bundled_free_avatars_with_unchanged_sprite_assignments() {
    let mut rt = runtime(5);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("visible-bots"), now);
    let mut slugs = HashSet::new();
    for player in rt.players.values().filter(|p| p.state.is_bot) {
        let slug = player
            .state
            .avatar
            .as_deref()
            .expect("practice bots have a real roster model");
        assert_eq!(shared::normalize_avatar_slug(Some(slug)), Some(slug));
        let avatar = shared::avatar_definition(slug).unwrap();
        assert!(avatar.passport.is_none() && !slug.starts_with("ekza-"));
        assert_eq!(avatar.license, "CC0");
        assert_eq!(
            player.state.sprite_character.as_deref(),
            Some(shared::normalize_sprite_character_id(None))
        );
        slugs.insert(slug.to_owned());
    }
    assert_eq!(
        slugs.len(),
        8,
        "the nine bots use two appearances per class"
    );
    for slug in slugs {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../client/assets/avatars")
            .join(format!("{slug}.glb"));
        assert!(
            path.is_file(),
            "bot model is not bundled: {}",
            path.display()
        );
        let packet = ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Mage,
            avatar: Some(slug),
            sprite_character: None,
            session_id: Some("free-bot-check".into()),
            passport_ticket: None,
        };
        assert!(matches!(
            rt.passport_admissions.begin(addr(90), &packet),
            passport_admission::Admission::Free
        ));
    }
}

#[test]
fn late_human_replaces_bot_at_safe_spawn_without_inheriting_stats_or_identity() {
    let mut rt = runtime(1);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("first"), now);
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    let bot_id = rt.players[&bot_addr].state.id;
    rt.players.get_mut(&bot_addr).unwrap().state.hp = 17.0;
    rt.players.get_mut(&bot_addr).unwrap().state.level = 7;
    award_gold(rt.players.get_mut(&bot_addr).unwrap(), 123);
    let event = CombatEvent {
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: bot_id,
        },
        target: CombatEntity {
            kind: CombatEntityKind::Player,
            id: rt.players[&addr(1)].state.id,
        },
        amount: 9.0,
        ..Default::default()
    };
    rt.combat_log.extend(now, [event]);
    rt.handle_packet(addr(2), join("second"), now + Duration::from_millis(20));
    let human = &rt.players[&addr(2)].state;
    let spawn = spawn_position_for_team(&rt.map_layout, Team::Blue);
    assert!(!human.is_bot && human.id != bot_id);
    assert_eq!(human.team, Team::Blue);
    assert_eq!(
        (human.x, human.z, human.hp, human.level),
        (
            spawn.x,
            spawn.z,
            shared::hero_balance::base_hp(human.hero_class),
            STARTING_LEVEL
        )
    );
    assert_eq!(joined_count(&rt.players), 2);
    let scoreboard = rt.combat_log.ledger.live_scoreboard().unwrap();
    let retired = scoreboard
        .players
        .iter()
        .find(|p| p.player_id == bot_id)
        .unwrap();
    assert_eq!(
        (retired.connected, retired.level, retired.earned_gold),
        (false, 7, 123)
    );
    assert_eq!(scoreboard.players.iter().filter(|p| p.connected).count(), 2);
    let roster = rt.combat_log.ledger.snapshot();
    assert_eq!(
        roster
            .iter()
            .find(|p| p.player_id == bot_id)
            .unwrap()
            .stats
            .damage_to_heroes,
        9.0
    );
    assert_eq!(
        roster
            .iter()
            .find(|p| p.player_id == human.id)
            .unwrap()
            .stats
            .damage_to_heroes,
        0.0
    );
    rt.handle_packet(addr(3), join("third"), now + Duration::from_millis(30));
    assert_eq!(
        rt.players[&addr(3)].join_error,
        Some(shared::protocol::JoinRejection::MatchFull)
    );
    assert!(!rt.players[&addr(3)].joined);
}

#[test]
fn a_deliberate_leave_frees_the_seat_and_lets_the_same_session_pick_again() {
    let mut rt = runtime(2);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("returning"), now);
    assert!(rt.players[&addr(1)].joined);
    let first_class = rt.players[&addr(1)].state.hero_class;
    let first_id = rt.players[&addr(1)].state.id;
    let first_team = rt.players[&addr(1)].state.team;
    let match_id = rt.match_id;
    let enemy_id = rt
        .players
        .values()
        .find(|p| p.joined && p.state.team != first_team)
        .unwrap()
        .state
        .id;
    let player = rt.players.get_mut(&addr(1)).unwrap();
    player.state.level = 4;
    award_gold(player, 100);
    let kill = CombatEvent {
        source: CombatEntity {
            kind: CombatEntityKind::Player,
            id: first_id,
        },
        target: CombatEntity {
            kind: CombatEntityKind::Player,
            id: enemy_id,
        },
        amount: 10.0,
        killed: true,
        ..Default::default()
    };
    rt.combat_log.extend(now, [kill.clone()]);
    rt.checkpoint_career_round(now);

    rt.handle_packet(
        addr(1),
        ClientPacket::Leave,
        now + Duration::from_millis(10),
    );
    let left = &rt.players[&addr(1)];
    // The endpoint survives (menus, career), the seat does not.
    assert!(!left.joined);
    assert!(left.session_id.is_none());
    assert!(rt.disconnected_sessions.is_empty());

    // Same session id, straight away, with another hero: no `SessionActive`,
    // and the new pick is what the server admits.
    let again = ClientPacket::Join {
        prematch: false,
        team: Team::Blue,
        character: CharacterChoice::Cube,
        hero_class: HeroClass::Cleric,
        avatar: None,
        sprite_character: None,
        session_id: Some("returning".into()),
        passport_ticket: None,
    };
    rt.handle_packet(addr(1), again, now + Duration::from_millis(20));
    let back = &rt.players[&addr(1)];
    assert!(back.joined);
    assert!(back.join_error.is_none());
    assert_ne!(back.state.hero_class, first_class);
    assert_eq!(back.state.hero_class, HeroClass::Cleric);
    assert_eq!(rt.match_id, match_id, "guest admission stays in this round");
    let new_id = back.state.id;
    let new_team = back.state.team;
    assert_ne!(new_id, first_id);
    assert_eq!(
        (back.state.level, back.state.earned_gold),
        (STARTING_LEVEL, 0)
    );

    // A projectile fired before leaving keeps its original attribution. New
    // income must advance immediately, not stall behind the old accumulator.
    rt.combat_log.extend(now, [kill]);
    award_gold(rt.players.get_mut(&addr(1)).unwrap(), 1);
    rt.checkpoint_career_round(now);
    let scoreboard = rt.combat_log.ledger.live_scoreboard().unwrap();
    let old = scoreboard
        .players
        .iter()
        .find(|p| p.player_id == first_id)
        .unwrap();
    assert_eq!(old.hero_class, first_class);
    assert_eq!(old.team, first_team);
    assert_eq!(
        (old.connected, old.kills, old.level, old.earned_gold),
        (false, 2, 4, 100)
    );
    let new = scoreboard
        .players
        .iter()
        .find(|p| p.player_id == new_id)
        .unwrap();
    assert_eq!(new.hero_class, HeroClass::Cleric);
    assert_eq!(new.team, new_team);
    assert_eq!(
        (new.connected, new.kills, new.level, new.earned_gold),
        (true, 0, STARTING_LEVEL, 1)
    );

    // A leave from an endpoint that never joined is harmless.
    rt.handle_packet(
        addr(9),
        ClientPacket::Leave,
        now + Duration::from_millis(30),
    );
    assert!(!rt.players.contains_key(&addr(9)) || !rt.players[&addr(9)].joined);
}

#[test]
fn signed_deliberate_leave_cannot_create_a_second_profile_seat_in_the_same_round() {
    let mut rt = runtime(2);
    let now = Instant::now();
    let profile = shared::career::ProfileSummary::new("4".repeat(64), "Round identity".into());
    rt.career
        .backend
        .test_authenticated(addr(1), profile.clone(), "signed-return");
    rt.handle_packet(addr(1), join("signed-return"), now);
    let old_id = rt.players[&addr(1)].state.id;
    let match_id = rt.match_id;
    assert!(rt.players[&addr(1)].joined);

    rt.handle_packet(
        addr(1),
        ClientPacket::Leave,
        now + Duration::from_millis(10),
    );
    assert_ne!(rt.players[&addr(1)].state.id, old_id);
    rt.handle_packet(
        addr(1),
        join("signed-return"),
        now + Duration::from_millis(20),
    );
    assert!(!rt.players[&addr(1)].joined);
    assert_eq!(rt.match_id, match_id);
    assert!(
        rt.career_view(addr(1), now)
            .error
            .unwrap()
            .contains("already participated in the current round")
    );
    let roster = rt.combat_log.ledger.snapshot();
    let identities: Vec<_> = roster
        .iter()
        .filter(|p| p.profile_id.as_ref() == Some(&profile.profile_id))
        .collect();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].player_id, old_id);
    assert!(identities[0].disconnected);
}

#[test]
fn disconnect_replacement_and_reconnect_keep_human_state_without_oversubscribing() {
    let mut rt = runtime(1);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("anchor"), now);
    rt.handle_packet(addr(2), join("reconnect"), now);
    let original_id = rt.players[&addr(2)].state.id;
    let original = rt.players.get_mut(&addr(2)).unwrap();
    original.state.hp = 63.0;
    original.state.level = 6;
    award_gold(original, 101);
    original
        .state
        .inventory
        .push(shared::shop::ItemId::VitalityGem);
    original.state.utility.dash_remaining_secs = 11.0;
    original.state.utility.haste_remaining_secs = 16.0;
    original.state.utility.last_request_id = 7;
    original.state.utility.dash_sequence = 3;
    original.dash_ready_at = Some(now + Duration::from_secs(20));
    original.haste_ready_at = Some(now + Duration::from_secs(25));
    let utility = original.state.utility;
    let dash_ready_at = original.dash_ready_at;
    let haste_ready_at = original.haste_ready_at;
    let later = now + PLAYER_TIMEOUT + Duration::from_millis(1);
    rt.players.get_mut(&addr(1)).unwrap().last_seen = later;
    rt.maintain_roster(later);
    rt.fill_practice_bots(later);
    assert_eq!(rt.players.values().filter(|p| p.state.is_bot).count(), 1);
    let replacement = rt.players.values_mut().find(|p| p.state.is_bot).unwrap();
    let replacement_id = replacement.state.id;
    replacement.state.level = 4;
    award_gold(replacement, 57);
    rt.handle_packet(addr(3), join("reconnect"), later + Duration::from_millis(1));
    let scoreboard = rt.combat_log.ledger.live_scoreboard().unwrap();
    let retired = scoreboard
        .players
        .iter()
        .find(|p| p.player_id == replacement_id)
        .unwrap();
    assert_eq!(
        (retired.connected, retired.level, retired.earned_gold),
        (false, 4, 57)
    );
    assert_eq!(scoreboard.players.iter().filter(|p| p.connected).count(), 2);
    assert_eq!(
        (rt.players[&addr(3)].state.id, rt.players[&addr(3)].state.hp),
        (original_id, 63.0)
    );
    assert_eq!(joined_count(&rt.players), 2);
    assert!(rt.players.values().all(|p| !p.state.is_bot));
    let restored = &rt.players[&addr(3)];
    assert_eq!((restored.state.level, restored.state.earned_gold), (6, 101));
    assert_eq!(
        restored.state.inventory,
        [shared::shop::ItemId::VitalityGem]
    );
    assert_eq!(restored.state.utility, utility);
    assert_eq!(
        (restored.dash_ready_at, restored.haste_ready_at),
        (dash_ready_at, haste_ready_at)
    );
    let row = scoreboard
        .players
        .iter()
        .find(|p| p.player_id == original_id)
        .unwrap();
    assert_eq!((row.connected, row.level, row.earned_gold), (true, 6, 101));
    let expired = later + PLAYER_TIMEOUT + Duration::from_secs(1);
    rt.maintain_roster(expired);
    rt.maintain_roster(expired + EMPTY_ROSTER_GRACE);
    assert_eq!(rt.game_state, GameState::Lobby);
    assert!(rt.players.values().all(|p| !p.state.is_bot));
}

#[test]
fn practice_turnover_rolls_round_before_lifetime_roster_overflow() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("anchor"), now);
    let anchor_id = rt.players[&addr(1)].state.id;
    for index in 2..42 {
        rt.handle_packet(addr(index), join(&format!("turnover-{index}")), now);
        assert!(
            rt.players[&addr(index)].joined,
            "late tester was blocked at turnover {index}"
        );
        assert!(rt.combat_log.ledger.snapshot().len() <= shared::career::MAX_PARTICIPANTS);
        now += PLAYER_TIMEOUT + Duration::from_millis(1);
        rt.players.get_mut(&addr(1)).unwrap().last_seen = now;
        rt.maintain_roster(now);
        rt.fill_practice_bots(now);
        assert_eq!(joined_count(&rt.players), 2);
        assert_eq!(rt.players[&addr(1)].state.id, anchor_id);
    }
    assert!(rt.match_id > 1);
    assert_eq!(rt.game_state, GameState::Running);
    assert!(rt.career_view(addr(1), now).last_result.is_some());
}

#[test]
fn maximum_practice_roster_rollover_reserves_identity_for_the_incoming_human() {
    let mut rt = runtime(16);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("large-anchor"), now);
    assert_eq!(rt.combat_log.ledger.snapshot().len(), 32);
    rt.handle_packet(addr(2), join("large-late"), now + Duration::from_millis(1));
    assert!(rt.players[&addr(2)].joined);
    assert_eq!(rt.match_id, 2);
    let roster = rt.combat_log.ledger.snapshot();
    assert_eq!(roster.len(), 32);
    assert_eq!(roster.iter().filter(|p| !p.is_bot).count(), 2);
    assert!(
        roster
            .iter()
            .any(|p| p.player_id == rt.players[&addr(2)].state.id)
    );
}

#[test]
fn practice_protects_signed_identity_without_allocating_and_marks_late_guest_auth() {
    let mut rt = runtime(1);
    let now = Instant::now();
    let profile = shared::career::ProfileSummary::new("1".repeat(64), "Original identity".into());
    rt.career
        .backend
        .test_authenticated(addr(1), profile.clone(), "signed-practice");
    rt.handle_packet(addr(1), join("signed-practice"), now);
    assert!(rt.career.backend.test_is_playing(addr(1)));
    let allocation = rt.career_allocation_for_test().unwrap();
    assert!(!rt.career.backend.started(&allocation.result_id));
    rt.handle_packet(
        addr(1),
        ClientPacket::Career {
            request: shared::career::CareerRequest::Challenge {
                public_key: "2".repeat(64),
                nickname: "Replacement".into(),
                session_id: "signed-practice".into(),
            },
        },
        now,
    );
    assert_eq!(rt.career.backend.profile(addr(1)), Some(profile.clone()));
    assert_eq!(rt.players[&addr(1)].career_profile, Some(profile));
    rt.handle_packet(addr(2), join("late-guest-auth"), now);
    assert!(
        !rt.career.backend.test_is_playing(addr(2)),
        "no account client yet"
    );
    rt.handle_packet(
        addr(2),
        ClientPacket::Career {
            request: shared::career::CareerRequest::Challenge {
                public_key: "3".repeat(64),
                nickname: "Guest signing in".into(),
                session_id: "late-guest-auth".into(),
            },
        },
        now,
    );
    assert!(
        rt.career.backend.view(addr(2)).challenge.is_some(),
        "first authentication remains available"
    );
    assert!(
        rt.career.backend.test_is_playing(addr(2)),
        "new account client inherits the actor's admission guard before async login"
    );
    assert_eq!(rt.game_state, GameState::Running);
}

#[test]
fn bots_attack_cast_take_real_damage_and_respawn_using_normal_timers() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("target"), now);
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    rt.structures.clear();
    rt.minions.clear();
    let bot_id = rt.players[&bot_addr].state.id;
    for (address, x) in [(addr(1), -8.0), (bot_addr, -4.0)] {
        let p = rt.players.get_mut(&address).unwrap();
        p.state.x = x;
        p.state.z = -8.0;
        p.state.hero_class = HeroClass::Mage;
    }
    now += Duration::from_millis(100);
    rt.simulate_bots(now, 0.1);
    assert_eq!(
        rt.players[&addr(1)].state.hp,
        rt.players[&addr(1)].state.max_hp,
        "damage waits for actual projectile travel"
    );
    assert!(rt.players[&bot_addr].state.mana < MAX_MANA);
    assert_eq!(rt.projectiles.len(), 2, "ordinary basic plus Q");
    assert!(
        rt.projectiles
            .values()
            .all(|p| p.state.owner_id == bot_id && p.state.source_kind == CombatEntityKind::Player)
    );
    let shots = rt.next_projectile_id;
    rt.simulate_bots(now + Duration::from_millis(10), 0.01);
    assert_eq!(
        rt.next_projectile_id, shots,
        "normal cooldown prevents a second strike"
    );
    let receipts = simulate_projectiles(
        &mut rt.players,
        &mut rt.minions,
        &mut rt.structures,
        &mut rt.neutrals,
        &mut rt.team_buffs,
        &mut rt.projectiles,
        &mut rt.game_state,
        0.5,
        now + Duration::from_millis(500),
    );
    rt.combat_log.extend(now, receipts);
    assert!(rt.players[&addr(1)].state.hp < rt.players[&addr(1)].state.max_hp);
    assert!(
        rt.combat_log
            .ledger
            .snapshot()
            .iter()
            .find(|p| p.player_id == bot_id)
            .unwrap()
            .stats
            .damage_to_heroes
            > 0.0
    );
    apply_player_damage(&mut rt.players, bot_id, 999.0, now);
    assert_eq!(rt.players[&bot_addr].state.hp, 0.0);
    rt.simulate_bots(now + Duration::from_secs(1), 0.1);
    assert_eq!(rt.players[&bot_addr].state.hp, 0.0);
    handle_respawns(
        &mut rt.players,
        &rt.structures,
        &rt.map_layout,
        &rt.game_state,
        now + RESPAWN_DELAY,
    );
    let spawn = spawn_position_for_team(&rt.map_layout, Team::Blue);
    assert_eq!(
        (
            rt.players[&bot_addr].state.x,
            rt.players[&bot_addr].state.z,
            rt.players[&bot_addr].state.hp
        ),
        (spawn.x, spawn.z, rt.players[&bot_addr].state.max_hp)
    );
}

#[test]
fn bot_controller_routes_around_real_forest_and_rejects_remote_control() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("forest-target"), now);
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    let nav = shared::navigation::world_navigation();
    let (from, to) = nav
        .obstacles()
        .iter()
        .filter(|o| o.kind == "tree_trunk")
        .find_map(|o| {
            let center = o
                .vertices
                .iter()
                .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]])
                .map(|p| p / o.vertices.len() as f32);
            let from = [center[0] - 4.0, center[1]];
            let to = [center[0] + 4.0, center[1]];
            (nav.point_clear(from)
                && nav.point_clear(to)
                && !nav.segment_clear(from, to)
                && nav.plan_route(from, to, &[]).is_some())
            .then_some((from, to))
        })
        .unwrap();
    rt.structures.clear();
    // A melee bot must walk around the trunk; long reach could shoot from `from`.
    rt.players.get_mut(&bot_addr).unwrap().state.hero_class = HeroClass::Warrior;
    for (address, point) in [(bot_addr, from), (addr(1), to)] {
        let p = rt.players.get_mut(&address).unwrap();
        p.state.x = point[0];
        p.state.z = point[1];
    }
    rt.handle_packet(bot_addr, ClientPacket::SetGodMode { enabled: true }, now);
    assert!(!rt.players[&bot_addr].god_mode);
    rt.handle_packet(
        bot_addr,
        ClientPacket::Transform {
            dash_sequence: 0,
            x: to[0],
            y: PLAYER_GROUND_Y,
            z: to[1],
            yaw: 0.0,
        },
        now,
    );
    assert_eq!(
        [rt.players[&bot_addr].state.x, rt.players[&bot_addr].state.z],
        from
    );
    let mut travelled = 0.0;
    for _ in 0..150 {
        let before = [rt.players[&bot_addr].state.x, rt.players[&bot_addr].state.z];
        now += Duration::from_millis(100);
        rt.simulate_bots(now, 0.1);
        let after = [rt.players[&bot_addr].state.x, rt.players[&bot_addr].state.z];
        assert!(
            nav.segment_clear(before, after),
            "bot crossed forest collision"
        );
        let distance = (after[0] - before[0]).hypot(after[1] - before[1]);
        assert!(distance <= PLAYER_SPEED * 0.1 + 0.001);
        travelled += distance;
        if (after[0] - to[0]).hypot(after[1] - to[1])
            <= shared::basic_attack_for_class(rt.players[&bot_addr].state.hero_class).range
                + PLAYER_HIT_RADIUS
        {
            break;
        }
    }
    let bot = &rt.players[&bot_addr].state;
    assert!(travelled > 1.0);
    assert!(
        (bot.x - to[0]).hypot(bot.z - to[1])
            <= shared::basic_attack_for_class(bot.hero_class).range + PLAYER_HIT_RADIUS + 0.1
    );
}

#[test]
fn bot_pushes_a_real_lane_and_damages_towers_without_crossing_live_structure_discs() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("lane-observer"), now);
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    let nav = shared::navigation::world_navigation();
    let mut damaged_tower = false;
    let mut moved = 0.0;
    for _ in 0..900 {
        let before = rt.players[&bot_addr].state.clone();
        let discs: Vec<_> = rt
            .structures
            .values()
            .filter(|s| s.state.hp > 0.0)
            .map(|s| shared::navigation::Disc {
                center: [s.state.x, s.state.z],
                radius: structure_collision_radius(s.state.kind),
            })
            .collect();
        now += Duration::from_millis(100);
        rt.players.get_mut(&addr(1)).unwrap().last_seen = now;
        rt.simulate_after_mana(now, 0.1);
        let after = &rt.players[&bot_addr].state;
        // Respawn is the ordinary explicit teleport, not a movement segment.
        if before.hp > 0.0 && after.hp > 0.0 {
            assert!(
                nav.segment_clear_with_discs([before.x, before.z], [after.x, after.z], &discs),
                "bot crossed a live tower or forest"
            );
            moved += (after.x - before.x).hypot(after.z - before.z);
        }
        let stats = rt.combat_log.ledger.snapshot();
        if stats
            .iter()
            .any(|p| p.is_bot && p.stats.damage_to_structures > 0.0)
        {
            damaged_tower = true;
            break;
        }
    }
    assert!(moved > 10.0, "bot should leave its base and push the lane");
    assert!(
        damaged_tower,
        "normal bot attacks must actually reach a lane tower"
    );
}

#[test]
fn practice_bots_unstack_spawn_without_moving_human_or_exceeding_speed() {
    let mut rt = runtime(5);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("crowd-observer"), now);
    let human_before = rt.players[&addr(1)].state.clone();
    let nav = shared::navigation::world_navigation();
    for _ in 0..120 {
        let previous: HashMap<_, _> = rt
            .players
            .iter()
            .map(|(addr, p)| (*addr, [p.state.x, p.state.z]))
            .collect();
        now += Duration::from_millis(50);
        rt.simulate_bots(now, 0.05);
        for (address, player) in &rt.players {
            let from = previous[address];
            let to = [player.state.x, player.state.z];
            assert!((to[0] - from[0]).hypot(to[1] - from[1]) <= PLAYER_SPEED * 0.05 + 0.001);
            assert!(nav.segment_clear(from, to));
        }
    }
    let alive: Vec<_> = rt
        .players
        .values()
        .filter(|p| p.joined && p.state.hp > 0.0)
        .collect();
    for (i, a) in alive.iter().enumerate() {
        for b in &alive[i + 1..] {
            assert!(
                (a.state.x - b.state.x).hypot(a.state.z - b.state.z)
                    >= shared::PLAYER_TARGET_RADIUS * 2.0 - 0.05,
                "heroes {} and {} remain overlapped",
                a.state.id,
                b.state.id
            );
        }
    }
    let human = &rt.players[&addr(1)].state;
    assert_eq!((human.x, human.z), (human_before.x, human_before.z));
}

#[test]
fn release_and_development_never_create_bots() {
    for config in [MatchConfig::release(1), MatchConfig::dev()] {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, config);
        let now = Instant::now();
        rt.handle_packet(addr(1), join("human-only"), now);
        rt.fill_practice_bots(now);
        assert!(rt.players.values().all(|p| !p.state.is_bot));
    }
}

fn send_udp(client: &UdpSocket, rt: &mut ServerRuntime, packet: ClientPacket) {
    client
        .send_to(
            &serde_json::to_vec(&packet).unwrap(),
            rt.socket.local_addr().unwrap(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buffer = [0_u8; 4096];
    loop {
        match rt.socket.peek_from(&mut buffer) {
            Ok((_, sender)) => {
                assert_eq!(sender, client.local_addr().unwrap());
                rt.receive_packets();
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "UDP delivery timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("UDP receive readiness failed: {error}"),
        }
    }
}

fn read_snapshot(client: &UdpSocket, rt: &mut ServerRuntime) -> ServerPacket {
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut assembler = shared::transport::SnapshotAssembler::default();
    let now = Instant::now();
    rt.last_snapshot_at = now - SNAPSHOT_INTERVAL;
    rt.simulate_after_mana(now, 0.0);
    let mut buffer = [0_u8; 65_536];
    loop {
        let (size, _) = client.recv_from(&mut buffer).unwrap();
        assert!(size <= shared::transport::MAX_DATAGRAM_BYTES);
        if let Some(bytes) = assembler.push(&buffer[..size], Instant::now()).unwrap() {
            let packet: ServerPacket = serde_json::from_slice(&bytes).unwrap();
            if matches!(packet, ServerPacket::Snapshot { .. }) {
                return packet;
            }
        }
    }
}

#[test]
fn live_udp_practice_solo_and_running_late_join_publish_real_bot_replacement() {
    let mut rt = runtime(1);
    let first = UdpSocket::bind("127.0.0.1:0").unwrap();
    send_udp(
        &first,
        &mut rt,
        ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
    );
    send_udp(&first, &mut rt, join("udp-practice-first"));
    let ServerPacket::Snapshot {
        match_mode,
        meta,
        game_state,
        players,
        scoreboard,
        your_id,
        ..
    } = read_snapshot(&first, &mut rt)
    else {
        unreachable!()
    };
    assert_eq!(match_mode, "practice");
    assert_eq!(game_state, GameState::Running);
    assert_eq!(players.len(), 1, "opponent spawn is outside team sight");
    let scoreboard = scoreboard.unwrap();
    assert_eq!(scoreboard.players.iter().filter(|p| p.connected).count(), 2);
    assert!(players.iter().all(|p| !p.is_bot));
    assert!(!players.iter().find(|p| p.id == your_id).unwrap().is_bot);
    let old_bot = rt
        .players
        .values()
        .find(|p| p.state.is_bot)
        .unwrap()
        .state
        .id;
    assert!(scoreboard.players.iter().any(|p| p.player_id == old_bot));
    let second = UdpSocket::bind("127.0.0.1:0").unwrap();
    send_udp(
        &second,
        &mut rt,
        ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
    );
    send_udp(&second, &mut rt, join("udp-practice-second"));
    let ServerPacket::Snapshot {
        meta: late_meta,
        game_state,
        players,
        scoreboard,
        your_id: second_id,
        ..
    } = read_snapshot(&second, &mut rt)
    else {
        unreachable!()
    };
    assert_eq!(meta.match_id, late_meta.match_id);
    assert_eq!(game_state, GameState::Running);
    assert_eq!(players.len(), 1, "opponent spawn is outside team sight");
    let scoreboard = scoreboard.unwrap();
    assert_eq!(scoreboard.players.iter().filter(|p| p.connected).count(), 2);
    assert!(players.iter().all(|p| !p.is_bot && p.id != old_bot));
    assert_ne!(your_id, second_id);
    assert!(scoreboard.players.iter().any(|p| p.player_id == your_id));
    assert!(
        scoreboard
            .players
            .iter()
            .filter(|p| p.connected)
            .all(|p| p.player_id != old_bot)
    );
    println!(
        "LIVE_UDP_PRACTICE solo_running=true bots=1 late_join_same_round=true bot_identity_replaced=true humans=2"
    );
}

#[test]
fn human_kills_of_practice_bots_count_on_the_live_scoreboard() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("scorer"), now);
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    let bot_id = rt.players[&bot_addr].state.id;
    let human_id = rt.players[&addr(1)].state.id;
    rt.structures.clear();
    rt.minions.clear();
    rt.bots.clear();
    for (address, x) in [(addr(1), -8.0), (bot_addr, -5.0)] {
        let p = rt.players.get_mut(&address).unwrap();
        p.state.x = x;
        p.state.z = -8.0;
    }
    // Attribution is under test, not damage output: leave the bot one hit away.
    rt.players.get_mut(&bot_addr).unwrap().state.hp = 1.0;
    let mut request_id = 0;
    for _ in 0..400 {
        now += Duration::from_millis(50);
        if rt.players[&addr(1)].state.basic_attack_remaining_secs <= 0.0 {
            request_id += 1;
            rt.handle_packet(
                addr(1),
                ClientPacket::BasicAttack {
                    target: TargetId {
                        kind: TargetKind::Player,
                        id: bot_id,
                    },
                    server_epoch: rt.server_epoch,
                    match_id: rt.match_id,
                    request_id,
                },
                now,
            );
        }
        rt.simulate_after_mana(now, 0.05);
        if rt.players[&bot_addr].state.hp <= 0.0 {
            break;
        }
    }
    assert_eq!(rt.players[&bot_addr].state.hp, 0.0, "the bot died");
    let board = rt
        .combat_log
        .ledger
        .live_scoreboard()
        .expect("live scoreboard");
    let row = |id: u64| board.players.iter().find(|p| p.player_id == id).unwrap();
    assert_eq!((row(human_id).kills, row(human_id).deaths), (1, 0));
    assert_eq!((row(bot_id).kills, row(bot_id).deaths), (0, 1));
    let snapshot = build_players_snapshot(&rt.players);
    assert!(snapshot.iter().any(|p| p.id == human_id && !p.is_bot));
}

/// The shipped client joins with `prematch: true`, so the round begins from
/// the draft's loading phase. Kills must still reach the scoreboard there.
#[test]
fn draft_started_practice_round_credits_human_kills() {
    let mut rt = runtime(1);
    let mut now = Instant::now();
    let ClientPacket::Join {
        team,
        character,
        hero_class,
        ..
    } = join("draft")
    else {
        unreachable!()
    };
    rt.handle_packet(
        addr(1),
        ClientPacket::Join {
            prematch: true,
            team,
            character,
            hero_class,
            avatar: None,
            sprite_character: None,
            session_id: Some("draft".into()),
            passport_ticket: None,
        },
        now,
    );
    let prematch = |rt: &ServerRuntime, id, action| ClientPacket::Prematch {
        request: shared::prematch::PrematchRequest {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            generation: rt.prematch_generation_for_test(),
            request_id: id,
            action,
        },
    };
    let lock = prematch(
        &rt,
        1,
        shared::prematch::PrematchAction::Lock { locked: true },
    );
    rt.handle_packet(addr(1), lock, now);
    now += Duration::from_secs(4);
    rt.simulate_after_mana(now, 0.05);
    let loaded = prematch(&rt, 2, shared::prematch::PrematchAction::Loaded);
    rt.handle_packet(addr(1), loaded, now);
    now += Duration::from_millis(50);
    rt.simulate_after_mana(now, 0.05);
    assert_eq!(rt.game_state, GameState::Running, "draft reached the match");
    let bot_addr = *rt.players.iter().find(|(_, p)| p.state.is_bot).unwrap().0;
    let bot_id = rt.players[&bot_addr].state.id;
    let human_id = rt.players[&addr(1)].state.id;
    rt.structures.clear();
    rt.minions.clear();
    rt.bots.clear();
    for (address, x) in [(addr(1), -8.0), (bot_addr, -5.0)] {
        let p = rt.players.get_mut(&address).unwrap();
        p.state.x = x;
        p.state.z = -8.0;
    }
    // Attribution is under test, not damage output: leave the bot one hit away.
    rt.players.get_mut(&bot_addr).unwrap().state.hp = 1.0;
    let mut request_id = 0;
    for _ in 0..400 {
        now += Duration::from_millis(50);
        if rt.players[&addr(1)].state.basic_attack_remaining_secs <= 0.0 {
            request_id += 1;
            rt.handle_packet(
                addr(1),
                ClientPacket::BasicAttack {
                    target: TargetId {
                        kind: TargetKind::Player,
                        id: bot_id,
                    },
                    server_epoch: rt.server_epoch,
                    match_id: rt.match_id,
                    request_id,
                },
                now,
            );
        }
        rt.simulate_after_mana(now, 0.05);
        if rt.players[&bot_addr].state.hp <= 0.0 {
            break;
        }
    }
    let board = rt
        .combat_log
        .ledger
        .live_scoreboard()
        .expect("live scoreboard");
    let row = |id: u64| board.players.iter().find(|p| p.player_id == id).unwrap();
    assert_eq!((row(human_id).kills, row(bot_id).deaths), (1, 1));
}

fn practice_command(command: shared::practice::PracticeCommand) -> ClientPacket {
    ClientPacket::Practice { command }
}

fn bots_of(rt: &ServerRuntime) -> Vec<SocketAddr> {
    let mut bots: Vec<_> = rt
        .players
        .iter()
        .filter(|(_, p)| p.state.is_bot)
        .map(|(a, _)| *a)
        .collect();
    bots.sort_unstable();
    bots
}

#[test]
fn sandbox_dummy_stands_in_front_of_the_human_and_returns_after_respawn() {
    use shared::practice::{MAX_DUMMIES, PracticeCommand};
    let mut rt = runtime(2);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("range"), now);
    assert_eq!(bots_of(&rt).len(), 3);
    rt.handle_packet(addr(1), practice_command(PracticeCommand::SpawnDummy), now);
    let dummies: Vec<_> = bots_of(&rt)
        .into_iter()
        .filter(|a| matches!(rt.bots.kind(*a), Some(bots::BotKind::Dummy { .. })))
        .collect();
    assert_eq!(dummies.len(), 1);
    assert_eq!(bots_of(&rt).len(), 4, "the standard roster stays");
    let dummy = rt.players[&dummies[0]].state.clone();
    let human = &rt.players[&addr(1)].state;
    assert_eq!(dummy.team, Team::Blue);
    assert!(dummy.max_hp > MAX_HP * 2.0);
    let distance = (dummy.x - human.x).hypot(dummy.z - human.z);
    assert!(
        (1.0..=5.0).contains(&distance),
        "dummy stands close in front, got {distance}"
    );
    // Ticks neither move it nor make it shoot, and the roster is not trimmed.
    for _ in 0..40 {
        now += Duration::from_millis(50);
        rt.handle_packet(addr(1), ClientPacket::Ping, now);
        rt.simulate_after_mana(now, 0.05);
    }
    let after = &rt.players[&dummies[0]].state;
    assert_eq!((after.x, after.z), (dummy.x, dummy.z));
    assert!(
        rt.projectiles
            .values()
            .all(|p| p.state.owner_id != dummy.id)
    );
    assert_eq!(bots_of(&rt).len(), 4);
    // Killed dummies respawn at base like everyone, then walk back to their spot.
    apply_player_damage(&mut rt.players, dummy.id, 10_000.0, now);
    assert_eq!(rt.players[&dummies[0]].state.hp, 0.0);
    // Respawn lands at base within a tick; the next bot tick walks it back.
    // Pings keep the otherwise silent test human from timing out meanwhile.
    for _ in 0..(RESPAWN_DELAY.as_millis() / 50 + 4) {
        now += Duration::from_millis(50);
        rt.handle_packet(addr(1), ClientPacket::Ping, now);
        rt.simulate_after_mana(now, 0.05);
    }
    let back = &rt.players[&dummies[0]].state;
    assert_eq!((back.x, back.z), (dummy.x, dummy.z));
    assert_eq!(back.hp, back.max_hp);
    // Extra dummies recycle the oldest one instead of growing without bound.
    for _ in 0..MAX_DUMMIES + 2 {
        rt.handle_packet(addr(1), practice_command(PracticeCommand::SpawnDummy), now);
    }
    let dummies = bots_of(&rt)
        .into_iter()
        .filter(|a| matches!(rt.bots.kind(*a), Some(bots::BotKind::Dummy { .. })))
        .count();
    assert_eq!(dummies, MAX_DUMMIES);
}

#[test]
fn sandbox_clear_duel_and_roster_replace_the_practice_bots() {
    use shared::practice::PracticeCommand;
    let mut rt = runtime(3);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("duelist"), now);
    assert_eq!(bots_of(&rt).len(), 5);
    rt.handle_packet(addr(1), practice_command(PracticeCommand::ClearBots), now);
    assert!(bots_of(&rt).is_empty());
    now += Duration::from_millis(50);
    rt.simulate_after_mana(now, 0.05);
    assert!(
        bots_of(&rt).is_empty(),
        "no automatic refill in the sandbox"
    );

    rt.handle_packet(
        addr(1),
        practice_command(PracticeCommand::StartDuel {
            level: 7,
            gold: 500,
        }),
        now,
    );
    let bots = bots_of(&rt);
    assert_eq!(bots.len(), 1);
    let duelist = &rt.players[&bots[0]].state;
    assert_eq!(duelist.team, Team::Blue);
    assert_eq!(
        duelist.hero_class,
        HeroClass::Mage,
        "mirrors the human's class"
    );
    assert_eq!(duelist.level, 7);
    assert_eq!(duelist.skill_points, 0);
    assert_eq!(
        duelist.ranks,
        [3, 3, 1, 3],
        "ultimate, Q and W ranked with 6 points"
    );
    assert_eq!(duelist.inventory.len(), shared::shop::INVENTORY_CAPACITY);
    assert_eq!(duelist.hp, duelist.max_hp);
    assert!(duelist.max_hp > MAX_HP);
    let spawn = spawn_position_for_team(&rt.map_layout, Team::Blue);
    assert!((duelist.x - spawn.x).hypot(duelist.z - spawn.z) < 1.0);
    assert_eq!(rt.bots.kind(bots[0]), Some(bots::BotKind::Duelist));
    // It walks mid toward the human instead of idling at base.
    for _ in 0..60 {
        now += Duration::from_millis(50);
        rt.simulate_after_mana(now, 0.05);
    }
    let moved = &rt.players[&bots[0]].state;
    assert!(
        (moved.x - spawn.x).hypot(moved.z - spawn.z) > 2.0,
        "duelist advanced"
    );
    assert_eq!(bots_of(&rt).len(), 1);

    rt.handle_packet(addr(1), practice_command(PracticeCommand::Roster), now);
    assert_eq!(bots_of(&rt).len(), 5);
    assert_eq!(joined_team_counts(&rt.players), (3, 3));
    assert!(
        bots_of(&rt)
            .iter()
            .all(|a| rt.bots.kind(*a) == Some(bots::BotKind::Lane))
    );
}

#[test]
fn all_practice_bots_leave_spawn_and_advance_without_nearby_enemies() {
    let mut rt = runtime(5);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("all-lanes-observer"), now);
    for phase in ["initial spawn", "respawn"] {
        let starts: HashMap<_, _> = rt
            .players
            .iter()
            .filter(|(_, p)| p.state.is_bot)
            .map(|(a, p)| (*a, [p.state.x, p.state.z]))
            .collect();
        for _ in 0..400 {
            now += Duration::from_millis(50);
            rt.simulate_bots(now, 0.05);
        }
        for (a, start) in starts {
            let p = &rt.players[&a].state;
            let distance = (p.x - start[0]).hypot(p.z - start[1]);
            assert!(
                distance > 20.0,
                "bot {} {:?} stalled after {phase}: moved {distance}",
                p.id,
                p.team
            );
        }
        // Ordinary death clears the route and ordinary respawn must allow it
        // to leave the same crowded base again with its assigned lane intact.
        let bots: Vec<_> = rt
            .players
            .values()
            .filter(|p| p.state.is_bot)
            .map(|p| p.state.id)
            .collect();
        for id in bots {
            apply_player_damage(&mut rt.players, id, 9999.0, now);
        }
        rt.simulate_bots(now, 0.05);
        now += RESPAWN_DELAY + Duration::from_secs(1);
        handle_respawns(
            &mut rt.players,
            &rt.structures,
            &rt.map_layout,
            &rt.game_state,
            now,
        );
    }
}

#[test]
fn bot_defends_spawn_then_resumes_lane_when_enemy_is_gone() {
    let mut rt = runtime(2);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("defence-observer"), now);
    let bot_addr = *rt
        .players
        .iter()
        .filter(|(_, p)| p.state.is_bot && p.state.team == Team::Blue)
        .min_by_key(|(_, p)| p.state.id)
        .unwrap()
        .0;
    let start = rt.players[&bot_addr].state.clone();
    let human = rt.players.get_mut(&addr(1)).unwrap();
    human.state.x = start.x + 3.0;
    human.state.z = start.z;
    rt.simulate_bots(now, 0.05);
    assert!(
        !rt.projectiles.is_empty()
            || rt.players[&addr(1)].state.hp < rt.players[&addr(1)].state.max_hp,
        "nearby enemy must trigger defence"
    );
    rt.players.get_mut(&addr(1)).unwrap().state.hp = 0.0;
    for _ in 0..400 {
        now += Duration::from_millis(50);
        rt.simulate_bots(now, 0.05);
    }
    let bot = &rt.players[&bot_addr].state;
    assert!(
        (bot.x - start.x).hypot(bot.z - start.z) > 20.0,
        "bot must resume its lane after the nearby enemy disappears"
    );
}

#[test]
fn sandbox_and_god_mode_are_practice_only_and_never_from_bots() {
    use shared::practice::PracticeCommand;
    let mut rt = runtime(2);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("god"), now);
    rt.handle_packet(addr(1), ClientPacket::SetGodMode { enabled: true }, now);
    assert!(rt.players[&addr(1)].god_mode, "practice accepts god mode");
    let bot = bots_of(&rt)[0];
    rt.handle_packet(bot, practice_command(PracticeCommand::ClearBots), now);
    assert_eq!(bots_of(&rt).len(), 3, "bot addresses cannot issue commands");

    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut dev = ServerRuntime::new(
        socket,
        MatchConfig {
            mode: MatchMode::Dev,
            team_size: 2,
        },
    );
    dev.handle_packet(addr(2), join("dev"), now);
    dev.handle_packet(addr(2), practice_command(PracticeCommand::SpawnDummy), now);
    assert!(
        bots_of(&dev).is_empty(),
        "development matches ignore the sandbox"
    );
}

#[test]
fn home_fountain_heals_both_teams_and_bots_but_never_dead_or_outside_players() {
    let mut rt = runtime(5);
    let now = Instant::now();
    rt.handle_packet(addr(1), join("fountain"), now);
    for p in rt.players.values_mut() {
        p.state.hp = 20.0;
    }
    regenerate_base_hp(&mut rt.players, &rt.map_layout, &GameState::Running, 1.0);
    assert!(rt.players.values().all(|p| {
        (p.state.hp - (20.0 + p.state.max_hp * BASE_HEAL_FRACTION_PER_SECOND)).abs() < 0.001
    }));
    regenerate_base_hp(&mut rt.players, &rt.map_layout, &GameState::Running, 100.0);
    assert!(rt.players.values().all(|p| p.state.hp == p.state.max_hp));
    let human = rt.players.get_mut(&addr(1)).unwrap();
    human.state.hp = 20.0;
    human.state.x = rt.map_layout.away.x;
    human.state.z = rt.map_layout.away.z;
    regenerate_base_hp(&mut rt.players, &rt.map_layout, &GameState::Running, 1.0);
    assert_eq!(
        rt.players[&addr(1)].state.hp,
        20.0,
        "enemy base cannot heal"
    );
    for phase in [
        GameState::Lobby,
        GameState::Victory {
            winner: Team::Green,
        },
        GameState::Running,
    ] {
        let p = rt.players.get_mut(&addr(1)).unwrap();
        p.state.x = rt.map_layout.home.x + BASE_HEAL_RADIUS + 0.01;
        p.state.z = rt.map_layout.home.z;
        regenerate_base_hp(&mut rt.players, &rt.map_layout, &phase, 1.0);
        assert_eq!(rt.players[&addr(1)].state.hp, 20.0);
    }
    for (joined, hp, phase) in [
        (true, 0.0, GameState::Running),
        (false, 20.0, GameState::Running),
        (
            true,
            20.0,
            GameState::Victory {
                winner: Team::Green,
            },
        ),
    ] {
        let p = rt.players.get_mut(&addr(1)).unwrap();
        p.joined = joined;
        p.state.hp = hp;
        p.state.x = rt.map_layout.home.x;
        p.state.z = rt.map_layout.home.z;
        regenerate_base_hp(&mut rt.players, &rt.map_layout, &phase, 1.0);
        assert_eq!(rt.players[&addr(1)].state.hp, hp);
    }
}

#[test]
fn live_udp_scoreboard_carries_accepted_kills_deaths_assists() {
    let mut rt = runtime(2);
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    send_udp(
        &client,
        &mut rt,
        ClientPacket::Hello {
            protocol_version: shared::protocol::PROTOCOL_VERSION,
        },
    );
    send_udp(&client, &mut rt, join("scoreboard-observer"));
    let human = rt.players[&client.local_addr().unwrap()].state.clone();
    let ally = rt
        .players
        .values()
        .find(|p| p.state.is_bot && p.state.team == human.team)
        .unwrap()
        .state
        .id;
    let enemy = rt
        .players
        .values()
        .find(|p| p.state.team != human.team)
        .unwrap()
        .state
        .id;
    let now = Instant::now();
    for (attacker, damage) in [(ally, 5.0), (human.id, 999.0)] {
        let receipt = apply_player_damage(&mut rt.players, enemy, damage, now).unwrap();
        rt.combat_log.extend(
            now,
            [HitSource::new(
                CombatEntityKind::Player,
                attacker,
                ProjectileStyle::Standard,
            )
            .annotate(receipt)],
        );
    }
    let ServerPacket::Snapshot {
        scoreboard: Some(board),
        ..
    } = read_snapshot(&client, &mut rt)
    else {
        panic!("active wire snapshot requires a scoreboard");
    };
    let own = board
        .players
        .iter()
        .find(|p| p.player_id == human.id)
        .unwrap();
    assert_eq!((own.kills, own.deaths, own.assists), (1, 0, 0));
    assert_eq!(
        board
            .players
            .iter()
            .find(|p| p.player_id == ally)
            .unwrap()
            .assists,
        1
    );
    assert_eq!(
        board
            .players
            .iter()
            .find(|p| p.player_id == enemy)
            .unwrap()
            .deaths,
        1
    );
    assert_eq!(
        board
            .players
            .iter()
            .filter(|p| p.team == shared::map::Team::Green)
            .map(|p| p.kills)
            .sum::<u32>(),
        1
    );
}

#[test]
fn five_bot_teams_fill_every_role_once_around_the_human_pick() {
    let mut rt = runtime(5);
    rt.handle_packet(addr(1), join("role-fill"), Instant::now());
    for team in [Team::Green, Team::Blue] {
        let mut classes: Vec<_> = rt
            .players
            .values()
            .filter(|p| p.joined && p.state.team == team)
            .map(|p| p.state.hero_class)
            .collect();
        classes.sort_by_key(|class| class.id());
        classes.dedup();
        assert_eq!(classes.len(), 5, "{team:?} repeats a class");
    }
}

#[test]
fn jungle_bots_clear_camps_on_their_own_half_for_warden_rewards() {
    let mut rt = runtime(5);
    let mut now = Instant::now();
    rt.handle_packet(addr(1), join("jungle-observer"), now);
    rt.minions.clear();
    let wardens: Vec<_> = rt
        .players
        .iter()
        .filter(|(_, p)| p.state.is_bot && p.state.hero_class == HeroClass::Warden)
        .map(|(a, _)| *a)
        .collect();
    assert_eq!(wardens.len(), 2, "one jungler per team");
    for _ in 0..(45 * 20) {
        now += Duration::from_millis(50);
        rt.minions.clear();
        rt.simulate_bots(now, 0.05);
        let receipts = simulate_projectiles(
            &mut rt.players,
            &mut rt.minions,
            &mut rt.structures,
            &mut rt.neutrals,
            &mut rt.team_buffs,
            &mut rt.projectiles,
            &mut rt.game_state,
            0.05,
            now,
        );
        rt.combat_log.extend(now, receipts);
        simulate_neutrals(&mut rt.players, &mut rt.neutrals, &rt.game_state, 0.05, now);
    }
    for address in wardens {
        let warden = &rt.players[&address].state;
        let own = spawn_position_for_team(&rt.map_layout, warden.team);
        let enemy_team = match warden.team {
            Team::Green => Team::Blue,
            Team::Blue => Team::Green,
        };
        let enemy = spawn_position_for_team(&rt.map_layout, enemy_team);
        let cleared_own_half = rt.neutrals.values().any(|n| {
            !n.state.camp_type.is_boss()
                && n.dead_until.is_some()
                && (n.anchor.x - own.x).hypot(n.anchor.z - own.z)
                    < (n.anchor.x - enemy.x).hypot(n.anchor.z - enemy.z)
        });
        assert!(
            cleared_own_half,
            "{:?} jungler cleared nothing",
            warden.team
        );
        assert!(warden.gold > shared::shop::STARTING_GOLD && warden.xp > 0);
    }
}
