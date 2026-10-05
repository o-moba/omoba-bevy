use super::*;
use crate::session::handle_join_request;
use shared::wire::CharacterChoice;
use shared::{BASIC_ATTACK_ACTION_SLOT, HeroClass};

#[path = "equipped_tests.rs"]
mod equipped_tests;

fn addr(n: u16) -> SocketAddr {
    format!("127.0.0.1:{}", 56000 + n).parse().unwrap()
}
fn add_player(
    w: &mut GameWorld,
    n: u16,
    class: HeroClass,
    team: Team,
    pos: [f32; 2],
    now: Instant,
) -> u64 {
    w.ensure_connected(addr(n), now);
    handle_join_request(
        w.players.get_mut(&addr(n)).unwrap(),
        team,
        CharacterChoice::Ipfs,
        class,
        None,
        &w.map_layout,
        now,
    );
    let p = w.players.get_mut(&addr(n)).unwrap();
    p.hero.x = pos[0];
    p.hero.z = pos[1];
    p.hero.hp = 1000.0;
    p.hero.max_hp = 1000.0;
    p.hero.mana = 500.0;
    p.hero.max_mana = 500.0;
    p.modifiers.unlock_all = true;
    p.hero.identity.id
}
fn fixture(class: HeroClass) -> (GameWorld, Instant, u64) {
    let now = Instant::now();
    let mut w = GameWorld::empty();
    add_player(&mut w, 1, class, Team::Green, [0.0, 0.0], now);
    let victim = add_player(&mut w, 2, HeroClass::Warrior, Team::Blue, [6.0, 0.0], now);
    (w, now, victim)
}
fn advance(w: &mut GameWorld, now: Instant, seconds: f32) -> Vec<CombatEvent> {
    let mut result = Vec::new();
    let count = (seconds / 0.05).ceil() as usize;
    for i in 1..=count {
        let at = now + duration((i as f32 * 0.05).min(seconds));
        let events = tick(w, TickCtx { now: at, dt: 0.05 });
        observe(w, &events, at);
        normalize(w, at);
        result.extend(events);
    }
    result
}
fn target(id: u64) -> TargetId {
    TargetId {
        kind: TargetKind::Player,
        id,
    }
}
#[test]
fn aimed_bind_hits_first_two_roots_and_misses_third() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [9.0, 0.0], now);
    add_player(&mut w, 4, HeroClass::Warrior, Team::Blue, [12.0, 0.0], now);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    let events = advance(&mut w, now, 0.7);
    assert_eq!(events.len(), 2);
    assert_eq!(w.players[&addr(2)].hero.hp, 976.0);
    assert_eq!(w.players[&addr(3)].hero.hp, 976.0);
    assert_eq!(w.players[&addr(4)].hero.hp, 1000.0);
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.7)),
        0.0
    );
    assert_eq!(w.players[&addr(1)].hero.mana, 485.0);
    let p = w.players.get_mut(&addr(2)).unwrap();
    let before = [p.hero.x, p.hero.z];
    crate::session::handle_transform_request(
        p,
        &w.map_layout,
        20.0,
        0.0,
        0.0,
        0.0,
        now + duration(0.7),
    );
    assert_eq!([p.hero.x, p.hero.z], before);
}
#[test]
fn bind_miss_and_invalid_or_replayed_casts_do_not_create_hits() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), 0, [f32::NAN, 0.0], 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    assert!(w.skill_runtime.effects.is_empty());
    cast(&mut w, addr(1), 0, [0.0, 18.0], 2, now);
    assert_eq!(w.skill_runtime.effects.len(), 1);
    assert!(advance(&mut w, now, 1.0).is_empty());
    cast(&mut w, addr(1), 0, [18.0, 0.0], 2, now + duration(10.0));
    assert!(w.skill_runtime.effects.is_empty());
}
#[test]
fn barrier_both_passes_stack_bounded_and_absorb_every_damage_entry() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let ally = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [6.0, 0.0], now);
    cast(&mut w, addr(1), 1, [15.0, 0.0], 1, now);
    advance(&mut w, now, 1.8);
    assert_eq!(
        state(&w.players[&addr(3)], now + duration(1.8))
            .unwrap()
            .shield_hp,
        60.0
    );
    assert_eq!(
        state(&w.players[&addr(1)], now + duration(1.8))
            .unwrap()
            .shield_hp,
        60.0
    );
    assert!(
        crate::combat_feedback::apply_player_damage(
            &mut w.players,
            ally,
            25.0,
            now + duration(1.8)
        )
        .is_none()
    );
    let event = crate::combat_feedback::apply_player_damage_typed(
        &mut w.players,
        ally,
        50.0,
        now + duration(1.8),
        true,
    )
    .unwrap();
    assert_eq!(event.amount, 15.0);
    assert_eq!(w.players[&addr(3)].hero.hp, 985.0);
}
#[test]
fn field_recast_is_free_once_and_expiry_also_detonates() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 1, now);
    advance(&mut w, now, 0.1);
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.1)),
        0.7
    );
    cast(&mut w, addr(1), 2, [6.0, 0.0], 2, now + duration(0.1));
    let events = advance(&mut w, now + duration(0.1), 0.1);
    assert_eq!(events.len(), 1);
    assert_eq!(w.players[&addr(1)].hero.mana, 476.0);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 2, now + duration(0.2));
    assert!(advance(&mut w, now + duration(0.2), 0.1).is_empty());
    let later = now + duration(11.0);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 3, later);
    assert_eq!(advance(&mut w, later, 5.1).len(), 1);
    assert!(
        !state(&w.players[&addr(1)], later + duration(5.1))
            .unwrap()
            .slots[2]
            .can_recast
    );
}
#[test]
fn ray_warns_then_consumes_and_reapplies_radiance_basic_consumes_once() {
    let (mut w, now, victim) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.4);
    let later = now + duration(0.5);
    cast(&mut w, addr(1), 3, [45.0, 0.0], 2, later);
    assert!(
        effects(&w, later)
            .iter()
            .any(|e| e.kind == EffectVisualKind::BeamWarning)
    );
    assert!(advance(&mut w, later, 0.7).is_empty());
    let events = advance(&mut w, later + duration(0.7), 0.15);
    assert_eq!(events.len(), 2);
    assert_eq!(w.players[&addr(2)].hero.hp, 886.0);
    let owner = w.players[&addr(1)].hero.identity.id;
    let events = basic_impact(
        &mut w,
        target(victim),
        12.0,
        source(owner, BASIC_ATTACK_ACTION_SLOT),
        Team::Green,
        999,
        later + duration(1.0),
    );
    assert_eq!(events.len(), 2);
    let events = basic_impact(
        &mut w,
        target(victim),
        12.0,
        source(owner, BASIC_ATTACK_ACTION_SLOT),
        Team::Green,
        1000,
        later + duration(1.1),
    );
    assert_eq!(events.len(), 1);
}
#[test]
fn rocket_mode_spends_mana_snapshot_survives_switch_and_splashes() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [6.0, 1.0], now);
    cast(&mut w, addr(1), 0, [0.0, 0.0], 1, now);
    crate::basic_attack::handle_basic_attack_request(&mut w, addr(1), target(victim), 1, now);
    assert_eq!(w.players[&addr(1)].hero.mana, 496.0);
    assert_eq!(
        w.projectiles.values().next().unwrap().state.style,
        ProjectileStyle::Rocket
    );
    cast(&mut w, addr(1), 0, [0.0, 0.0], 2, now + duration(0.3));
    assert_eq!(w.players[&addr(1)].hero.skills.mode, WeaponMode::Repeater);
    let events = crate::sim::projectiles::simulate_projectiles(
        &mut w,
        TickCtx {
            now: now + duration(0.4),
            dt: 0.4,
        },
    );
    assert_eq!(events.len(), 2);
    assert!((w.players[&addr(2)].hero.hp - 976.6).abs() < 0.001);
    assert!((w.players[&addr(3)].hero.hp - 976.6).abs() < 0.001);
    cast(&mut w, addr(1), 0, [0.0, 0.0], 3, now + duration(0.6));
    w.players.get_mut(&addr(1)).unwrap().hero.mana = 3.0;
    crate::basic_attack::handle_basic_attack_request(
        &mut w,
        addr(1),
        target(victim),
        2,
        now + duration(2.0),
    );
    assert!(w.projectiles.is_empty());
}
#[test]
fn armed_traps_do_not_trigger_early_or_repeat_for_same_cast() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 1, now);
    assert!(advance(&mut w, now, 0.5).is_empty());
    let events = advance(&mut w, now + duration(0.5), 0.2);
    assert_eq!(events.len(), 1);
    assert!(events[0].trap_triggered);
    assert_eq!(w.players[&addr(2)].hero.hp, 980.0);
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.7)),
        0.0
    );
    assert!(advance(&mut w, now + duration(0.7), 2.0).is_empty());
}
#[test]
fn shielded_trap_emits_one_activation_without_damage_or_repeat() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    let caster = w.players[&addr(1)].hero.identity.id;
    shield(&mut w, victim, victim, 100.0, 10.0, now);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 1, now);
    assert!(advance(&mut w, now, 0.5).is_empty());
    let events = advance(&mut w, now + duration(0.5), 0.2);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert!(event.trap_triggered);
    assert_eq!(event.amount, 0.0);
    assert!(!event.killed);
    assert_eq!(event.source.id, caster);
    assert_eq!(event.target.id, victim);
    assert_eq!(event.action_slot, Some(2));
    assert_eq!([event.x, event.z], [6.0, 0.0]);
    assert_eq!(w.players[&addr(2)].hero.hp, 1000.0);
    assert_eq!(w.players[&addr(2)].hero.skills.shields[0].amount, 80.0);
    assert_eq!(
        w.players[&addr(2)]
            .hero
            .skills
            .control
            .movement(now + duration(0.7)),
        0.0
    );
    let mut log = crate::combat_feedback::CombatLog::default();
    log.enable_sandbox(now);
    log.extend(now + duration(0.7), events);
    let first = log.snapshot(now + duration(0.7));
    assert!(first[0].id > 0);
    assert_eq!(log.snapshot(now + duration(0.8)), first);
    let analytics = log.sandbox_analytics(now + duration(0.8));
    assert_eq!(analytics.hits, 0);
    assert_eq!(analytics.damage, 0.0);
    assert!(analytics.breakdown.is_empty());
    assert!(advance(&mut w, now + duration(0.7), 2.0).is_empty());
}

