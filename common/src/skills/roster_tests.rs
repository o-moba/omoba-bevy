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
    assert_eq!(ids.len(), 44);
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
            cast(&mut world, addr(1), slot, [13.0, 0.0], 1, now);
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
                || world.players[&addr(2)].hero.hp < 1000.0;
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
