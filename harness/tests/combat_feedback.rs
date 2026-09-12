//! Real UDP receipts and mixed-wave combat. Hero placement uses the existing
//! explicit dev targeting fixture; the wave scenario uses normal spawn/movement.
use harness::{Bot, Character, HeroClass, ServerPacket, ServerProcess, Team};
use shared::combat::{CombatEntityKind, MinionKind, ProjectileStyle};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

fn snapshot(bot: &mut Bot) -> ServerPacket {
    bot.ping();
    bot.recv_snapshot(Instant::now() + Duration::from_secs(3))
        .expect("live complete snapshot")
}

fn strike(bot: &Bot, packet: &ServerPacket, target: u64, request_id: u64) {
    let meta = packet.meta();
    bot.send_raw(
        serde_json::to_string(&serde_json::json!({
            "type":"basic_attack", "target":{"kind":"player","id":target},
            "server_epoch":meta.server_epoch, "match_id":meta.match_id, "request_id":request_id
        }))
        .unwrap()
        .as_bytes(),
    );
}

#[test]
fn every_class_sends_styled_flight_then_one_confirmed_receipt_and_no_immune_hit() {
    for (class, style, damage) in [
        (HeroClass::Warrior, ProjectileStyle::Crescent, 12.0),
        (HeroClass::Mage, ProjectileStyle::Arcane, 8.0),
        (HeroClass::Ranger, ProjectileStyle::Arrow, 10.0),
        (HeroClass::Cleric, ProjectileStyle::Holy, 8.0),
    ] {
        let server = ServerProcess::spawn_with_env(&[
            ("OMOBA_MATCH_MODE", "dev"),
            ("OMOBA_TARGETING_QA", "1"),
        ]);
        let mut attacker = Bot::connect_framed(server.addr());
        let mut victim = Bot::connect_framed(server.addr());
        attacker.join_with_loadout(Team::Green, Character::Ipfs, class, None);
        victim.join_with_loadout(Team::Blue, Character::Ipfs, HeroClass::Warrior, None);
        let a = attacker.my_id(Duration::from_secs(3));
        let b = victim.my_id(Duration::from_secs(3));
        let mut baseline = snapshot(&mut attacker);
        while baseline.player(b).is_none() {
            baseline = snapshot(&mut attacker);
        }
        assert!(baseline.combat_events().is_empty());
        assert!((baseline.player(a).unwrap().x - baseline.player(b).unwrap().x).abs() <= 4.01);
        strike(&attacker, &baseline, b, 1);
        let started = Instant::now();
        let mut saw_flight = false;
        let mut event_id = None;
        let mut repeated_receipt = false;
        let mut observed = HashSet::new();
        while started.elapsed() < Duration::from_millis(1400) {
            victim.ping();
            let packet = snapshot(&mut attacker);
            for projectile in packet.projectiles().iter().filter(|p| p.owner_id == a) {
                assert_eq!(projectile.source_kind, CombatEntityKind::Player);
                assert_eq!(projectile.style, style);
                assert_eq!(
                    projectile.action_slot,
                    Some(shared::BASIC_ATTACK_ACTION_SLOT)
                );
                assert!(projectile.direction.iter().all(|v| v.is_finite()));
                saw_flight = true;
                assert_eq!(
                    packet.player(b).unwrap().hp,
                    100.0,
                    "flight precedes damage"
                );
            }
            for event in packet.combat_events() {
                assert_eq!(
                    (event.source.kind, event.source.id),
                    (CombatEntityKind::Player, a)
                );
                assert_eq!(
                    (event.target.kind, event.target.id),
                    (CombatEntityKind::Player, b)
                );
                assert_eq!(event.style, style);
                assert_eq!(event.amount, damage);
                assert!(!event.killed);
                repeated_receipt |= !observed.insert(event.id);
                event_id = Some(event.id);
                assert_eq!(packet.player(b).unwrap().hp, 100.0 - damage);
            }
            // Same-round retries must not fabricate a second strike or receipt.
            strike(&attacker, &baseline, b, 1);
        }
        assert!(saw_flight, "{class:?}: no visible authoritative flight");
        assert_eq!(
            observed.len(),
            1,
            "one accepted strike gives one deduplicated event"
        );
        assert!(
            event_id.is_some() && repeated_receipt,
            "recent event survives multiple UDP snapshots"
        );
        victim.set_god_mode(true);
        let mut packet = snapshot(&mut attacker);
        let ready = Instant::now() + Duration::from_secs(3);
        while packet.player(b).unwrap().hp < 100.0 && Instant::now() < ready {
            packet = snapshot(&mut attacker);
        }
        assert_eq!(packet.player(b).unwrap().hp, 100.0);
        assert!(
            packet.combat_events().is_empty(),
            "healing is not damage; old receipt expired"
        );
        strike(&attacker, &packet, b, 2);
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            victim.ping();
            let packet = snapshot(&mut attacker);
            assert!(
                packet.combat_events().is_empty(),
                "god mode cannot emit a hit"
            );
            assert_eq!(packet.player(b).unwrap().hp, 100.0);
        }
        eprintln!(
            "{class:?}: styled flight, one {damage} HP receipt, loss repetition/replay guard and immunity verified"
        );
    }
}