#[test]
fn shockline_damage_is_physical_and_reveal_has_expiry() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().modifiers.armor = 100.0;
    cast(&mut w, addr(1), 1, [24.0, 0.0], 1, now);
    advance(&mut w, now, 0.3);
    assert_eq!(w.players[&addr(2)].hero.hp, 985.0);
    assert!(crate::vision::revealed(
        &w.players[&addr(2)],
        now + duration(0.3)
    ));
    assert!(!crate::vision::revealed(
        &w.players[&addr(2)],
        now + duration(2.5)
    ));
}
#[test]
fn momentum_assist_uses_accepted_contributions_and_deduplicates_death() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    let finisher = add_player(&mut w, 3, HeroClass::Warrior, Team::Green, [3.0, 0.0], now);
    let owner = w.players[&addr(1)].hero.identity.id;
    let e = apply_hit(
        &mut w,
        target(victim),
        20.0,
        DamageType::Physical,
        source(owner, 1),
        Team::Green,
        false,
        false,
        false,
        now,
    );
    observe(&mut w, &e, now);
    let kill = apply_hit(
        &mut w,
        target(victim),
        2000.0,
        DamageType::Physical,
        source(finisher, 0),
        Team::Green,
        false,
        false,
        false,
        now + duration(2.0),
    );
    observe(&mut w, &kill, now + duration(2.0));
    assert_eq!(
        w.players[&addr(1)]
            .hero
            .skills
            .movement(now + duration(3.0)),
        1.4
    );
    let until = w.players[&addr(1)].hero.skills.momentum_until;
    observe(&mut w, &kill, now + duration(4.0));
    assert_eq!(w.players[&addr(1)].hero.skills.momentum_until, until);
    assert_eq!(
        w.players[&addr(1)]
            .hero
            .skills
            .movement(now + duration(9.0)),
        1.0
    );
}
#[test]
fn mixed_recipe_executes_foreign_skill_and_round_reset_clears_runtime() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    let mut recipe = shared::loadout::CoreId::Dawnweaver.preset();
    recipe.skills[1] = SkillId::WildZap;
    w.players.get_mut(&addr(1)).unwrap().hero.skills.loadout =
        Some(shared::loadout::resolve(&recipe).unwrap());
    cast(&mut w, addr(1), 1, [24.0, 0.0], 9, now);
    assert_eq!(effects(&w, now)[0].skill, SkillId::WildZap);
    advance(&mut w, now, 0.3);
    assert_eq!(w.players[&addr(2)].hero.hp, 970.0);
    assert!(!w.skill_runtime.marks.is_empty());
    w.reset_round(now + duration(1.0));
    assert!(effects(&w, now + duration(1.0)).is_empty());
    assert!(w.skill_runtime.marks.is_empty());
    assert_eq!(w.players[&addr(1)].hero.skills.request_id, 0);
}
#[test]
fn death_and_respawn_preserve_request_highwater_but_clear_transient_state() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    cast(&mut w, addr(1), 0, [0.0, 0.0], 17, now);
    let owner = w.players[&addr(1)].hero.identity.id;
    crate::combat_feedback::apply_player_damage(&mut w.players, owner, 2000.0, now);
    normalize(&mut w, now);
    crate::session::handle_respawns(&mut w, now + duration(6.0));
    assert_eq!(w.players[&addr(1)].hero.skills.request_id, 17);
    assert_eq!(w.players[&addr(1)].hero.skills.mode, WeaponMode::Repeater);
    cast(&mut w, addr(1), 0, [0.0, 0.0], 17, now + duration(6.0));
    assert_eq!(w.players[&addr(1)].hero.skills.mode, WeaponMode::Repeater);
}

