use super::*;
use crate::balance::PLAYER_GROUND_Y;
use crate::session::handle_transform_request;
use shared::wire::TeamBuffKind;
use shared::{MINION_TARGET_RADIUS, PLAYER_TARGET_RADIUS};

const RAY: u8 = 3;

/// Expected rank-1, level-1 Horizon Ray damage at `lateral` from the axis.
fn ray_damage(lateral: f32, target_radius: f32) -> f32 {
    let SkillEffect::Beam {
        width,
        damage,
        core_width,
        edge_damage_multiplier,
    } = skill(SkillId::DawnRay).effect
    else {
        unreachable!()
    };
    damage
        * shared::loadout::beam_damage_multiplier(
            lateral,
            width,
            target_radius,
            core_width,
            edge_damage_multiplier,
        )
}

fn place(w: &mut GameWorld, n: u16, pos: [f32; 2]) {
    let p = w.players.get_mut(&addr(n)).unwrap();
    p.hero.x = pos[0];
    p.hero.z = pos[1];
}

fn fire_ray(w: &mut GameWorld, now: Instant) -> Vec<CombatEvent> {
    cast(w, addr(1), RAY, [45.0, 0.0], 1, now);
    advance(w, now, 0.95)
}

#[test]
fn ray_burns_hardest_on_its_axis_and_eases_to_the_edge_multiplier_at_the_graze() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let offsets = [0.0, 0.35, 0.8, 1.2, 1.41, -1.44];
    place(&mut w, 2, [20.0, offsets[0]]);
    for (n, z) in (3..).zip(&offsets[1..]) {
        add_player(&mut w, n, HeroClass::Warrior, Team::Blue, [20.0, *z], now);
    }
    let events = fire_ray(&mut w, now);
    assert_eq!(events.len(), 5, "1.44 is outside 0.8 + 0.62 and is missed");
    let mut previous = f32::INFINITY;
    for (n, z) in (2..).zip(&offsets[..5]) {
        let lost = 1000.0 - w.players[&addr(n)].hero.hp;
        let expected = ray_damage(z.abs(), PLAYER_TARGET_RADIUS);
        assert!((lost - expected).abs() < 0.001, "{z}: {lost} vs {expected}");
        assert!(lost <= previous, "damage never rises away from the axis");
        previous = lost;
        let id = w.players[&addr(n)].hero.identity.id;
        let receipt = events.iter().find(|e| e.target.id == id).unwrap();
        assert_eq!(receipt.core_hit, z.abs() <= 0.35, "{z}");
        assert!(!receipt.radiance_burst);
    }
    assert_eq!(1000.0 - w.players[&addr(2)].hero.hp, 80.0);
    assert!((1000.0 - w.players[&addr(6)].hero.hp - 48.0).abs() < 0.05);
    assert_eq!(w.players[&addr(7)].hero.hp, 1000.0);
}

#[test]
fn falloff_uses_each_targets_own_reach_and_never_scales_the_radiance_burst() {
    let (mut w, now, victim) = fixture(HeroClass::Dawnweaver);
    let edge = 0.8 + MINION_TARGET_RADIUS - 0.01;
    let minion = add_minion(&mut w, [12.0, edge]);
    let minion_hp = w.minions[&minion].state.hp;
    place(&mut w, 2, [20.0, 1.2]);
    let owner = w.players[&addr(1)].hero.identity.id;
    w.skill_runtime.marks.insert(
        (key(target(victim)), owner),
        Mark {
            expires: now + duration(6.0),
            amount: 18.0,
        },
    );
    let events = fire_ray(&mut w, now);
    let lost = minion_hp - w.minions[&minion].state.hp;
    assert!((lost - ray_damage(edge, MINION_TARGET_RADIUS)).abs() < 0.001);
    assert!(
        (lost - 48.0).abs() < 0.05,
        "a minion grazing its own reach takes the edge damage"
    );
    let on_victim: Vec<_> = events.iter().filter(|e| e.target.id == victim).collect();
    assert_eq!(on_victim.len(), 2);
    assert!(on_victim[0].radiance_burst && on_victim[0].amount == 18.0);
    assert!(!on_victim[1].radiance_burst && !on_victim[1].core_hit);
    assert!((on_victim[1].amount - ray_damage(1.2, PLAYER_TARGET_RADIUS)).abs() < 0.001);
    let mark = &w.skill_runtime.marks[&(key(target(victim)), owner)];
    assert_eq!(
        mark.amount, 18.0,
        "the re-applied mark is not reduced by the edge"
    );
}