#[test]
fn normal_waves_have_two_melee_one_caster_and_caster_damage_arrives_after_release() {
    let server = ServerProcess::spawn();
    let mut observer = Bot::connect_framed(server.addr());
    observer.join(Team::Green, Character::Ipfs);
    observer.set_god_mode(true);
    let started = Instant::now();
    let mut checked_wave = false;
    let mut kind_by_id = HashMap::new();
    let mut seen_projectiles = HashSet::new();
    let mut first_release = HashMap::new();
    let mut second_release = false;
    let mut confirmed_caster_hit = false;
    while started.elapsed() < Duration::from_secs(65) {
        let packet = snapshot(&mut observer);
        for minion in packet.minions() {
            kind_by_id.insert(minion.id, minion.kind);
        }
        if !checked_wave && packet.minions().len() == 18 {
            for team in [Team::Green, Team::Blue] {
                for lane in ["top", "mid", "bot"] {
                    let group: Vec<_> = packet
                        .minions()
                        .iter()
                        .filter(|m| m.team == Some(team) && m.lane == lane)
                        .collect();
                    assert_eq!(group.len(), 3);
                    assert_eq!(
                        group.iter().filter(|m| m.kind == MinionKind::Melee).count(),
                        2
                    );
                    assert_eq!(
                        group
                            .iter()
                            .filter(|m| m.kind == MinionKind::Caster)
                            .count(),
                        1
                    );
                }
            }
            checked_wave = true;
        }
        for projectile in packet
            .projectiles()
            .iter()
            .filter(|p| p.source_kind == CombatEntityKind::Minion)
        {
            assert_eq!(projectile.style, ProjectileStyle::CasterBolt);
            assert_eq!(
                kind_by_id.get(&projectile.owner_id),
                Some(&MinionKind::Caster)
            );
            if seen_projectiles.insert(projectile.id) {
                let released = first_release
                    .entry(projectile.owner_id)
                    .or_insert((projectile.id, Instant::now()));
                if released.0 != projectile.id {
                    assert!(
                        released.1.elapsed() >= Duration::from_millis(1050),
                        "caster fired before its 1.2s cooldown (snapshot tolerance)"
                    );
                    second_release = true;
                }
            }
        }
        for event in packet.combat_events().iter().filter(|e| {
            e.source.kind == CombatEntityKind::Minion && e.style == ProjectileStyle::CasterBolt
        }) {
            assert_eq!(event.target.kind, CombatEntityKind::Minion);
            assert!(
                event.amount > 0.0 && event.amount <= 7.0,
                "actual HP loss capped by caster damage"
            );
            assert!(
                first_release.contains_key(&event.source.id),
                "caster receipt follows a real observed projectile release"
            );
            confirmed_caster_hit = true;
        }
        if checked_wave && confirmed_caster_hit && second_release {
            eprintln!(
                "normal mixed wave/caster projectile/impact/cooldown verified after {:.2}s",
                started.elapsed().as_secs_f32()
            );
            return;
        }
    }
    panic!(
        "missing live proof: wave={checked_wave}, hit={confirmed_caster_hit}, repeat={second_release}"
    );
}

#[test]
fn legacy_snapshots_have_inert_combat_defaults() {
    let packet: ServerPacket =
        serde_json::from_str(r#"{"type":"snapshot","your_id":1,"minions":[{"id":2}]}"#).unwrap();
    assert!(packet.combat_events().is_empty());
    assert!(packet.projectiles().is_empty());
    assert_eq!(packet.minions()[0].kind, MinionKind::Melee);
    assert_eq!(packet.minions()[0].attack_sequence, 0);
}