fn add_minion(w: &mut GameWorld, pos: [f32; 2]) -> u64 {
    crate::world::spawn_minion_wave_for_team_lane(
        &w.map_layout,
        &mut w.minions,
        &mut w.next_minion_id,
        Team::Blue,
        shared::map::Lane::Mid,
    );
    let id = *w.minions.keys().min().unwrap();
    w.minions.retain(|key, _| *key == id);
    let m = w.minions.get_mut(&id).unwrap();
    m.state.x = pos[0];
    m.state.z = pos[1];
    id
}
#[test]
fn global_rocket_passes_minions_and_scales_distance_and_missing_health() {
    let (mut near, now, _) = fixture(HeroClass::Wildspark);
    cast(&mut near, addr(1), 3, [256.0, 0.0], 1, now);
    advance(&mut near, now, 0.3);
    let near_damage = 1000.0 - near.players[&addr(2)].hero.hp;
    let (mut far, now, _) = fixture(HeroClass::Wildspark);
    far.players.get_mut(&addr(2)).unwrap().hero.x = 25.0;
    let minion = add_minion(&mut far, [4.0, 0.0]);
    let creep_hp = far.minions[&minion].state.hp;
    cast(&mut far, addr(1), 3, [256.0, 0.0], 1, now);
    advance(&mut far, now, 0.7);
    assert_eq!(far.minions[&minion].state.hp, creep_hp);
    assert_eq!(far.players[&addr(2)].hero.hp, 916.0);
    assert!(near_damage < 84.0);
    let (mut wounded, now, _) = fixture(HeroClass::Wildspark);
    let p = wounded.players.get_mut(&addr(2)).unwrap();
    p.hero.x = 25.0;
    p.hero.hp = 500.0;
    cast(&mut wounded, addr(1), 3, [256.0, 0.0], 1, now);
    advance(&mut wounded, now, 0.7);
    assert_eq!(wounded.players[&addr(2)].hero.hp, 291.0);
}
#[test]
fn npc_root_stops_actual_minion_movement_then_expires() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    w.players.get_mut(&addr(2)).unwrap().hero.z = 12.0;
    let id = add_minion(&mut w, [6.0, 0.0]);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    advance(&mut w, now, 0.4);
    let t = TargetId {
        kind: TargetKind::Minion,
        id,
    };
    assert_eq!(w.skill_runtime.npc_movement(t, now + duration(0.4)), 0.0);
    let before = [w.minions[&id].state.x, w.minions[&id].state.z];
    crate::sim::minions::simulate_minions(
        &mut w,
        TickCtx {
            now: now + duration(0.4),
            dt: 0.1,
        },
    );
    assert_eq!([w.minions[&id].state.x, w.minions[&id].state.z], before);
    assert_eq!(w.skill_runtime.npc_movement(t, now + duration(2.0)), 1.0);
}
#[test]
fn visible_incoming_effect_survives_hidden_owner_and_does_not_publish_destination() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 40.0;
    cast(&mut w, addr(1), 3, [256.0, 0.0], 1, now);
    advance(&mut w, now, 0.6);
    let at = now + duration(0.6);
    let owner = w.players[&addr(1)].hero.identity.id;
    let viewer = &w.players[&addr(2)];
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
        players: crate::snapshot::build_players_snapshot(&w, Some(viewer.hero.identity.id), at),
        scoreboard: None,
        prematch: None,
        skill_effects: effects(&w, at),
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    crate::vision::filter_snapshot(&mut packet, viewer, &w, at);
    let shared::wire::ServerPacket::Snapshot {
        players,
        skill_effects,
        ..
    } = packet
    else {
        unreachable!()
    };
    assert!(!players.iter().any(|p| p.id == owner));
    assert_eq!(skill_effects.len(), 1);
    assert!(distance(skill_effects[0].position, skill_effects[0].end) <= 1.001);
}
#[test]
fn beam_warning_clips_hidden_origin_but_preserves_visible_threat() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 40.0;
    cast(&mut w, addr(1), 3, [45.0, 0.0], 1, now);
    let viewer = &w.players[&addr(2)];
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
        skill_effects: effects(&w, now),
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: Vec::new(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    crate::vision::filter_snapshot(&mut packet, viewer, &w, now);
    let shared::wire::ServerPacket::Snapshot {
        players,
        skill_effects,
        ..
    } = packet
    else {
        unreachable!()
    };
    assert_eq!(players.len(), 1);
    assert_eq!(skill_effects.len(), 1);
    let visible = &skill_effects[0];
    assert_eq!(visible.kind, EffectVisualKind::BeamWarning);
    assert_eq!(visible.owner_id, 0);
    assert!(visible.position[0] > 0.0 && visible.position[0] <= 40.0);
    assert!(visible.end[0] >= 40.0);
    assert_eq!(
        w.skill_runtime.effects.values().next().unwrap().origin,
        [0.0, 0.0]
    );
}
#[test]
fn sibling_trap_victim_does_not_block_other_eligible_hero() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().hero.z = -0.75;
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [6.0, 1.0], now);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 1, now);
    let third = *w.skill_runtime.effects.keys().max().unwrap();
    w.skill_runtime.effects.remove(&third);
    advance(&mut w, now, 0.7);
    assert_eq!(w.players[&addr(2)].hero.hp, 980.0);
    assert_eq!(w.players[&addr(3)].hero.hp, 980.0);
}
#[test]
fn standard_recovery_blocks_spending_but_free_zone_recast_remains_usable() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 1, [15.0, 0.0], 2, now);
    assert_eq!(w.players[&addr(1)].hero.mana, mana);
    cast(&mut w, addr(1), 2, [6.0, 0.0], 3, now + duration(0.2));
    let mana = w.players[&addr(1)].hero.mana;
    cast(&mut w, addr(1), 2, [6.0, 0.0], 4, now + duration(0.2));
    assert_eq!(w.players[&addr(1)].hero.mana, mana);
    assert!(
        !state(&w.players[&addr(1)], now + duration(0.2))
            .unwrap()
            .slots[2]
            .can_recast
    );
}

