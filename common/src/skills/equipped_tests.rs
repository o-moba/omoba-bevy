//! Behavior regressions are separate from the crowd-control extraction.
use super::*;
use shared::loadout::{CoreId, SkillId};

fn equip(w: &mut GameWorld, n: u16, core: CoreId, skills: [SkillId; 4]) {
    let mut recipe = core.preset();
    recipe.skills = skills;
    w.players.get_mut(&addr(n)).unwrap().hero.skills.loadout =
        Some(shared::loadout::resolve(&recipe).unwrap());
}

fn brittle_and_cross_kit_bluff() -> (GameWorld, Instant, u64) {
    let (mut w, now, victim) = fixture(HeroClass::Cinderforge);
    // Move Furnace Breath from W to R. The bonus receipt must retain R.
    let mut skills = CoreId::Cinderforge.preset().skills;
    skills.swap(1, 3);
    equip(&mut w, 1, CoreId::Cinderforge, skills);
    cast(&mut w, addr(1), 3, [7.0, 0.0], 1, now);
    advance(&mut w, now, 0.9);
    assert!(
        state(&w.players[&addr(2)], now + duration(0.9))
            .unwrap()
            .brittle
    );
    add_player(
        &mut w,
        3,
        HeroClass::Dawnweaver,
        Team::Green,
        [4.0, 0.0],
        now,
    );
    let mut skills = CoreId::Dawnweaver.preset().skills;
    skills[0] = SkillId::DaggerBluff;
    equip(&mut w, 3, CoreId::Dawnweaver, skills);
    (w, now + duration(0.95), victim)
}

#[test]
fn admitted_cross_kit_bluff_consumes_brittle_once_with_original_bound_source() {
    let (mut w, now, victim) = brittle_and_cross_kit_bluff();
    let before = w.players[&addr(2)].hero.hp;
    let owner = w.players[&addr(1)].hero.identity.id;
    crate::recall::start(w.players.get_mut(&addr(2)).unwrap(), now);
    cast(&mut w, addr(3), 0, [6.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(2)].hero.hp, before - 70.0);
    assert!(w.players[&addr(2)].timers.recall.is_none());
    assert!(remaining(w.players[&addr(2)].hero.skills.control.stun_until, now) > 0.0);
    let events = std::mem::take(&mut w.skill_runtime.pending);
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].source.id,
            events[0].target.id,
            events[0].action_slot
        ),
        (owner, victim, Some(3))
    );
    assert_eq!(events[0].amount, 70.0);
    w.players.get_mut(&addr(3)).unwrap().modifiers.no_cooldowns = true;
    cast(&mut w, addr(3), 0, [6.0, 0.0], 2, now + duration(0.1));
    assert!(w.skill_runtime.pending.is_empty());
    assert_eq!(w.players[&addr(2)].hero.hp, before - 70.0);
}

#[test]
fn rejected_bluff_keeps_brittle_recall_and_cast_resources() {
    for condition in 0..7 {
        let (mut w, now, _) = brittle_and_cross_kit_bluff();
        let before = w.players[&addr(2)].hero.hp;
        let victim = w.players.get_mut(&addr(2)).unwrap();
        crate::recall::start(victim, now);
        match condition {
            0 => victim.modifiers.god_mode = true,
            1 => victim.hero.skills.advanced.parry_until = Some(now + duration(1.0)),
            2 => victim.hero.skills.advanced.untargetable_until = Some(now + duration(1.0)),
            3 => victim.hero.skills.advanced.unstoppable_until = Some(now + duration(1.0)),
            4 => victim.hero.hp = 0.0,
            5 => victim.hero.x = 20.0,
            _ => victim.joined = false,
        }
        cast(&mut w, addr(3), 0, [6.0, 0.0], 1, now);
        assert_eq!(
            w.players[&addr(3)].hero.mana,
            500.0,
            "condition {condition}"
        );
        assert!(w.players[&addr(3)].timers.last_cast_at[0].is_none());
        assert!(w.players[&addr(2)].timers.recall.is_some());
        assert!(w.skill_runtime.pending.is_empty());
        let victim = w.players.get_mut(&addr(2)).unwrap();
        victim.modifiers.god_mode = false;
        victim.hero.skills.advanced.parry_until = None;
        victim.hero.skills.advanced.untargetable_until = None;
        victim.hero.skills.advanced.unstoppable_until = None;
        victim.hero.hp = before;
        victim.hero.x = 6.0;
        victim.joined = true;
        cast(&mut w, addr(3), 0, [6.0, 0.0], 2, now + duration(0.1));
        assert_eq!(
            w.players[&addr(2)].hero.hp,
            before - 70.0,
            "condition {condition} retained Brittle"
        );
    }
}

