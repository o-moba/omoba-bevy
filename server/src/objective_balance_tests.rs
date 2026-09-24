//! Controlled stationary, no-minion outer-tower safety sample through production authority.

use shared::map::Lane;

use crate::sim::towers::simulate_tower_attacks;

use std::net::UdpSocket;

use shared::wire::TargetId;

use shared::map::Team;

use crate::sim::cast::handle_cast_request;

use crate::sim::projectiles::simulate_projectiles;

use shared::wire::TargetKind;

use crate::world::build_structures;

use crate::match_rules::MatchConfig;

use crate::world::spawn_minion_wave_for_team_lane;

use crate::game_world::GameWorld;

use shared::wire::CharacterChoice;

use shared::HeroClass;

use crate::runtime::ServerRuntime;

use shared::wire::StructureKind;

use std::time::Instant;

use shared::wire::ClientPacket;

use crate::basic_attack::handle_basic_attack_request;

use crate::game_world::TickCtx;

use std::net::SocketAddr;

use crate::world::build_map_layout;

use crate::balance::MANA_REGEN_PER_SECOND;

use std::time::Duration;

fn siege(
    class: HeroClass,
    hp: Option<f32>,
    tower_damage: f32,
    fire_hero: bool,
) -> serde_json::Value {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
    let now = Instant::now();
    let addr: SocketAddr = "127.0.0.1:59201".parse().unwrap();
    rt.handle_packet(
        addr,
        ClientPacket::Join {
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: class,
            avatar: None,
            sprite_character: None,
            session_id: None,
            passport_ticket: None,
        },
        now,
    );
    let player = rt.world.players.get_mut(&addr).unwrap();
    player.hero.x = 0.0;
    player.hero.z = 0.0;
    if let Some(hp) = hp {
        player.hero.hp = hp;
        player.hero.max_hp = hp;
    }
    let starting_hp = player.hero.hp;
    let tower_id = *rt
        .world
        .structures
        .iter()
        .find(|(_, s)| s.state.team == Team::Blue && s.state.kind == StructureKind::Tower)
        .unwrap()
        .0;
    rt.world.structures.retain(|id, _| *id == tower_id);
    let tower = rt.world.structures.get_mut(&tower_id).unwrap();
    tower.state.x = 3.0;
    tower.state.z = 0.0;
    // Keep the production profile base damage and vary only its hero multiplier.
    tower.hero_damage_multiplier = tower_damage / tower.attack_damage;
    let target = TargetId {
        kind: TargetKind::Structure,
        id: tower_id,
    };
    rt.world.minions.clear();
    rt.world.neutrals.clear();
    let dt = 1.0 / 120.0;
    let mut elapsed = 60.0;
    for tick in 0..7200 {
        let at = now + Duration::from_secs_f32(tick as f32 * dt);
        let p = rt.world.players.get_mut(&addr).unwrap();
        if tick > 0 {
            p.hero.mana = (p.hero.mana + MANA_REGEN_PER_SECOND * dt).min(p.hero.max_mana);
        }
        if fire_hero {
            handle_basic_attack_request(&mut rt.world, addr, target, tick + 1, at);
            handle_cast_request(&mut rt.world, addr, target, 0, at);
        }
        simulate_tower_attacks(&mut rt.world, at);
        simulate_projectiles(&mut rt.world, TickCtx { now: at, dt });
        if rt.world.players[&addr].hero.hp <= 0.0 || rt.world.structures[&tower_id].state.hp <= 0.0
        {
            elapsed = tick as f32 * dt;
            break;
        }
    }
    serde_json::json!({"class":class,"hero_hp":starting_hp,"tower_damage":tower_damage,"offensive_policy":if fire_hero {"basic_then_q_every_tick"} else {"passive"},"seconds":elapsed,"hero_remaining_hp":rt.world.players[&addr].hero.hp,"tower_remaining_hp":rt.world.structures[&tower_id].state.hp,"tower_destroyed":rt.world.structures[&tower_id].state.hp<=0.0})
}

#[test]
fn no_minion_outer_tower_preserves_early_threat() {
    let configured = shared::map::ResolvedMap::default();
    assert!(
        configured
            .structures
            .iter()
            .all(|s| s.stats.attack_damage * s.stats.hero_damage_multiplier
                == if s.lane.is_some() { 28.0 } else { 36.0 })
    );
    let mut rows = Vec::new();
    for class in HeroClass::ALL {
        for damage in [14.0, 28.0] {
            for hp in [Some(100.0), None] {
                for offensive in [false, true] {
                    rows.push(siege(class, hp, damage, offensive));
                }
            }
        }
    }
    let report = serde_json::json!({"setup":{"level":1,"items":[],"mana":"finite 100, ordinary 8/s regeneration","distance":3.0,"dt":1.0/120.0,"tower_hp":240,"tower_cooldown":0.9,"skills":"current production Q/basic; HP100 rows isolate historical resource baseline, not full historical kits","excluded":"fountain, minions, healing, retreat, other towers, respawn","placement":"default outer tower moved to unobstructed origin+3m; ranges, projectile heights and authority unchanged"},"rows":rows});
    if let Ok(path) = std::env::var("OMOBA_OBJECTIVE_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    for row in report["rows"].as_array().unwrap() {
        if row["tower_damage"] == 28.0 && row["offensive_policy"] == "basic_then_q_every_tick" {
            assert_eq!(row["tower_destroyed"], false, "{row}");
            assert_eq!(row["hero_remaining_hp"], 0.0, "{row}");
        }
    }
}

#[test]
fn standard_wave_tower_clear_keeps_ordinary_minion_damage() {
    let layout = build_map_layout();
    let mut rows = Vec::new();
    for damage in [14.0, 28.0] {
        let mut world = GameWorld::empty();
        world.structures = build_structures(&layout);
        let id = *world
            .structures
            .iter()
            .find(|(_, s)| s.state.kind == StructureKind::Tower && s.state.team == Team::Blue)
            .unwrap()
            .0;
        world.structures.retain(|key, _| *key == id);
        let tower = world.structures.get_mut(&id).unwrap();
        tower.state.x = 0.0;
        tower.state.z = 0.0;
        tower.attack_damage = damage;
        spawn_minion_wave_for_team_lane(
            &layout,
            &mut world.minions,
            &mut 1,
            Team::Green,
            Lane::Mid,
        );
        for m in world.minions.values_mut() {
            m.state.x = m.state.id as f32;
            m.state.z = 0.0;
        }
        let now = Instant::now();
        let mut hits = 0;
        for shot in 0..20 {
            let receipts =
                simulate_tower_attacks(&mut world, now + Duration::from_millis(900 * shot));
            hits += receipts.len();
            if world.minions.values().all(|m| m.state.hp <= 0.0) {
                rows.push(serde_json::json!({"damage":damage,"shots":hits,"seconds_from_first_shot":shot as f32*0.9}));
                break;
            }
        }
    }
    assert_eq!(rows[0]["shots"], 14);
    assert_eq!(rows[1]["shots"], 8);
    if let Ok(path) = std::env::var("OMOBA_WAVE_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&serde_json::json!({"setup":"Production spawn (two melee65HP, caster45HP) and tower target/damage authority, stationary in range, no retaliation/healing/movement, exact900ms shot opportunities; old14 vs proposed blanket28", "rows":rows})).unwrap()).unwrap();
    }
}