#[test]
fn diagonal_point_preview_roundoff_is_clamped_without_admitting_invalid_range() {
    let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
    // A client f32 clamp of (100, 100) to range 16 produces this endpoint.
    let diagonal = [11.313_709_f32, 11.313_709_f32];
    assert!(distance([0.0, 0.0], diagonal) > 16.0);
    cast(&mut w, addr(1), 2, diagonal, 1, now);
    let effect = w.skill_runtime.effects.values().next().unwrap();
    assert!(distance([0.0, 0.0], effect.pos) <= 16.0);
    assert_eq!(w.players[&addr(1)].hero.mana, 476.0);

    for invalid in [[16.01, 0.0], [f32::INFINITY, 0.0], [4097.0, 0.0]] {
        let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
        cast(&mut w, addr(1), 2, invalid, 1, now);
        assert!(w.skill_runtime.effects.is_empty());
        assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
    }
}

#[test]
fn repeater_stacks_change_accepted_cadence_expire_and_modes_enforce_range() {
    use crate::basic_attack::handle_basic_attack_request as strike;
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    for (request, secs, count, stacks) in [
        (1, 0.0, 1, 1),
        (2, 0.61, 1, 1),
        (3, 0.64, 2, 2),
        (4, 1.19, 2, 2),
        (5, 1.22, 3, 3),
        (6, 1.75, 4, 3),
    ] {
        strike(
            &mut w,
            addr(1),
            target(victim),
            request,
            now + duration(secs),
        );
        assert_eq!(w.projectiles.len(), count);
        assert_eq!(
            state(&w.players[&addr(1)], now + duration(secs))
                .unwrap()
                .passive_stacks,
            stacks
        );
    }
    assert_eq!(
        state(&w.players[&addr(1)], now + duration(5.0))
            .unwrap()
            .passive_stacks,
        0
    );
    assert_eq!(
        hero_stats::basic_attack_cooldown_at(&w.players[&addr(1)], now + duration(5.0)),
        duration(0.7)
    );
    strike(&mut w, addr(1), target(victim), 7, now + duration(5.1));
    assert_eq!(w.projectiles.len(), 5);
    assert_eq!(
        state(&w.players[&addr(1)], now + duration(5.1))
            .unwrap()
            .passive_stacks,
        1
    );

    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 13.0;
    strike(&mut w, addr(1), target(victim), 1, now);
    assert!(w.projectiles.is_empty());
    cast(&mut w, addr(1), 0, [0.0, 0.0], 1, now);
    strike(&mut w, addr(1), target(victim), 2, now);
    strike(&mut w, addr(1), target(victim), 3, now + duration(1.18));
    assert_eq!(w.projectiles.len(), 1);
    strike(&mut w, addr(1), target(victim), 4, now + duration(1.21));
    assert_eq!(w.projectiles.len(), 2);
    assert_eq!(w.players[&addr(1)].hero.mana, 492.0);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 18.0;
    strike(&mut w, addr(1), target(victim), 5, now + duration(3.0));
    assert_eq!(w.projectiles.len(), 2);
    assert_eq!(w.players[&addr(1)].hero.mana, 492.0);
}