#[test]
fn rejected_root_keeps_brittle_and_parry_records_the_control_attempt() {
    for parry in [false, true] {
        let (mut w, now, victim) = brittle_and_cross_kit_bluff();
        let target = candidates(&w)
            .into_iter()
            .find(|c| c.target.id == victim)
            .unwrap();
        let owner = w.players[&addr(3)].hero.identity.id;
        let before = w.players[&addr(2)].hero.hp;
        let p = w.players.get_mut(&addr(2)).unwrap();
        if parry {
            p.hero.skills.advanced.parry_until = Some(now + duration(1.0));
        } else {
            p.modifiers.god_mode = true;
        }
        control(&mut w, target, owner, Team::Green, 1.0, 1.0, 0.0, 0.0, now);
        assert!(w.players[&addr(2)].hero.skills.control.root_until.is_none());
        assert!(w.skill_runtime.pending.is_empty());
        assert_eq!(
            w.players[&addr(2)].hero.skills.advanced.parried_control,
            parry
        );
        let p = w.players.get_mut(&addr(2)).unwrap();
        p.modifiers.god_mode = false;
        p.hero.skills.advanced.parry_until = None;
        control(&mut w, target, owner, Team::Green, 1.0, 1.0, 0.0, 0.0, now);
        assert_eq!(w.players[&addr(2)].hero.hp, before - 70.0);
    }
}

#[test]
fn moved_ultimate_unlock_upgrade_cooldown_and_animation_follow_skill() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let mut skills = CoreId::Dawnweaver.preset().skills;
    skills.swap(0, 3);
    equip(&mut w, 1, CoreId::Dawnweaver, skills);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.modifiers.unlock_all = false;
    p.hero.progress.skill_points = 1;
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(p.hero.progress.skill_points, 1);
    crate::sim::cast::apply_skill_upgrade(p, 3);
    assert_eq!(p.hero.progress.ranks[3], 2);
    assert_eq!(
        hero_stats::ability_cooldown(p, SkillSlot::Q),
        shared::scaled_cooldown(&skill(SkillId::DawnRay).ability, 1)
    );
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    cast(&mut w, addr(1), 3, [18.0, 0.0], 2, now);
    assert_eq!(w.players[&addr(1)].hero.last_action.slot, 3);
    assert_eq!(
        w.players[&addr(1)].hero.last_action.kind,
        shared::PlayerActionKind::Attack
    );
    w.players.get_mut(&addr(1)).unwrap().hero.progress.level = 6;
    cast(&mut w, addr(1), 0, [18.0, 0.0], 3, now + duration(2.0));
    assert_eq!(w.players[&addr(1)].hero.last_action.slot, 0);
    assert_eq!(
        w.players[&addr(1)].hero.last_action.kind,
        shared::PlayerActionKind::Cast
    );
    assert!(w.players[&addr(1)].timers.last_cast_at[0].is_some());
}

