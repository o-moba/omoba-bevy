//! Outcome tests use the same casts, damage and ticks as live packets.
use super::*;
use shared::loadout::{CoreId, Technique};
fn strike(w: &mut GameWorld, owner: u64, victim: u64, now: Instant) -> Vec<CombatEvent> {
    basic_impact(
        w,
        target(victim),
        10.0,
        source(owner, shared::BASIC_ATTACK_ACTION_SLOT),
        Team::Green,
        0,
        now,
    )
}
#[test]
fn full_roster_resolves_and_each_kit_has_four_distinct_skills() {
    let mut ids = BTreeSet::new();
    for class in HeroClass::ALL.into_iter().filter(|c| c.is_standard()) {
        let kit = shared::loadout::preset_for_class(class).unwrap();
        assert_eq!(kit.core().class(), class);
        for (i, id) in kit.skills().into_iter().enumerate() {
            assert!(ids.insert(id.id()));
            assert_eq!(skill(id).slot.index(), i);
        }
    }
    assert_eq!(ids.len(), SkillId::ALL.len());
    let mut bad = CoreId::Riftshot.preset();
    bad.skills[3] = SkillId::OrbitalCollapse;
    assert!(matches!(
        shared::loadout::resolve(&bad),
        Err(shared::loadout::LoadoutError::RequiresOrbController { .. })
    ));
    bad.skills[0] = SkillId::OrbitalCommand;
    assert!(shared::loadout::resolve(&bad).is_ok());
}
#[test]
fn rift_needle_reduces_cooldowns_only_on_hit_and_builds_hit_stacks() {
    let (mut w, now, _) = fixture(HeroClass::Riftshot);
    cast(&mut w, addr(1), 0, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    let p = &w.players[&addr(1)];
    assert_eq!(p.hero.skills.advanced.stacks, 1);
    assert!(p.timers.last_cast_at[0].unwrap() < now);
    let (mut miss, now, _) = fixture(HeroClass::Riftshot);
    cast(&mut miss, addr(1), 0, [0.0, 20.0], 1, now);
    advance(&mut miss, now, 1.0);
    assert_eq!(miss.players[&addr(1)].hero.skills.advanced.stacks, 0);
    assert_eq!(miss.players[&addr(1)].timers.last_cast_at[0], Some(now));
}
#[test]
fn seal_only_owner_detonates_and_spell_refunds_resource() {
    let (mut w, now, victim) = fixture(HeroClass::Riftshot);
    let owner = w.players[&addr(1)].hero.identity.id;
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [1.0, 0.0], now);
    cast(&mut w, addr(1), 1, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    let events = strike(&mut w, ally, victim, now + duration(0.4));
    assert_eq!(events.iter().map(|e| e.amount).sum::<f32>(), 10.0);
    let events = strike(&mut w, owner, victim, now + duration(0.5));
    assert_eq!(events.iter().map(|e| e.amount).sum::<f32>(), 44.0);
    assert_eq!(strike(&mut w, owner, victim, now + duration(0.6)).len(), 1);
}
fn strike_tower(w: &mut GameWorld, owner: u64, tower: u64, now: Instant) -> f32 {
    basic_impact(
        w,
        TargetId {
            kind: TargetKind::Structure,
            id: tower,
        },
        10.0,
        source(owner, shared::BASIC_ATTACK_ACTION_SLOT),
        Team::Green,
        0,
        now,
    )
    .iter()
    .map(|e| e.amount)
    .sum()
}
#[test]
fn seal_marks_an_unprotected_enemy_structure_and_only_the_owner_attack_detonates_it() {
    let (mut w, now, _) = fixture(HeroClass::Riftshot);
    let owner = w.players[&addr(1)].hero.identity.id;
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [1.0, 0.0], now);
    w.players.get_mut(&addr(2)).unwrap().hero.z = 20.0;
    let tower = add_tower(&mut w, Team::Blue, [10.0, 0.0]);
    let hp = w.structures[&tower].state.hp;
    cast(&mut w, addr(1), 1, [20.0, 0.0], 1, now);
    assert!(advance(&mut w, now, 0.6).is_empty(), "the seal is a mark");
    assert_eq!(w.structures[&tower].state.hp, hp);
    assert!(
        w.skill_runtime.effects.is_empty(),
        "the bolt ends on the structure it marked"
    );
    assert_eq!(strike_tower(&mut w, ally, tower, now + duration(0.7)), 10.0);
    assert_eq!(
        strike_tower(&mut w, owner, tower, now + duration(0.8)),
        44.0
    );
    assert_eq!(
        strike_tower(&mut w, owner, tower, now + duration(0.9)),
        10.0
    );
    assert_eq!(w.structures[&tower].state.hp, hp - 64.0);
}
#[test]
fn seal_stops_at_the_first_structure_while_needle_and_protected_towers_do_not_block() {
    let seal_bonus = |w: &mut GameWorld, now: Instant, victim: u64| {
        let owner = w.players[&addr(1)].hero.identity.id;
        strike(w, owner, victim, now)
            .iter()
            .map(|e| e.amount)
            .sum::<f32>()
            - 10.0
    };
    // An unprotected tower stands between the Riftshot and the hero at [6, 0].
    let (mut w, now, victim) = fixture(HeroClass::Riftshot);
    let owner = w.players[&addr(1)].hero.identity.id;
    let tower = add_tower(&mut w, Team::Blue, [3.0, 0.0]);
    cast(&mut w, addr(1), 1, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.6);
    assert_eq!(seal_bonus(&mut w, now + duration(0.7), victim), 0.0);
    assert_eq!(
        strike_tower(&mut w, owner, tower, now + duration(0.8)),
        44.0
    );

    // A protected tower cannot take the mark, so the seal flies on to the hero.
    let (mut w, now, victim) = fixture(HeroClass::Riftshot);
    let owner = w.players[&addr(1)].hero.identity.id;
    add_tower(&mut w, Team::Blue, [10.0, 30.0]);
    let inner = add_tower(&mut w, Team::Blue, [3.0, 0.0]);
    w.structures.get_mut(&inner).unwrap().state.tier = 1;
    assert!(crate::sim::towers::structure_is_protected(
        &w.structures,
        inner
    ));
    cast(&mut w, addr(1), 1, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.6);
    assert_eq!(seal_bonus(&mut w, now + duration(0.7), victim), 34.0);
    assert_eq!(strike_tower(&mut w, owner, inner, now + duration(0.8)), 0.0);

    // Rift Needle still passes through structures.
    let (mut w, now, _) = fixture(HeroClass::Riftshot);
    let tower = add_tower(&mut w, Team::Blue, [3.0, 0.0]);
    let hp = w.structures[&tower].state.hp;
    cast(&mut w, addr(1), 0, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.6);
    assert!(w.players[&addr(2)].hero.hp < 1000.0);
    assert_eq!(w.structures[&tower].state.hp, hp);
}
#[test]
fn four_allied_hits_stun_once_then_grant_immunity() {
    let (mut w, now, victim) = fixture(HeroClass::Frostguard);
    let owner = w.players[&addr(1)].hero.identity.id;
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [1.0, 0.0], now);
    strike(&mut w, owner, victim, now);
    for i in 1..4 {
        strike(&mut w, ally, victim, now + duration(i as f32 * 0.1));
    }
    assert!(
        remaining(
            w.players[&addr(2)].hero.skills.control.root_until,
            now + duration(0.3)
        ) > 1.0
    );
    let until = w.players[&addr(2)].hero.skills.control.root_until;
    for i in 4..10 {
        strike(&mut w, owner, victim, now + duration(i as f32 * 0.1));
    }
    assert_eq!(w.players[&addr(2)].hero.skills.control.root_until, until);
}
#[test]
fn shield_intercepts_front_once_reduces_later_and_ignores_rear() {
    let (mut w, now, _) = fixture(HeroClass::Frostguard);
    cast(&mut w, addr(1), 2, [10.0, 0.0], 1, now);
    let owner = w.players[&addr(1)].hero.identity.id;
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [4.0, 0.0], [0.0, 0.0], 0.2, now),
        Some((owner, 0.0))
    );
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [4.0, 0.0], [0.0, 0.0], 0.2, now),
        Some((owner, 0.35))
    );
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [-3.0, 0.0], [2.0, 0.0], 0.2, now),
        None
    );
    let before = w.players[&addr(1)].hero.hp;
    raw_damage(
        &mut w,
        target(owner),
        30.0,
        DamageType::Magic,
        source(999, 3),
        Team::Blue,
        now,
    );
    assert!(w.players[&addr(1)].hero.hp < before);
}
#[test]
fn northwall_publishes_its_interception_count() {
    let (mut w, now, _) = fixture(HeroClass::Frostguard);
    let owner = w.players[&addr(1)].hero.identity.id;
    let walls = |w: &GameWorld, at: Instant| -> Vec<u8> {
        effects(w, at)
            .iter()
            .filter(|e| e.kind == EffectVisualKind::ShieldWall)
            .map(|e| e.consumed_segments)
            .collect()
    };
    cast(&mut w, addr(1), 2, [10.0, 0.0], 1, now);
    assert_eq!(walls(&w, now), [0]);
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [4.0, 0.0], [0.0, 0.0], 0.2, now),
        Some((owner, 0.0))
    );
    assert_eq!(walls(&w, now), [1], "the free block is spent");
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [4.0, 0.0], [0.0, 0.0], 0.2, now),
        Some((owner, 0.35))
    );
    assert_eq!(walls(&w, now), [2]);
    // A shot from behind is not intercepted and does not count.
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [-3.0, 0.0], [2.0, 0.0], 0.2, now),
        None
    );
    assert_eq!(walls(&w, now), [2]);
    // The count belongs to the wall: the next one starts with its free block.
    advance(&mut w, now, 3.2);
    assert!(walls(&w, now + duration(3.2)).is_empty());
    let next = now + duration(15.0);
    cast(&mut w, addr(1), 2, [10.0, 0.0], 2, next);
    assert_eq!(walls(&w, next), [0]);
    assert_eq!(
        advanced::intercept(&mut w, Team::Blue, [4.0, 0.0], [0.0, 0.0], 0.2, next),
        Some((owner, 0.0))
    );
}
#[test]
fn sheltering_leap_reaches_the_ally_and_never_doubles_the_self_shield() {
    let shield = |w: &GameWorld, n: u16, at: Instant| {
        state(&w.players[&addr(n)], at).map_or(0.0, |s| s.shield_hp)
    };
    let defense = |w: &GameWorld, n: u16| w.players[&addr(n)].hero.skills.advanced.defense;
    // A real leap: the caster lands on the ally and each of them gets one shield.
    let (mut w, now, _) = fixture(HeroClass::Frostguard);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [8.0, 0.0], now);
    cast(&mut w, addr(1), 1, [8.0, 0.0], 1, now);
    let p = &w.players[&addr(1)].hero;
    assert_eq!([p.x, p.z], [8.0, 0.0]);
    assert_eq!((shield(&w, 1, now), shield(&w, 3, now)), (28.0, 28.0));
    assert_eq!((defense(&w, 1), defense(&w, 3)), (25.0, 25.0));

    // Aimed at its own feet the caster is the picked ally: no move, and still
    // one shield, not both halves of the skill.
    let (mut w, now, _) = fixture(HeroClass::Frostguard);
    cast(&mut w, addr(1), 1, [0.0, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(p.timers.last_cast_at[1], Some(now));
    assert_eq!([p.hero.x, p.hero.z], [0.0, 0.0]);
    assert_eq!(shield(&w, 1, now), 28.0);
    assert_eq!(defense(&w, 1), 25.0);

    // Anchor Step shares the branch and the rule.
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    cast(&mut w, addr(1), 1, [0.0, 0.0], 1, now);
    assert_eq!(shield(&w, 1, now), 25.0);
    assert_eq!(defense(&w, 1), 25.0);
}
#[test]
fn winter_shard_slow_survives_winter_divide_refresh() {
    let (mut w, now, _) = fixture(HeroClass::Frostguard);
    let movement =
        |w: &GameWorld, at: Instant| w.players[&addr(2)].hero.skills.control.movement(at);
    // Winter Divide first: the strip knocks the hero at [6, 0] up for a second.
    cast(&mut w, addr(1), 3, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.5);
    assert_eq!(movement(&w, now + duration(0.5)), 0.0);
    // Winter Shard lands inside the strip: 0.55 for 1.5 s.
    let shard = now + duration(0.5);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 2, shard);
    advance(&mut w, shard, 0.7);
    // Inside the strip the stronger zone slow rules once the knock-up ends.
    assert_eq!(movement(&w, now + duration(1.2)), 0.5);
    // The hero steps out: the strip's slow fades, the shard's keeps running.
    w.players.get_mut(&addr(2)).unwrap().hero.z = 10.0;
    advance(&mut w, now + duration(1.2), 0.4);
    assert_eq!(
        movement(&w, now + duration(1.6)),
        0.55,
        "the strip must not cut the shard's slow short"
    );
    assert_eq!(movement(&w, now + duration(2.4)), 1.0);

    // Orbital Field is the same kind of zone: enemies are slowed while inside it.
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 2.0;
    cast(&mut w, addr(1), 1, [0.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    assert!(w.players[&addr(2)].hero.hp < 1000.0);
    assert_eq!(movement(&w, now + duration(0.3)), 0.6);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 30.0;
    advance(&mut w, now + duration(0.3), 0.3);
    assert_eq!(movement(&w, now + duration(0.6)), 1.0);
}
#[test]
fn parry_blocks_damage_and_control_then_stuns_counter_target() {
    let (mut w, now, victim) = fixture(HeroClass::Edgeweaver);
    let owner = w.players[&addr(1)].hero.identity.id;
    cast(&mut w, addr(1), 1, [12.0, 0.0], 1, now);
    assert!(
        raw_damage(
            &mut w,
            target(owner),
            100.0,
            DamageType::True,
            source(victim, 0),
            Team::Blue,
            now + duration(0.2)
        )
        .is_none()
    );
    let c = candidates(&w)
        .into_iter()
        .find(|c| c.target == target(owner))
        .unwrap();
    control(
        &mut w,
        c,
        victim,
        Team::Blue,
        1.0,
        1.0,
        0.0,
        0.0,
        now + duration(0.2),
    );
    assert!(w.players[&addr(1)].hero.skills.advanced.parried_control);
    assert_eq!(w.players[&addr(1)].hero.skills.control.root_until, None);
    advance(&mut w, now, 0.8);
    assert!(
        remaining(
            w.players[&addr(2)].hero.skills.control.root_until,
            now + duration(0.8)
        ) > 1.0
    );
}
#[test]
fn four_distinct_sides_are_required_and_completion_heals_over_time() {
    let (mut w, now, victim) = fixture(HeroClass::Edgeweaver);
    let owner = w.players[&addr(1)].hero.identity.id;
    cast(&mut w, addr(1), 3, [6.0, 0.0], 1, now);
    for pos in [[8.0, 0.0], [8.0, 0.0], [6.0, 2.0], [4.0, 0.0]] {
        let p = w.players.get_mut(&addr(1)).unwrap();
        p.hero.x = pos[0];
        p.hero.z = pos[1];
        strike(&mut w, owner, victim, now + duration(0.2));
    }
    assert_eq!(
        w.players[&addr(1)]
            .hero
            .skills
            .advanced
            .challenge
            .as_ref()
            .unwrap()
            .sides
            .count_ones(),
        3
    );
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.x = 6.0;
    p.hero.z = -2.0;
    p.hero.hp = 500.0;
    strike(&mut w, owner, victim, now + duration(0.3));
    advance(&mut w, now + duration(0.3), 0.2);
    assert!(w.players[&addr(1)].hero.skills.advanced.challenge.is_none());
    assert!(w.players[&addr(1)].hero.hp > 512.0);
}
#[test]
fn return_orb_has_separate_hit_sets_and_true_return_damage() {
    let (mut w, now, _) = fixture(HeroClass::Emberveil);
    w.players.get_mut(&addr(2)).unwrap().modifiers.resistance = 300.0;
    cast(&mut w, addr(1), 0, [17.0, 0.0], 1, now);
    let events = advance(&mut w, now, 1.7);
    let amounts: Vec<_> = events.iter().map(|e| e.amount).collect();
    assert_eq!(amounts.len(), 2);
    assert!(amounts[1] > amounts[0] * 3.5);
}
#[test]
fn charm_forces_walk_and_rejects_casts_while_active() {
    let (mut w, now, _) = fixture(HeroClass::Emberveil);
    cast(&mut w, addr(1), 2, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.7);
    assert!(w.players[&addr(2)].hero.x < 6.0);
    assert!(w.players[&addr(2)].hero.skills.advanced.charm.is_some());
}
#[test]
fn dash_recasts_are_bounded_and_replay_cannot_move_twice() {
    let (mut w, now, _) = fixture(HeroClass::Emberveil);
    cast(&mut w, addr(1), 3, [2.0, 0.0], 1, now);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 3, [3.0, 0.0], 1, now + duration(0.2));
    assert_eq!(w.players[&addr(1)].hero.x, 2.0);
    cast(&mut w, addr(1), 3, [3.0, 0.0], 2, now + duration(0.3));
    cast(&mut w, addr(1), 3, [4.0, 0.0], 3, now + duration(0.6));
    cast(&mut w, addr(1), 3, [5.0, 0.0], 4, now + duration(0.9));
    assert_eq!(w.players[&addr(1)].hero.x, 4.0);
    assert_eq!(w.players[&addr(1)].hero.mana, mana);
}
#[test]
fn echo_followup_bound_to_hit_target_and_flow_refunds_two_attacks() {
    let (mut w, now, victim) = fixture(HeroClass::Stormfist);
    let owner = w.players[&addr(1)].hero.identity.id;
    assert_eq!(hero_stats::max_mana(&w.players[&addr(1)]), 200.0);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    let first = w.players[&addr(2)].hero.hp;
    cast(&mut w, addr(1), 0, [0.0, 18.0], 2, now + duration(0.4));
    assert!(w.players[&addr(1)].hero.x > 3.0);
    assert!(w.players[&addr(2)].hero.hp < first);
    w.players.get_mut(&addr(1)).unwrap().hero.mana = 50.0;
    for _ in 0..3 {
        strike(&mut w, owner, victim, now + duration(0.6));
    }
    assert_eq!(w.players[&addr(1)].hero.mana, 80.0);
}
/// An ordinary root: it stops movement and leaves casting alone.
fn root(w: &mut GameWorld, n: u16, until: Instant) {
    w.players
        .get_mut(&addr(n))
        .unwrap()
        .hero
        .skills
        .control
        .root_until = Some(until);
}
fn recast_uses(w: &GameWorld, slot: usize) -> u8 {
    w.players[&addr(1)].hero.skills.advanced.recasts[slot]
        .as_ref()
        .map_or(0, |r| r.uses)
}
#[test]
fn rooted_caster_cannot_take_echo_followup_dash_and_keeps_the_recast() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    // The first cast is a projectile and stays legal while rooted.
    root(&mut w, 1, now + duration(0.2));
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].timers.last_cast_at[0], Some(now));
    advance(&mut w, now, 0.3);
    assert_eq!(recast_uses(&w, 0), 1);

    let at = now + duration(0.4);
    root(&mut w, 1, at + duration(1.0));
    let mana = w.players[&addr(1)].hero.mana;
    let hp = w.players[&addr(2)].hero.hp;
    cast(&mut w, addr(1), 0, [18.0, 0.0], 2, at);
    let p = &w.players[&addr(1)];
    assert_eq!(
        [p.hero.x, p.hero.z],
        [0.0, 0.0],
        "a rooted hero does not dash"
    );
    assert_eq!(p.hero.mana, mana);
    assert_eq!(w.players[&addr(2)].hero.hp, hp);
    assert_eq!(recast_uses(&w, 0), 1, "the refused press keeps the recast");
    assert!(state(p, at).unwrap().slots[0].can_recast);

    // The same recast works once the root has ended.
    let free = at + duration(1.1);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 3, free);
    assert!(w.players[&addr(1)].hero.x > 3.0);
    assert!(w.players[&addr(2)].hero.hp < hp);
    assert_eq!(recast_uses(&w, 0), 0);
}
#[test]
fn rooted_caster_keeps_anchor_step_sustain_recast_but_cannot_leap() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    // The leap moves the caster: refused before any cost.
    root(&mut w, 1, now + duration(1.0));
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 1, [5.0, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(p.timers.last_cast_at[1], None);
    assert_eq!(([p.hero.x, p.hero.z], p.hero.mana), ([0.0, 0.0], mana));

    let at = now + duration(1.1);
    cast(&mut w, addr(1), 1, [5.0, 0.0], 2, at);
    assert_eq!(w.players[&addr(1)].hero.x, 5.0);
    assert_eq!(recast_uses(&w, 1), 1);

    // The recast moves nobody, so a root does not block it.
    let later = at + duration(0.5);
    root(&mut w, 1, later + duration(1.0));
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 1, [9.0, 0.0], 3, later);
    let p = &w.players[&addr(1)];
    assert!(remaining(p.hero.skills.advanced.sustain_until, later) > 2.9);
    assert_eq!(p.hero.mana, mana - 25.0);
    assert_eq!(p.hero.x, 5.0);
    assert_eq!(recast_uses(&w, 1), 0);
}
#[test]
fn rooted_caster_cannot_ride_hook_followup_and_keeps_the_use() {
    let (mut w, now, _) = fixture(HeroClass::Chainkeeper);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.4);
    assert_eq!(recast_uses(&w, 0), 1);

    let at = now + duration(0.5);
    root(&mut w, 1, at + duration(0.5));
    cast(&mut w, addr(1), 0, [18.0, 0.0], 2, at);
    let p = &w.players[&addr(1)];
    assert_eq!(
        [p.hero.x, p.hero.z],
        [0.0, 0.0],
        "a rooted hero does not ride the chain"
    );
    assert_eq!(recast_uses(&w, 0), 1);

    let free = at + duration(0.6);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 3, free);
    let hooked = w.players[&addr(2)].hero.x;
    let p = &w.players[&addr(1)];
    assert!((p.hero.x - (hooked - 1.0)).abs() < 0.001);
    assert_eq!(recast_uses(&w, 0), 0);
}
#[test]
fn orb_travels_attaches_and_leashes_instead_of_duplicating() {
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [10.0, 0.0], now);
    cast(&mut w, addr(1), 0, [12.0, 0.0], 1, now);
    advance(&mut w, now, 0.4);
    assert!(w.players[&addr(2)].hero.hp < 1000.0);
    cast(&mut w, addr(1), 2, [10.0, 0.0], 2, now + duration(0.5));
    advance(&mut w, now + duration(0.5), 0.3);
    assert_eq!(
        w.players[&addr(1)]
            .hero
            .skills
            .advanced
            .orb
            .as_ref()
            .unwrap()
            .attached,
        Some(ally)
    );
    assert!(!w.players[&addr(3)].hero.skills.shields.is_empty());
    w.players.get_mut(&addr(3)).unwrap().hero.x = 60.0;
    advance(&mut w, now + duration(0.8), 3.5);
    assert!(
        distance(
            w.players[&addr(1)]
                .hero
                .skills
                .advanced
                .orb
                .as_ref()
                .unwrap()
                .pos,
            [0.0, 0.0]
        ) < 1.0
    );
}
#[test]
fn orb_leash_return_hits_each_enemy_once() {
    let (mut w, now, victim) = fixture(HeroClass::Orbitwright);
    // The orb flies out over the hero at [6, 0] and parks at [12, 0].
    cast(&mut w, addr(1), 0, [12.0, 0.0], 1, now);
    let out = advance(&mut w, now, 1.0);
    assert_eq!(out.iter().filter(|e| e.target.id == victim).count(), 1);
    // Its owner is carried far away (a recall does this); the leash pulls the
    // orb back along the same line, over the same hero.
    w.players.get_mut(&addr(1)).unwrap().hero.x = -30.0;
    let back = advance(&mut w, now + duration(1.0), 3.0);
    assert_eq!(
        back.iter().filter(|e| e.target.id == victim).count(),
        1,
        "one return flight is one hit per enemy"
    );
    let orb = w.players[&addr(1)]
        .hero
        .skills
        .advanced
        .orb
        .as_ref()
        .unwrap();
    assert!(distance(orb.pos, [-30.0, 0.0]) < 0.01);
}
#[test]
fn orbital_field_haste_never_shortens_a_longer_haste() {
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [2.0, 0.0], now);
    // The ally already carries a two-second haste from another source.
    let long = now + duration(2.0);
    w.players
        .get_mut(&addr(3))
        .unwrap()
        .hero
        .skills
        .advanced
        .speed_until = Some(long);
    cast(&mut w, addr(1), 1, [0.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    let at = now + duration(0.3);
    assert_eq!(
        w.players[&addr(3)].hero.skills.advanced.speed_until,
        Some(long)
    );
    // An ally without one is hastened for as long as it stands in the field.
    assert!(remaining(w.players[&addr(1)].hero.skills.advanced.speed_until, at) > 0.0);
    // Stepping out keeps the rest of the longer haste.
    w.players.get_mut(&addr(3)).unwrap().hero.x = 30.0;
    advance(&mut w, at, 0.5);
    assert!(
        w.players[&addr(3)]
            .hero
            .skills
            .movement(now + duration(0.8))
            > 1.0
    );
}
#[test]
fn orb_guard_does_not_replace_a_stronger_active_defense() {
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    add_player(
        &mut w,
        3,
        HeroClass::Frostguard,
        Team::Green,
        [5.0, 0.0],
        now,
    );
    // The orb at home guards its owner with the aura.
    advance(&mut w, now, 0.1);
    let holder = |w: &GameWorld| w.players[&addr(1)].hero.skills.advanced.clone();
    assert_eq!(holder(&w).defense, 15.0);
    // A Frostguard leaps onto her: 25 defense for three seconds.
    let leap = now + duration(0.1);
    cast(&mut w, addr(3), 1, [0.0, 0.0], 1, leap);
    assert_eq!(holder(&w).defense, 25.0);
    advance(&mut w, leap, 0.5);
    let s = holder(&w);
    assert_eq!(s.defense, 25.0, "the aura must not weaken its holder");
    assert!(remaining(s.defense_until, leap + duration(0.5)) > 2.4);
    // When the leap's protection runs out the aura takes over again.
    advance(&mut w, leap + duration(0.5), 2.7);
    let s = holder(&w);
    assert_eq!(s.defense, 15.0);
    assert!(remaining(s.defense_until, leap + duration(3.2)) > 0.0);
}
#[test]
fn lantern_requires_allied_nearby_explicit_action_and_cannot_replay() {
    let (mut w, now, _) = fixture(HeroClass::Chainkeeper);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [5.0, 0.0], now);
    cast(&mut w, addr(1), 1, [5.0, 0.0], 1, now);
    let id = *w.skill_runtime.effects.keys().next().unwrap();
    advanced::interact(&mut w, addr(2), id, 1, now);
    assert!(w.skill_runtime.effects.contains_key(&id));
    advanced::interact(&mut w, addr(3), id, 1, now);
    assert!(!w.skill_runtime.effects.contains_key(&id));
    assert!(w.players[&addr(3)].hero.x < 1.0);
    w.players.get_mut(&addr(3)).unwrap().hero.x = 5.0;
    advanced::interact(&mut w, addr(3), id, 1, now);
    assert_eq!(w.players[&addr(3)].hero.x, 5.0);
}
#[test]
fn shroud_waits_after_hostility_and_proximity_reveals() {
    let (mut w, now, _) = fixture(HeroClass::Veilstalker);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.progress.level = 6;
    p.hero.skills.advanced.last_combat = Some(now);
    assert!(!advanced::camouflaged(
        &w.players[&addr(1)],
        now + duration(3.0)
    ));
    assert!(advanced::camouflaged(
        &w.players[&addr(1)],
        now + duration(5.0)
    ));
    let sight = vec![shared::vision::VisionSource {
        position: [8.0, 0.0],
        radius: 20.0,
    }];
    assert!(!crate::vision::player_visible(
        &sight,
        &w.players[&addr(1)],
        now + duration(5.0)
    ));
    let sight = vec![shared::vision::VisionSource {
        position: [3.0, 0.0],
        radius: 20.0,
    }];
    assert!(crate::vision::player_visible(
        &sight,
        &w.players[&addr(1)],
        now + duration(5.0)
    ));
}
#[test]
fn prepared_curse_charms_early_hit_only_slows() {
    for (wait, charmed) in [(0.4, false), (2.2, true)] {
        let (mut w, now, victim) = fixture(HeroClass::Veilstalker);
        let owner = w.players[&addr(1)].hero.identity.id;
        cast(&mut w, addr(1), 1, [6.0, 0.0], 1, now);
        strike(&mut w, owner, victim, now + duration(wait));
        assert_eq!(
            w.players[&addr(2)].hero.skills.advanced.charm.is_some(),
            charmed
        );
    }
}
#[test]
fn pillar_arms_later_blocks_sweeps_and_charge_consumes_it() {
    let (mut w, now, _) = fixture(HeroClass::Cinderforge);
    cast(&mut w, addr(1), 0, [15.0, 0.0], 1, now);
    assert!(advanced::terrain(&w, now).is_empty());
    let at = now + duration(0.8);
    assert_eq!(advanced::terrain(&w, at).len(), 1);
    let clipped =
        shared::navigation::clip_discs([10.0, 0.0], [18.0, 0.0], &advanced::terrain(&w, at));
    assert!(clipped[0] < 15.0);
    w.players.get_mut(&addr(1)).unwrap().hero.x = 10.0;
    cast(&mut w, addr(1), 2, [20.0, 0.0], 2, at);
    assert!(advanced::terrain(&w, at).is_empty());
}
#[test]
fn pillar_stays_replicated_while_it_blocks_after_owner_death() {
    let replicated = |w: &GameWorld, at: Instant| -> Vec<[f32; 2]> {
        effects(w, at)
            .iter()
            .filter(|e| e.skill == SkillId::FaultLine && e.kind == EffectVisualKind::Trap)
            .map(|e| e.position)
            .collect()
    };
    let blocking = |w: &GameWorld, at: Instant| -> Vec<[f32; 2]> {
        advanced::terrain(w, at).iter().map(|d| d.center).collect()
    };
    // The smith dies before the pillar rises, then after it has risen.
    for death in [0.3, 0.8] {
        let (mut w, now, _) = fixture(HeroClass::Cinderforge);
        cast(&mut w, addr(1), 0, [15.0, 0.0], 1, now);
        advance(&mut w, now, death);
        w.players.get_mut(&addr(1)).unwrap().hero.hp = 0.0;
        advance(&mut w, now + duration(death), 1.0);
        let at = now + duration(death + 1.0);
        assert_eq!(blocking(&w, at), [[15.0, 0.0]]);
        assert_eq!(
            replicated(&w, at),
            blocking(&w, at),
            "a pillar that blocks movement stays replicated"
        );
        // Both end together when the pillar's own timer runs out.
        advance(&mut w, at, 4.1 - death);
        let end = now + duration(5.1);
        assert!(blocking(&w, end).is_empty());
        assert!(replicated(&w, end).is_empty());
    }
}
#[test]
fn furnace_breath_telegraph_follows_the_caster_until_the_blast() {
    let (mut w, now, _) = fixture(HeroClass::Cinderforge);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [5.0, 12.0], now);
    let telegraph = |w: &GameWorld, at: Instant| {
        let effects = effects(w, at);
        let e = effects
            .iter()
            .find(|e| e.skill == SkillId::FurnaceBreath)
            .expect("the breath is telegraphed until it fires");
        assert_eq!(e.kind, EffectVisualKind::BeamWarning);
        (e.position, e.end)
    };
    cast(&mut w, addr(1), 1, [7.0, 0.0], 1, now);
    assert_eq!(telegraph(&w, now), ([0.0, 0.0], [7.0, 0.0]));
    // The smith moves during the windup; the hero at [6, 0] stays in the old cone.
    w.players.get_mut(&addr(1)).unwrap().hero.z = 12.0;
    advance(&mut w, now, 0.4);
    assert_eq!(
        telegraph(&w, now + duration(0.4)),
        ([0.0, 12.0], [7.0, 12.0]),
        "the telegraph is drawn where the blast will be resolved"
    );
    let events = advance(&mut w, now + duration(0.4), 0.5);
    assert!(!events.is_empty());
    assert_eq!(w.players[&addr(2)].hero.hp, 1000.0);
    assert!(w.players[&addr(3)].hero.hp < 1000.0);
}
fn add_team_minion(w: &mut GameWorld, team: Team, pos: [f32; 2]) -> u64 {
    let before: BTreeSet<_> = w.minions.keys().copied().collect();
    crate::world::spawn_minion_wave_for_team_lane(
        &w.map_layout,
        &mut w.minions,
        &mut w.next_minion_id,
        team,
        shared::map::Lane::Mid,
    );
    let id = *w.minions.keys().find(|id| !before.contains(id)).unwrap();
    w.minions
        .retain(|key, _| *key == id || before.contains(key));
    let m = w.minions.get_mut(&id).unwrap();
    m.state.x = pos[0];
    m.state.z = pos[1];
    id
}
fn add_tower(w: &mut GameWorld, team: Team, pos: [f32; 2]) -> u64 {
    let mut next = w.structures.keys().max().map_or(1, |id| id + 1);
    let id = next;
    crate::world::add_structure(
        &mut w.structures,
        &mut next,
        shared::wire::StructureKind::Tower,
        crate::entities::StructureRole::LaneTower {
            lane: shared::map::Lane::Mid,
        },
        team,
        crate::entities::Vec3f::new(pos[0], 3.0, pos[1]),
    );
    id
}
#[test]
fn hero_only_picks_skip_a_nearer_non_hero() {
    // Patient Curse: a minion and a tower sit nearer the aim than the hero.
    let (mut w, now, victim) = fixture(HeroClass::Veilstalker);
    let owner = w.players[&addr(1)].hero.identity.id;
    add_team_minion(&mut w, Team::Blue, [7.0, 0.0]);
    add_tower(&mut w, Team::Blue, [7.5, 1.0]);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 1, [7.0, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(p.timers.last_cast_at[1], Some(now));
    assert!(p.hero.mana < mana);
    strike(&mut w, owner, victim, now + duration(2.2));
    assert!(
        w.players[&addr(2)].hero.skills.advanced.charm.is_some(),
        "the ripe curse sits on the hero"
    );

    // Orbital Guard: an allied minion sits nearer the aim than the allied hero.
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [10.0, 0.0], now);
    add_team_minion(&mut w, Team::Green, [11.0, 0.0]);
    cast(&mut w, addr(1), 2, [11.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].timers.last_cast_at[2], Some(now));
    advance(&mut w, now, 0.8);
    let orb = w.players[&addr(1)]
        .hero
        .skills
        .advanced
        .orb
        .as_ref()
        .unwrap();
    assert_eq!(orb.attached, Some(ally));
    assert!(!w.players[&addr(3)].hero.skills.shields.is_empty());

    // With no hero in the circle the cast is still refused before any cost.
    let (mut w, now, _) = fixture(HeroClass::Veilstalker);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 12.0;
    add_team_minion(&mut w, Team::Blue, [7.0, 0.0]);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 1, [7.0, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(p.timers.last_cast_at[1], None);
    assert_eq!(p.hero.mana, mana);
}
#[test]
fn fourfold_duel_acquires_the_hero_when_a_nearer_minion_shares_the_aim_circle() {
    let (mut w, now, victim) = fixture(HeroClass::Edgeweaver);
    add_team_minion(&mut w, Team::Blue, [6.6, 0.0]);
    cast(&mut w, addr(1), 3, [6.6, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(p.timers.last_cast_at[3], Some(now));
    assert_eq!(
        p.hero.skills.advanced.challenge.as_ref().map(|c| c.target),
        Some(target(victim))
    );
    assert_eq!(
        state(p, now).unwrap().challenge_target,
        Some(victim),
        "the duel the caster sees is the one on the hero"
    );
}
#[test]
fn thunder_kick_ignores_structures_and_picks_the_unit_beside_them() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    let tower = add_tower(&mut w, Team::Blue, [4.0, 0.0]);
    let tower_hp = w.structures[&tower].state.hp;
    let p = w.players.get_mut(&addr(2)).unwrap();
    p.hero.x = 3.0;
    p.hero.z = 2.0;
    // Aimed at the tower itself: the hero beside it is the nearest unit.
    cast(&mut w, addr(1), 3, [4.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].timers.last_cast_at[3], Some(now));
    let kicked = &w.players[&addr(2)].hero;
    assert!(kicked.hp < 1000.0);
    assert!(
        distance([kicked.x, kicked.z], [3.0, 2.0]) > 3.0,
        "the kick throws the hero: {:?}",
        [kicked.x, kicked.z]
    );
    assert!(remaining(kicked.skills.control.stun_until, now) > 0.0);
    assert_eq!(w.structures[&tower].state.hp, tower_hp);

    // A lone tower is not a target: the 40 s cooldown stays.
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 30.0;
    let tower = add_tower(&mut w, Team::Blue, [4.0, 0.0]);
    cast(&mut w, addr(1), 3, [4.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].timers.last_cast_at[3], None);
    assert_eq!(w.structures[&tower].state.hp, tower_hp);
    assert!(std::mem::take(&mut w.skill_runtime.pending).is_empty());
}
#[test]
fn invalid_target_nonfinite_out_of_range_casts_leave_pools_and_objects_unchanged() {
    for class in [
        HeroClass::Edgeweaver,
        HeroClass::Stormfist,
        HeroClass::Veilstalker,
        HeroClass::Orbitwright,
        HeroClass::Frostguard,
    ] {
        let (mut w, now, _) = fixture(class);
        let before = w.players[&addr(1)].hero.mana;
        cast(&mut w, addr(1), 1, [f32::NAN, 0.0], 1, now);
        cast(&mut w, addr(1), 1, [5000.0, 5000.0], 2, now);
        assert_eq!(w.players[&addr(1)].hero.mana, before, "{class:?}");
        assert!(w.skill_runtime.effects.is_empty(), "{class:?}");
    }
}
#[test]
fn mixed_recipe_executes_techniques_without_class_switches() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills[0] = SkillId::RiftNeedle;
    recipe.skills[1] = SkillId::MirrorGuard;
    w.players.get_mut(&addr(1)).unwrap().hero.skills.loadout =
        Some(shared::loadout::resolve(&recipe).unwrap());
    cast(&mut w, addr(1), 0, [20.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    assert!(w.players[&addr(2)].hero.hp < 1000.0);
    cast(&mut w, addr(1), 1, [10.0, 0.0], 2, now + duration(0.5));
    assert!(
        w.players[&addr(1)]
            .hero
            .skills
            .advanced
            .immune(now + duration(0.6))
    );
    assert!(matches!(
        skill(SkillId::MirrorGuard).effect,
        SkillEffect::Technique {
            action: Technique::Parry,
            ..
        }
    ));
}

#[test]
fn field_forge_requires_safe_channel_and_one_item_upgrade_is_bounded() {
    let (mut w, now, _) = fixture(HeroClass::Cinderforge);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 80.0;
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.progress.level = 6;
    p.economy
        .inventory
        .push(shared::shop::ItemId::GuardianCrest);
    advance(&mut w, now, 1.0);
    let view = state(&w.players[&addr(1)], now + duration(1.0)).unwrap();
    assert!(view.forge_remaining_secs > 1.5 && view.forge_remaining_secs < 2.5);
    assert!(!view.forge_ready);
    advance(&mut w, now + duration(1.0), 2.5);
    let p = &w.players[&addr(1)];
    assert!(p.hero.skills.advanced.forged);
    assert!(p.hero.skills.advanced.forge_ready);
    let mitigation = hero_stats::mitigate(p, 100.0, false);
    advance(&mut w, now + duration(3.5), 4.0);
    assert_eq!(
        hero_stats::mitigate(&w.players[&addr(1)], 100.0, false),
        mitigation
    );
    cast(&mut w, addr(1), 0, [15.0, 0.0], 1, now + duration(8.0));
    assert!(!w.players[&addr(1)].hero.skills.advanced.forge_ready);
}
#[test]
fn paired_attacks_slow_then_crit_and_expire_after_two() {
    let (mut w, now, victim) = fixture(HeroClass::Edgeweaver);
    let owner = w.players[&addr(1)].hero.identity.id;
    // West side is not this victim's initial vital, so only the active buff applies.
    w.players.get_mut(&addr(1)).unwrap().hero.skills.loadout = Some({
        let mut r = CoreId::Edgeweaver.preset();
        r.passive = PassiveId::Flow;
        shared::loadout::resolve(&r).unwrap()
    });
    cast(&mut w, addr(1), 2, [0.0, 0.0], 1, now);
    assert_eq!(
        strike(&mut w, owner, victim, now + duration(0.2))
            .iter()
            .map(|e| e.amount)
            .sum::<f32>(),
        10.0
    );
    assert!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.3))
            < 1.0
    );
    assert_eq!(
        strike(&mut w, owner, victim, now + duration(0.4))
            .iter()
            .map(|e| e.amount)
            .sum::<f32>(),
        20.0
    );
    assert_eq!(
        strike(&mut w, owner, victim, now + duration(0.6))
            .iter()
            .map(|e| e.amount)
            .sum::<f32>(),
        10.0
    );
}
#[test]
fn thorn_recasts_prioritize_the_mark_and_stop_after_three() {
    let (mut w, now, victim) = fixture(HeroClass::Veilstalker);
    cast(&mut w, addr(1), 0, [13.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [3.0, 1.0], now);
    for i in 0..4 {
        cast(
            &mut w,
            addr(1),
            0,
            [0.0, 13.0],
            2 + i,
            now + duration(0.5 + i as f32 * 0.2),
        );
    }
    assert_eq!(w.players[&addr(3)].hero.hp, 1000.0);
    assert!(w.players[&addr(2)].hero.hp < 910.0);
    assert_eq!(
        w.players[&addr(1)].hero.skills.advanced.recasts[0]
            .as_ref()
            .unwrap()
            .uses,
        0
    );
    assert_eq!(w.players[&addr(2)].hero.identity.id, victim);
}
#[test]
fn new_stuns_disable_real_basic_attack_admission() {
    let (mut w, now, victim) = fixture(HeroClass::Frostguard);
    let owner = w.players[&addr(1)].hero.identity.id;
    w.players.get_mut(&addr(2)).unwrap().hero.x = 2.0;
    for _ in 0..4 {
        strike(&mut w, owner, victim, now);
    }
    crate::basic_attack::handle_basic_attack_request(
        &mut w,
        addr(2),
        target(owner),
        1,
        now + duration(0.1),
    );
    assert!(w.projectiles.is_empty());
    crate::basic_attack::handle_basic_attack_request(
        &mut w,
        addr(2),
        target(owner),
        2,
        now + duration(2.0),
    );
    assert_eq!(w.projectiles.len(), 1);
}
#[test]
fn death_and_actor_reset_remove_orb_and_owned_objects() {
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    cast(&mut w, addr(1), 0, [12.0, 0.0], 1, now);
    advance(&mut w, now, 0.2);
    assert!(
        effects(&w, now)
            .iter()
            .any(|e| e.kind == EffectVisualKind::Orb)
    );
    w.players.get_mut(&addr(1)).unwrap().hero.hp = 0.0;
    normalize(&mut w, now + duration(0.3));
    assert!(
        !effects(&w, now + duration(0.3))
            .iter()
            .any(|e| e.kind == EffectVisualKind::Orb)
    );
    let (mut w, now, _) = fixture(HeroClass::Chainkeeper);
    let id = w.players[&addr(1)].hero.identity.id;
    cast(&mut w, addr(1), 1, [5.0, 0.0], 1, now);
    clear_actor(&mut w, id);
    assert!(effects(&w, now).is_empty());
}

#[test]
fn colossus_redirect_is_nearby_free_once_and_adds_knockup() {
    let (mut w, now, _) = fixture(HeroClass::Cinderforge);
    cast(&mut w, addr(1), 3, [28.0, 0.0], 1, now);
    let mana = w.players[&addr(1)].hero.mana;
    let seq = w.players[&addr(1)].hero.last_action.sequence;
    cast(&mut w, addr(1), 3, [28.0, 0.0], 2, now + duration(0.2));
    assert_eq!(w.players[&addr(1)].hero.last_action.sequence, seq);
    advance(&mut w, now, 2.1);
    cast(&mut w, addr(1), 3, [28.0, 0.0], 3, now + duration(2.1));
    assert_eq!(w.players[&addr(1)].hero.mana, mana);
    advance(&mut w, now + duration(2.1), 0.3);
    assert!(
        remaining(
            w.players[&addr(2)].hero.skills.control.stun_until,
            now + duration(2.4)
        ) > 0.0
    );
    assert_eq!(
        w.players[&addr(1)].hero.skills.advanced.recasts[3]
            .as_ref()
            .unwrap()
            .uses,
        0
    );
}
#[test]
fn brittle_is_consumed_by_a_following_attack_once() {
    let (mut w, now, victim) = fixture(HeroClass::Cinderforge);
    let owner = w.players[&addr(1)].hero.identity.id;
    cast(&mut w, addr(1), 1, [7.0, 0.0], 1, now);
    advance(&mut w, now, 0.9);
    let first = strike(&mut w, owner, victim, now + duration(0.95))
        .iter()
        .map(|e| e.amount)
        .sum::<f32>();
    let second = strike(&mut w, owner, victim, now + duration(1.0))
        .iter()
        .map(|e| e.amount)
        .sum::<f32>();
    assert!(first > second + 60.0);
}
#[test]
fn cage_consumes_contacted_segments_without_repeating_damage() {
    let (mut w, now, _) = fixture(HeroClass::Chainkeeper);
    cast(&mut w, addr(1), 3, [0.0, 0.0], 1, now);
    advance(&mut w, now, 0.2);
    let hp = w.players[&addr(2)].hero.hp;
    assert!(hp < 1000.0);
    assert!(w.skill_runtime.effects.values().any(|e| e.hit_count > 0));
    assert!(
        effects(&w, now + duration(0.2))
            .iter()
            .any(|e| e.kind == shared::loadout::EffectVisualKind::Cage && e.consumed_segments != 0)
    );
    advance(&mut w, now + duration(0.2), 0.8);
    assert_eq!(w.players[&addr(2)].hero.hp, hp);
}
#[test]
fn reveal_pulse_recast_only_slows_original_victims() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 4.0;
    cast(&mut w, addr(1), 2, [0.0, 0.0], 1, now);
    assert!(remaining(w.players[&addr(2)].hero.skills.control.reveal_until, now) > 0.0);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [3.0, 0.0], now);
    cast(&mut w, addr(1), 2, [0.0, 0.0], 2, now + duration(0.2));
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.3)),
        0.5
    );
    assert_eq!(
        w.players[&addr(3)]
            .hero
            .skills
            .control
            .movement(now + duration(0.3)),
        1.0
    );
}
#[test]
fn reveal_pulse_that_hits_nobody_grants_no_recast() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 20.0;
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 2, [0.0, 0.0], 1, now);
    let p = &w.players[&addr(1)];
    assert_eq!(
        p.timers.last_cast_at[2],
        Some(now),
        "the empty pulse is still cast"
    );
    let spent = p.hero.mana;
    assert!(spent < mana);
    assert!(p.hero.skills.advanced.recasts[2].is_none());
    assert!(!state(p, now).unwrap().slots[2].can_recast);
    // A second press is an ordinary cast on cooldown and charges nothing.
    cast(&mut w, addr(1), 2, [0.0, 0.0], 2, now + duration(0.5));
    assert_eq!(w.players[&addr(1)].hero.mana, spent);
}
#[test]
fn thorn_recast_faces_the_unit_it_hits() {
    let (mut w, now, _) = fixture(HeroClass::Veilstalker);
    cast(&mut w, addr(1), 0, [13.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    // The recast is aimed away from the marked hero at [6, 0] and strikes it anyway.
    let hp = w.players[&addr(2)].hero.hp;
    cast(&mut w, addr(1), 0, [0.0, 13.0], 2, now + duration(0.5));
    assert!(w.players[&addr(2)].hero.hp < hp);
    let toward_victim = shared::math::hero_yaw_towards(6.0, 0.0);
    let p = &w.players[&addr(1)];
    assert_eq!(p.hero.yaw, toward_victim);
    assert_eq!(p.hero.last_action.yaw, Some(toward_victim));

    // With nobody in reach the recast strikes nothing and keeps the aimed facing.
    w.players.get_mut(&addr(2)).unwrap().hero.x = 60.0;
    cast(&mut w, addr(1), 0, [0.0, 13.0], 3, now + duration(0.7));
    let toward_aim = shared::math::hero_yaw_towards(0.0, 13.0);
    let p = &w.players[&addr(1)];
    assert_eq!(recast_uses(&w, 0), 1);
    assert_eq!(p.hero.yaw, toward_aim);
    assert_eq!(p.hero.last_action.yaw, Some(toward_aim));
}
#[test]
fn hook_pull_never_carries_the_victim_past_the_caster() {
    // Caught at `from`, held at `to`: two units closer, but never nearer than
    // the one-unit stand-off and never through the caster at the origin.
    for (from, to) in [(6.0, 4.0), (2.5, 1.0), (1.5, 1.0), (0.8, 0.8)] {
        let (mut w, now, _) = fixture(HeroClass::Chainkeeper);
        w.players.get_mut(&addr(2)).unwrap().hero.x = from;
        cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
        advance(&mut w, now, 0.4);
        assert_eq!(recast_uses(&w, 0), 1, "caught from {from}");
        let held = &w.players[&addr(2)].hero;
        assert!(
            (held.x - to).abs() < 0.001 && held.z.abs() < 0.001,
            "caught at {from}, held at {:?}",
            [held.x, held.z]
        );
    }
}
#[test]
fn blink_uses_legal_landing_and_advances_authoritative_movement_sequence() {
    let (mut w, now, _) = fixture(HeroClass::Riftshot);
    let sequence = w.players[&addr(1)].hero.utility.dash_sequence;
    cast(&mut w, addr(1), 2, [3.0, 2.0], 1, now);
    assert_eq!(
        [w.players[&addr(1)].hero.x, w.players[&addr(1)].hero.z],
        [3.0, 2.0]
    );
    assert!(w.players[&addr(1)].hero.utility.dash_sequence > sequence);
}

/// Every public standard slot must produce combat state, beyond paying a cost
/// or recording an accepted request. Named tests above pin each effect's rules.
#[test]
fn every_standard_slot_produces_its_actual_effect_or_status() {
    for class in HeroClass::ALL.into_iter().filter(|c| c.is_standard()) {
        for slot in 0..4 {
            let (mut world, now, _) = fixture(class);
            // Nearby targets exercise targeted, radial and directional skills.
            world.players.get_mut(&addr(1)).unwrap().hero.x = 10.0;
            world.players.get_mut(&addr(2)).unwrap().hero.x = 13.0;
            add_player(
                &mut world,
                3,
                HeroClass::Warrior,
                Team::Green,
                [12.0, 0.0],
                now,
            );
            let before = world.players[&addr(1)].hero.skills.clone();
            let before_pos = [
                world.players[&addr(1)].hero.x,
                world.players[&addr(1)].hero.z,
            ];
            let victim_before = world.players[&addr(2)].hero.skills.clone();
            let definition = world.players[&addr(1)]
                .hero
                .skills
                .loadout
                .unwrap()
                .skill(SkillSlot::from_index(slot).unwrap());
            // Point intent is clamped by the client; melee can reach a target
            // hitbox whose center lies just beyond the allowed aim endpoint.
            let aim_x = if definition.ability.targeting == shared::TargetingMode::Point {
                10.0 + definition.ability.cast_range.min(3.0)
            } else {
                13.0
            };
            cast(&mut world, addr(1), slot, [aim_x, 0.0], 1, now);
            let after = &world.players[&addr(1)].hero;
            let mut a = after.skills.clone();
            let mut b = before;
            // These prove admission only, not that a skill did anything.
            a.request_id = 0;
            b.request_id = 0;
            a.recovery_until = None;
            b.recovery_until = None;
            a.advanced.last_combat = None;
            b.advanced.last_combat = None;
            let immediate = !world.skill_runtime.effects.is_empty()
                || a != b
                || before_pos != [after.x, after.z]
                || world.players[&addr(2)].hero.hp < 1000.0
                || world.players[&addr(2)].hero.skills != victim_before;
            let events = advance(&mut world, now, 1.0);
            let prepared_curse = if class == HeroClass::Veilstalker && slot == 1 {
                advance(&mut world, now, 2.6);
                let owner = world.players[&addr(1)].hero.identity.id;
                let victim = world.players[&addr(2)].hero.identity.id;
                strike(&mut world, owner, victim, now + duration(2.7));
                world.players[&addr(2)].hero.skills.advanced.charm.is_some()
            } else {
                false
            };
            assert!(
                immediate || !events.is_empty() || prepared_curse,
                "{class:?} slot {slot} produced no combat effect"
            );
        }
    }
}

#[test]
fn clockwork_passive_stacks_real_bonus_damage_and_resets_on_expiry() {
    let (mut w, now, victim) = fixture(HeroClass::Orbitwright);
    let owner = w.players[&addr(1)].hero.identity.id;
    let first: f32 = strike(&mut w, owner, victim, now)
        .iter()
        .map(|e| e.amount)
        .sum();
    let second: f32 = strike(&mut w, owner, victim, now + duration(0.2))
        .iter()
        .map(|e| e.amount)
        .sum();
    assert!(second > first);
    assert_eq!(w.players[&addr(1)].hero.skills.advanced.stacks, 2);
    let expired: f32 = strike(&mut w, owner, victim, now + duration(5.0))
        .iter()
        .map(|e| e.amount)
        .sum();
    assert_eq!(expired, first);
    assert_eq!(w.players[&addr(1)].hero.skills.advanced.stacks, 1);
}
#[test]
fn essence_and_souls_passives_consume_actual_lethal_receipts_once() {
    for class in [HeroClass::Emberveil, HeroClass::Chainkeeper] {
        let (mut w, now, victim) = fixture(class);
        let owner = w.players[&addr(1)].hero.identity.id;
        w.players.get_mut(&addr(1)).unwrap().hero.hp = 100.0;
        w.players.get_mut(&addr(2)).unwrap().hero.hp = 1.0;
        let events = strike(&mut w, owner, victim, now);
        assert!(events.iter().any(|e| e.killed));
        observe(&mut w, &events, now);
        observe(&mut w, &events, now);
        if class == HeroClass::Emberveil {
            assert_eq!(w.players[&addr(1)].hero.hp, 140.0);
        } else {
            assert_eq!(
                effects(&w, now)
                    .iter()
                    .filter(|e| e.kind == EffectVisualKind::Soul)
                    .count(),
                1
            );
            w.players.get_mut(&addr(1)).unwrap().hero.x = 6.0;
            advance(&mut w, now, 0.1);
            assert_eq!(w.players[&addr(1)].hero.skills.advanced.souls, 1);
            assert!(
                !effects(&w, now + duration(0.1))
                    .iter()
                    .any(|e| e.kind == EffectVisualKind::Soul)
            );
        }
    }
}