#[test]
fn skill_admission_guards_preserve_resources_and_bound_live_effects() {
    for condition in ["mana", "locked", "dead", "cooldown"] {
        let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
        if condition == "cooldown" {
            cast(&mut w, addr(1), 0, [0.0, 18.0], 1, now);
        }
        let p = w.players.get_mut(&addr(1)).unwrap();
        match condition {
            "mana" => p.hero.mana = 0.0,
            "locked" => p.modifiers.unlock_all = false,
            "dead" => p.hero.hp = 0.0,
            _ => {}
        }
        let (mana, timers, count) = (
            p.hero.mana,
            p.timers.last_cast_at,
            w.skill_runtime.effects.len(),
        );
        cast(
            &mut w,
            addr(1),
            if condition == "locked" { 3 } else { 0 },
            [0.0, 18.0],
            2,
            now + duration(0.2),
        );
        assert_eq!(w.players[&addr(1)].hero.mana, mana, "{condition}");
        assert_eq!(
            w.players[&addr(1)].timers.last_cast_at,
            timers,
            "{condition}"
        );
        assert_eq!(w.skill_runtime.effects.len(), count, "{condition}");
    }
    for global in [false, true] {
        let (mut w, now, _) = fixture(HeroClass::Dawnweaver);
        let owners: Vec<_> = if global { (3..=10).collect() } else { vec![1] };
        for owner in owners {
            if owner != 1 {
                add_player(
                    &mut w,
                    owner,
                    HeroClass::Dawnweaver,
                    Team::Green,
                    [0.0, 0.0],
                    now,
                );
            }
            w.players
                .get_mut(&addr(owner))
                .unwrap()
                .modifiers
                .no_cooldowns = true;
            for request in 1..=MAX_OWNER_EFFECTS as u64 {
                cast(&mut w, addr(owner), 0, [0.0, 18.0], request, now);
            }
        }
        assert_eq!(
            w.skill_runtime.effects.len(),
            if global {
                MAX_EFFECTS
            } else {
                MAX_OWNER_EFFECTS
            }
        );
        let p = &w.players[&addr(1)];
        let (mana, timers) = (p.hero.mana, p.timers.last_cast_at);
        cast(&mut w, addr(1), 0, [0.0, 18.0], 100, now);
        assert_eq!(w.players[&addr(1)].hero.mana, mana);
        assert_eq!(w.players[&addr(1)].timers.last_cast_at, timers);
        assert_eq!(
            w.skill_runtime.effects.len(),
            if global {
                MAX_EFFECTS
            } else {
                MAX_OWNER_EFFECTS
            }
        );
    }
}