#[test]
fn caster_stands_until_the_ray_fires_then_moves_freely() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), RAY, [45.0, 0.0], 1, now);
    let mid = now + duration(0.4);
    let p = w.players.get_mut(&addr(1)).unwrap();
    assert!(p.hero.skills.cast_locked(mid));
    assert_eq!(p.hero.skills.movement(mid), 0.0);
    assert_eq!(state(p, mid).unwrap().movement_multiplier, 0.0);
    let layout = &w.map_layout;
    let p = w.players.get_mut(&addr(1)).unwrap();
    handle_transform_request(p, layout, 0.3, PLAYER_GROUND_Y, 0.0, 0.0, mid);
    assert_eq!(
        [p.hero.x, p.hero.z],
        [0.0, 0.0],
        "no drift through the slack"
    );
    let after = now + duration(0.81);
    assert!(!p.hero.skills.cast_locked(after));
    assert!(p.hero.skills.movement(after) > 0.0);
    handle_transform_request(p, layout, 0.3, PLAYER_GROUND_Y, 0.0, 0.0, after);
    assert!(p.hero.x > 0.0);
    // A non-beam cast never locks movement.
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    assert!(!w.players[&addr(1)].hero.skills.cast_locked(now));
}

#[test]
fn one_late_tick_still_fires_a_due_ray_exactly_once() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), RAY, [45.0, 0.0], 1, now);
    let late = now + duration(1.2);
    let events = tick(&mut w, TickCtx { now: late, dt: 0.1 });
    assert_eq!(events.len(), 1);
    assert_eq!(w.players[&addr(2)].hero.hp, 920.0);
    let shown: Vec<_> = effects(&w, late)
        .into_iter()
        .filter(|e| e.skill == SkillId::DawnRay)
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        shown,
        [EffectVisualKind::Beam],
        "the late release is still replicated"
    );
    assert!(advance(&mut w, late, 0.5).is_empty());
    assert!(w.skill_runtime.effects.is_empty());
}

#[test]
fn death_during_the_windup_cancels_the_ray_and_its_lock() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), RAY, [45.0, 0.0], 1, now);
    advance(&mut w, now, 0.4);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.hp = 0.0;
    p.hero.skills.transient_reset();
    assert!(!p.hero.skills.cast_locked(now + duration(0.5)));
    assert!(advance(&mut w, now + duration(0.4), 0.6).is_empty());
    assert_eq!(w.players[&addr(2)].hero.hp, 1000.0);
    assert!(w.skill_runtime.effects.is_empty());
}

#[test]
fn a_fully_intercepted_snare_does_not_mark_the_wall_holder() {
    let (mut w, now, victim) = fixture(HeroClass::Dawnweaver);
    place(&mut w, 2, [6.0, 6.0]);
    let holder = add_player(
        &mut w,
        3,
        HeroClass::Frostguard,
        Team::Blue,
        [10.0, 0.0],
        now,
    );
    cast(&mut w, addr(3), 2, [0.0, 0.0], 1, now);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    let events = advance(&mut w, now, 1.0);
    assert!(events.iter().all(|e| e.target.id != victim));
    assert_eq!(
        w.players[&addr(3)].hero.hp,
        1000.0,
        "the first interception blocks it all"
    );
    assert!(
        w.skill_runtime
            .marks
            .keys()
            .all(|(t, _)| *t != key(target(holder))),
        "a blocked spell does not arm Radiance on the holder"
    );
}

#[test]
fn radiance_mark_includes_the_team_damage_buff() {
    let (mut w, now, victim) = fixture(HeroClass::Dawnweaver);
    w.team_buffs
        .grant(Team::Green, TeamBuffKind::DragonAttack, now);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.5);
    let owner = w.players[&addr(1)].hero.identity.id;
    let mark = &w.skill_runtime.marks[&(key(target(victim)), owner)];
    assert!((mark.amount - 18.0 * 1.12).abs() < 0.001);
}