#[test]
fn sandbox_unlock_all_preserves_manual_upgrades_without_bypassing_points_or_rank_cap() {
    let (mut w, _, _) = fixture(HeroClass::Dawnweaver);
    let mut skills = CoreId::Dawnweaver.preset().skills;
    skills.swap(0, 3);
    equip(&mut w, 1, CoreId::Dawnweaver, skills);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.modifiers.unlock_all = false;
    p.hero.progress.skill_points = 1;
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(p.hero.progress.ranks[0], 1);
    assert_eq!(p.hero.progress.skill_points, 1);

    p.modifiers.unlock_all = true;
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(p.hero.progress.ranks[0], 2);
    assert_eq!(p.hero.progress.skill_points, 0);
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(p.hero.progress.ranks[0], 2, "points are still required");

    p.hero.progress.skill_points = 2;
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(
        p.hero.progress.ranks[0],
        skill(SkillId::DawnRay).ability.max_rank
    );
    assert_eq!(p.hero.progress.skill_points, 1);
    crate::sim::cast::apply_skill_upgrade(p, 0);
    assert_eq!(
        p.hero.progress.ranks[0],
        skill(SkillId::DawnRay).ability.max_rank
    );
    assert_eq!(
        p.hero.progress.skill_points, 1,
        "rank cap does not spend points"
    );
}

#[test]
fn four_distinct_ultimates_stay_locked_until_six_then_execute_in_their_bindings() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    equip(
        &mut w,
        1,
        CoreId::Dawnweaver,
        [
            SkillId::DawnRay,
            SkillId::WildRocket,
            SkillId::FourfoldDuel,
            SkillId::Nightfall,
        ],
    );
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.modifiers.unlock_all = false;
    p.hero.progress.level = 5;
    p.hero.progress.skill_points = 4;
    crate::bots::auto_rank_skills(p);
    assert_eq!(p.hero.progress.skill_points, 4);
    for slot in 0..4 {
        cast(&mut w, addr(1), slot, [6.0, 0.0], slot as u64 + 1, now);
    }
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    w.players.get_mut(&addr(1)).unwrap().hero.progress.level = 6;
    for slot in 0..4 {
        cast(
            &mut w,
            addr(1),
            slot,
            [6.0, 0.0],
            slot as u64 + 5,
            now + duration(1.0 + slot as f32),
        );
        assert!(
            w.players[&addr(1)].timers.last_cast_at[slot as usize].is_some(),
            "binding {slot}"
        );
        assert_eq!(w.players[&addr(1)].hero.last_action.slot, slot);
    }
}

#[test]
fn moved_zone_recasts_only_its_binding_and_preserves_event_source() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let mut skills = CoreId::Dawnweaver.preset().skills;
    skills.swap(0, 2);
    equip(&mut w, 1, CoreId::Dawnweaver, skills);
    cast(&mut w, addr(1), 0, [6.0, 0.0], 1, now);
    assert!(state(&w.players[&addr(1)], now).unwrap().slots[0].can_recast);
    assert!(!state(&w.players[&addr(1)], now).unwrap().slots[2].can_recast);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 0, [6.0, 0.0], 2, now + duration(0.1));
    let events = std::mem::take(&mut w.skill_runtime.pending);
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e.action_slot == Some(0)));
    assert_eq!(w.players[&addr(1)].hero.mana, mana);
    assert!(!state(&w.players[&addr(1)], now).unwrap().slots[0].can_recast);
}

#[test]
fn moved_orb_receipts_use_the_cast_binding() {
    let (mut w, now, _) = fixture(HeroClass::Orbitwright);
    let mut skills = CoreId::Orbitwright.preset().skills;
    skills.swap(0, 3);
    equip(&mut w, 1, CoreId::Orbitwright, skills);
    cast(&mut w, addr(1), 3, [12.0, 0.0], 1, now);
    let events = advance(&mut w, now, 1.0);
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e.action_slot == Some(3)));
}

#[test]
fn moved_concussion_mark_keeps_its_source_when_basic_attacks_complete_it() {
    let (mut w, now, victim) = fixture(HeroClass::Frostguard);
    let mut skills = CoreId::Frostguard.preset().skills;
    skills.swap(0, 3);
    equip(&mut w, 1, CoreId::Frostguard, skills);
    cast(&mut w, addr(1), 3, [12.0, 0.0], 1, now);
    advance(&mut w, now, 0.7);
    let owner = w.players[&addr(1)].hero.identity.id;
    let mut receipts = Vec::new();
    for index in 0..3 {
        receipts.extend(basic_impact(
            &mut w,
            target(victim),
            12.0,
            source(owner, BASIC_ATTACK_ACTION_SLOT),
            Team::Green,
            100 + index,
            now + duration(0.8 + index as f32 * 0.1),
        ));
    }
    let proc = receipts
        .iter()
        .find(|event| event.amount == 20.0)
        .expect("four stacks trigger concussion");
    assert_eq!(proc.action_slot, Some(3));
}

