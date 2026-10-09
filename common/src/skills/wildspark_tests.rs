use super::*;

#[test]
fn rocket_acceleration_is_capped_and_tick_partition_independent() {
    let (launch, speed, acceleration) = (18.0, 45.0, 54.0);
    let early = rocket_step(0.0, 0.1, launch, speed, acceleration);
    let later = rocket_step(early, 0.1, launch, speed, acceleration);
    assert!(later > early && early > launch * 0.1);
    let whole = rocket_step(0.0, 2.0, launch, speed, acceleration);
    let mut split = 0.0;
    for _ in 0..40 {
        split += rocket_step(split, 0.05, launch, speed, acceleration);
    }
    assert!((split - whole).abs() < 0.0001);
    assert!((rocket_step(100.0, 0.1, launch, speed, acceleration) - 4.5).abs() < 0.0001);
    for range in [0.01, 3.0, 15.75, 20.0, 256.0] {
        let time = rocket_time(range, launch, speed, acceleration);
        assert!((rocket_step(0.0, time, launch, speed, acceleration) - range).abs() < 0.001);
    }
}

#[test]
fn rocket_can_finish_five_wounded_enemies_with_one_authoritative_blast() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    for n in 2..=6 {
        if n > 2 {
            add_player(&mut w, n, HeroClass::Warrior, Team::Blue, [25.0, 0.0], now);
        }
        let p = w.players.get_mut(&addr(n)).unwrap();
        p.hero.x = 25.0;
        p.hero.z = (n as f32 - 4.0) * 0.6;
        p.hero.hp = 200.0;
    }
    let healthy = add_player(&mut w, 7, HeroClass::Warrior, Team::Blue, [24.0, 2.8], now);
    let outside = add_player(&mut w, 8, HeroClass::Warrior, Team::Blue, [25.0, 8.0], now);
    let ally = add_player(
        &mut w,
        9,
        HeroClass::Warrior,
        Team::Green,
        [25.0, -2.0],
        now,
    );
    w.players.get_mut(&addr(8)).unwrap().hero.hp = 100.0;
    w.players.get_mut(&addr(9)).unwrap().hero.hp = 100.0;
    cast(&mut w, addr(1), 3, [256.0, 0.0], 1, now);
    let hits = advance(&mut w, now, 0.9);
    assert_eq!(hits.iter().filter(|e| e.killed).count(), 5);
    assert_eq!(hits.len(), 6);
    for n in 2..=6 {
        assert_eq!(w.players[&addr(n)].hero.hp, 0.0);
    }
    assert_eq!(w.players[&addr(7)].hero.hp, 916.0);
    assert_eq!(w.players[&addr(8)].hero.hp, 100.0);
    assert_eq!(w.players[&addr(9)].hero.hp, 100.0);
    assert!(hits.iter().any(|e| e.target.id == healthy));
    assert!(
        hits.iter()
            .all(|e| e.target.id != outside && e.target.id != ally)
    );
    let blast = hits[0].area_impact.unwrap();
    assert!(blast.valid());
    assert_eq!(blast.skill, SkillId::WildRocket);
    assert_eq!(blast.radius, 3.0);
    assert!(hits.iter().all(|e| e.area_impact == Some(blast)));
    assert!(advance(&mut w, now + duration(0.9), 0.2).is_empty());
}

#[test]
fn shielded_rocket_has_a_blast_receipt_without_fake_damage() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    shield(&mut w, victim, victim, 1000.0, 10.0, now);
    cast(&mut w, addr(1), 3, [256.0, 0.0], 1, now);
    let hits = advance(&mut w, now, 0.4);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].target.id, victim);
    assert_eq!(hits[0].amount, 0.0);
    assert!(!hits[0].killed);
    assert!(hits[0].area_impact.unwrap().valid());
    assert_eq!(w.players[&addr(2)].hero.hp, 1000.0);
    assert!(effects(&w, now + duration(0.4)).is_empty());
}

#[test]
fn shockline_slows_only_the_first_hostile_target_at_long_range() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 19.0;
    let second = add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [22.0, 0.0], now);
    let ally = add_player(&mut w, 4, HeroClass::Warrior, Team::Green, [12.0, 0.0], now);
    cast(&mut w, addr(1), 1, [24.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 480.0);
    let hits = advance(&mut w, now, 0.9);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].target.id, victim);
    assert!(
        !hits
            .iter()
            .any(|e| e.target.id == second || e.target.id == ally)
    );
    assert!(
        (w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.9))
            - 0.65)
            .abs()
            < 0.001
    );
    assert_eq!(w.players[&addr(3)].hero.hp, 1000.0);
    assert_eq!(w.players[&addr(4)].hero.hp, 1000.0);
}

#[test]
fn visible_blast_survives_hidden_owner_but_hidden_center_is_withheld() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(1)).unwrap().hero.x = 80.0;
    w.players.get_mut(&addr(2)).unwrap().hero.x = 0.0;
    let viewer = &w.players[&addr(2)];
    let event = CombatEvent {
        id: 1,
        source: shared::combat::CombatEntity {
            kind: CombatEntityKind::Player,
            id: 1,
        },
        target: shared::combat::CombatEntity {
            kind: CombatEntityKind::Player,
            id: 2,
        },
        amount: 20.0,
        area_impact: Some(shared::combat::AreaImpact {
            id: 5,
            skill: SkillId::WildRocket,
            center: [0.0, 0.0],
            radius: 3.0,
        }),
        ..Default::default()
    };
    let mut hidden = event.clone();
    hidden.id = 2;
    hidden.area_impact.as_mut().unwrap().center = [100.0, 100.0];
    let mut packet = shared::wire::ServerPacket::Snapshot {
        vision: None,
        sandbox: None,
        debug_access: None,
        match_mode: "dev".into(),
        geometry_id: shared::map::GEOMETRY_ID.into(),
        map_profile: "verdant_default".into(),
        meta: Default::default(),
        join_error: None,
        your_id: viewer.hero.identity.id,
        players: crate::snapshot::build_players_snapshot(&w, Some(viewer.hero.identity.id), now),
        scoreboard: None,
        prematch: None,
        skill_effects: Vec::new(),
        projectiles: Vec::new(),
        combat_events: vec![event, hidden],
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    crate::vision::filter_snapshot(&mut packet, viewer, &w, now);
    let shared::wire::ServerPacket::Snapshot { combat_events, .. } = packet else {
        unreachable!()
    };
    assert_eq!(combat_events.len(), 2);
    assert_eq!(combat_events[0].source.kind, CombatEntityKind::Unknown);
    assert_eq!(combat_events[0].source.id, 0);
    assert!(combat_events[0].area_impact.is_some());
    assert!(combat_events[1].area_impact.is_none());
}