#[test]
fn public_loadout_preserves_combat_state_but_redacts_cast_request_sequence() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    cast(&mut w, addr(1), 0, [0.0, 0.0], 42, now);
    let player = &w.players[&addr(1)];
    let owner = player.owner_view(now, &w.map_layout, &w.game_state);
    let public = player.public_view(now, &w.map_layout, &w.game_state);
    let mut expected = owner.loadout.unwrap();
    assert_eq!(expected.cast_request_id, 42);
    assert_eq!(expected.weapon_mode, WeaponMode::Rockets);
    assert!(expected.recipe.is_some());
    expected.cast_request_id = 0;
    assert_eq!(public.loadout, Some(expected));
    assert_eq!(player.hero.skills.request_id, 42);
}

#[path = "roster_tests.rs"]
mod roster_tests;

#[test]
fn accepted_action_facing_is_stable_across_movement_and_rejected_casts() {
    for class in [
        HeroClass::Dawnweaver,
        HeroClass::Riftshot,
        HeroClass::Warrior,
    ] {
        let (mut w, now, victim) = fixture(class);
        let p = w.players.get_mut(&addr(1)).unwrap();
        p.modifiers.bypass_vision = true;
        if class.is_standard() {
            cast(&mut w, addr(1), 0, [18.0, 0.0], 1, now);
        } else {
            crate::sim::cast::handle_cast_request(&mut w, addr(1), target(victim), 0, now);
        }
        let accepted = w.players[&addr(1)].hero.last_action;
        assert_eq!(accepted.sequence, 1, "{}", class.id());
        let yaw = accepted.yaw.unwrap();
        assert!((yaw - shared::math::hero_yaw_towards(1.0, 0.0)).abs() < 1e-5);
        // A later locomotion update must not change the direction of this action.
        w.players.get_mut(&addr(1)).unwrap().hero.yaw = 1.25;
        if class.is_standard() {
            cast(&mut w, addr(1), 0, [-18.0, 0.0], 2, now);
        } else {
            crate::sim::cast::handle_cast_request(&mut w, addr(1), target(victim), 0, now);
        }
        assert_eq!(
            w.players[&addr(1)].hero.last_action,
            accepted,
            "rejected cooldown cast must not turn the model"
        );
        let p = w.players.get_mut(&addr(1)).unwrap();
        crate::sim::cast::record_player_action(p, SkillSlot::W);
        assert_eq!(
            p.hero.last_action.yaw, None,
            "self actions clear previous aim"
        );
    }
}