#[test]
fn recast_from_replaced_skill_cannot_grant_free_cast_or_advertise_it() {
    let (mut w, now, _) = fixture(HeroClass::Veilstalker);
    cast(&mut w, addr(1), 0, [10.0, 0.0], 1, now);
    assert!(state(&w.players[&addr(1)], now).unwrap().slots[0].can_recast);
    let mut skills = CoreId::Veilstalker.preset().skills;
    skills[0] = SkillId::DawnBind;
    equip(&mut w, 1, CoreId::Veilstalker, skills);
    assert!(!state(&w.players[&addr(1)], now).unwrap().slots[0].can_recast);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 0, [10.0, 0.0], 2, now + duration(0.5));
    assert_eq!(
        w.players[&addr(1)].hero.mana,
        mana,
        "old recast does not bypass replacement cooldown"
    );
}

#[test]
fn borrowed_stormfist_recast_charges_its_skill_cost_and_rejects_insufficient_mana() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let mut skills = CoreId::Dawnweaver.preset().skills;
    skills[3] = SkillId::AnchorStep;
    equip(&mut w, 1, CoreId::Dawnweaver, skills);
    cast(&mut w, addr(1), 3, [2.0, 0.0], 1, now);
    assert!(state(&w.players[&addr(1)], now).unwrap().slots[3].can_recast);
    let p = w.players.get_mut(&addr(1)).unwrap();
    p.hero.mana = 24.0;
    let action = p.hero.last_action;
    let last_cast = p.timers.last_cast_at[3];
    cast(&mut w, addr(1), 3, [2.0, 0.0], 2, now + duration(0.4));
    let p = &w.players[&addr(1)];
    assert_eq!(p.hero.mana, 24.0);
    assert_eq!(p.hero.last_action.sequence, action.sequence);
    assert_eq!(p.timers.last_cast_at[3], last_cast);
    assert!(p.hero.skills.advanced.sustain_until.is_none());
    assert!(state(p, now + duration(0.4)).unwrap().slots[3].can_recast);

    w.players.get_mut(&addr(1)).unwrap().hero.mana = 25.0;
    cast(&mut w, addr(1), 3, [2.0, 0.0], 3, now + duration(0.6));
    let p = &w.players[&addr(1)];
    assert_eq!(p.hero.mana, 0.0);
    assert!(p.hero.last_action.sequence > action.sequence);
    assert_eq!(p.hero.last_action.slot, 3);
    assert_eq!(p.timers.last_cast_at[3], last_cast);
    assert!(remaining(p.hero.skills.advanced.sustain_until, now + duration(0.6)) > 0.0);
    assert!(!state(p, now + duration(0.6)).unwrap().slots[3].can_recast);
}

#[test]
fn borrowed_free_recast_on_stormfist_remains_free_at_zero_mana() {
    let (mut w, now, _) = fixture(HeroClass::Stormfist);
    let mut skills = CoreId::Stormfist.preset().skills;
    skills[3] = SkillId::FlameDance;
    equip(&mut w, 1, CoreId::Stormfist, skills);
    cast(&mut w, addr(1), 3, [2.0, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].hero.x, 2.0);
    assert!(state(&w.players[&addr(1)], now).unwrap().slots[3].can_recast);
    w.players.get_mut(&addr(1)).unwrap().hero.mana = 0.0;
    cast(&mut w, addr(1), 3, [3.0, 0.0], 2, now + duration(0.4));
    let p = &w.players[&addr(1)];
    assert_eq!(p.hero.mana, 0.0);
    assert_eq!(p.hero.x, 3.0);
    assert_eq!(p.hero.last_action.slot, 3);
    assert_eq!(p.hero.skills.advanced.recasts[3].as_ref().unwrap().uses, 1);
}