#[test]
fn rocket_carries_temporary_team_sight_at_its_real_position_until_impact() {
    let (mut w, now, _) = fixture(HeroClass::Wildspark);
    w.players.get_mut(&addr(2)).unwrap().hero.x = 100.0;
    cast(&mut w, addr(1), 3, [256.0, 0.0], 1, now);
    advance(&mut w, now, 1.0);
    let at = now + duration(1.0);
    let rocket = effects(&w, at)
        .into_iter()
        .find(|e| e.kind == EffectVisualKind::Rocket)
        .unwrap();
    assert!(rocket.position[0] > shared::vision::HERO_SIGHT_RADIUS);
    let green = crate::vision::sources(Team::Green, &w);
    let light = green
        .iter()
        .find(|s| s.radius == shared::vision::ROCKET_SIGHT_RADIUS)
        .unwrap();
    assert_eq!(light.position, rocket.position);
    assert!(shared::vision::point_visible(
        &green,
        [rocket.position[0] + 3.0, rocket.position[1]],
        false
    ));
    assert!(
        !crate::vision::sources(Team::Blue, &w)
            .iter()
            .any(|s| s.radius == shared::vision::ROCKET_SIGHT_RADIUS)
    );
    w.players.get_mut(&addr(2)).unwrap().hero.x = rocket.position[0] + 3.0;
    advance(&mut w, at, 0.2);
    assert!(effects(&w, at + duration(0.2)).is_empty());
    assert!(
        !crate::vision::sources(Team::Green, &w)
            .iter()
            .any(|s| s.radius == shared::vision::ROCKET_SIGHT_RADIUS)
    );
}

#[test]
fn all_towers_are_public_map_landmarks_without_granting_attack_vision() {
    let now = Instant::now();
    let mut w = GameWorld::new(shared::map::ResolvedMap::default(), now);
    add_player(
        &mut w,
        1,
        HeroClass::Wildspark,
        Team::Green,
        [-100.0, -100.0],
        now,
    );
    let viewer = &w.players[&addr(1)];
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
        skill_effects: effects(&w, now),
        projectiles: Vec::new(),
        combat_events: Vec::new(),
        structures: w.structures.values().map(|s| s.state.clone()).collect(),
        minions: Vec::new(),
        neutrals: Vec::new(),
        team_buffs: Vec::new(),
        forest_pickups: Vec::new(),
        game_state: GameState::Running,
        rematch_in_secs: None,
    };
    crate::vision::filter_snapshot(&mut packet, viewer, &w, now);
    let shared::wire::ServerPacket::Snapshot { structures, .. } = packet else {
        panic!("snapshot")
    };
    assert_eq!(structures.len(), w.structures.len());
    assert!(structures.iter().any(|s| s.team == Team::Blue));
    assert!(structures.iter().any(|s| s.team == Team::Green));
    let far = structures.iter().find(|s| s.team == Team::Blue).unwrap();
    assert!(!crate::vision::target_visible(
        Team::Green,
        TargetId {
            kind: TargetKind::Structure,
            id: far.id
        },
        &w,
        now
    ));
}

#[test]
fn repeater_is_single_target_and_keeps_the_fired_mode_after_switching() {
    let (mut w, now, victim) = fixture(HeroClass::Wildspark);
    add_player(&mut w, 3, HeroClass::Warrior, Team::Blue, [6.0, 1.0], now);
    crate::basic_attack::handle_basic_attack_request(&mut w, addr(1), target(victim), 1, now);
    assert_eq!(
        w.projectiles.values().next().unwrap().state.style,
        ProjectileStyle::Bullet
    );
    cast(&mut w, addr(1), 0, [0.0, 0.0], 1, now);
    assert_eq!(
        w.projectiles.values().next().unwrap().state.style,
        ProjectileStyle::Bullet
    );
    let events = crate::sim::projectiles::simulate_projectiles(
        &mut w,
        TickCtx {
            now: now + duration(0.4),
            dt: 0.4,
        },
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].style, ProjectileStyle::Bullet);
    assert_eq!(w.players[&addr(3)].hero.hp, 1000.0);
    assert_eq!(w.players[&addr(1)].hero.mana, 500.0);
}
