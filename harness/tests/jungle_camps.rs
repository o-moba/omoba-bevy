//! Real local UDP farming/respawn. No placement, invulnerability or clock fixture.
use harness::{Bot, Character, HeroClass, NeutralCampType, ServerPacket, ServerProcess, Team};
use shared::combat::CombatEntityKind;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// The real lanes keep fighting during the forty-second camp wait. Count their
/// accepted kills independently so every assertion still proves exactly 55 XP
/// per Spitter, rather than accepting any larger XP total.
#[derive(Default)]
struct LaneRewards {
    teams: HashMap<u64, Team>,
    rewarded: HashSet<u64>,
}

impl LaneRewards {
    fn observe(&mut self, packet: &ServerPacket, own_team: Team) {
        for minion in packet.minions() {
            if let Some(team) = minion.team {
                self.teams.insert(minion.id, team);
            }
        }
        for event in packet
            .combat_events()
            .iter()
            .filter(|event| event.killed && event.target.kind == CombatEntityKind::Minion)
        {
            let team = self
                .teams
                .get(&event.target.id)
                .expect("saw wave before its confirmed death");
            if *team != own_team {
                self.rewarded.insert(event.target.id);
            }
        }
    }

    fn jungle_xp(&self, player: &harness::PlayerState) -> u32 {
        // Independent published level-curve expectations from balance.rs.
        // One admitted farmer per team receives the entire 90-XP minion pool.
        const LEVEL_THRESHOLDS: [u32; 9] = [90, 150, 180, 220, 260, 300, 340, 380, 420];
        assert!(
            (1..=9).contains(&player.level),
            "scenario must not reach XP-capped level 10"
        );
        let total = player.xp
            + LEVEL_THRESHOLDS[..player.level as usize - 1]
                .iter()
                .sum::<u32>();
        total
            .checked_sub(self.rewarded.len() as u32 * 90)
            .expect("observed lane kills cannot exceed awarded XP")
    }
}

#[test]
fn both_teams_farm_real_camps_and_observe_same_id_respawn_after_forty_seconds() {
    let server = ServerProcess::spawn();
    let mut bots = [
        Bot::connect_framed(server.addr()),
        Bot::connect_framed(server.addr()),
    ];
    for (bot, (team, class)) in bots.iter().zip([
        (Team::Green, HeroClass::Mage),
        (Team::Blue, HeroClass::Ranger),
    ]) {
        bot.join_with_loadout(team, Character::Ipfs, class, None);
    }
    let mut navigators: [harness::navigation::BotNavigator; 2] = Default::default();
    let mut target_ids = [None; 2];
    let mut anchors = [[0.0; 2]; 2];
    let mut first_death: [Option<Instant>; 2] = [None; 2];
    let mut respawned = [false; 2];
    let mut finished = [false; 2];
    let mut first_gold = [0; 2];
    let mut request_id = [0_u64; 2];
    let mut saw_six = [false; 2];
    let mut lane_rewards: [LaneRewards; 2] = Default::default();
    let start = Instant::now();
    while !finished.into_iter().all(|done| done) && start.elapsed() < Duration::from_secs(85) {
        for index in 0..2 {
            let bot = &mut bots[index];
            bot.ping();
            let packet = bot
                .recv_snapshot(Instant::now() + Duration::from_secs(2))
                .expect("live snapshot");
            let Some(me) = packet.player(packet.your_id()) else {
                continue;
            };
            lane_rewards[index].observe(&packet, if index == 0 { Team::Green } else { Team::Blue });
            assert!(
                me.hp > 0.0,
                "farmer died before completing two camp kills: {me:?}"
            );
            if target_ids[index].is_none() {
                let ordinary: Vec<_> = packet
                    .neutrals()
                    .iter()
                    .filter(|n| !n.camp_type.is_boss())
                    .collect();
                if ordinary.len() != 6 {
                    continue;
                }
                saw_six[index] = true;
                let mob = ordinary
                    .into_iter()
                    .filter(|n| n.camp_type == NeutralCampType::Spitter)
                    .min_by(|a, b| {
                        ((a.x - me.x).hypot(a.z - me.z))
                            .total_cmp(&((b.x - me.x).hypot(b.z - me.z)))
                    })
                    .expect("each side has a spitter");
                target_ids[index] = Some(mob.id);
                anchors[index] = [mob.x, mob.z];
                first_gold[index] = me.gold;
            }
            let target = target_ids[index].unwrap();
            let mob = packet.neutrals().iter().find(|n| n.id == target);
            if first_death[index].is_none() && mob.is_none() {
                assert_eq!(
                    lane_rewards[index].jungle_xp(me),
                    55,
                    "one last hit grants exactly Spitter XP after accounted lane rewards"
                );
                assert!(me.gold >= first_gold[index] + 35);
                first_death[index] = Some(Instant::now());
                eprintln!(
                    "team {index}: first kill at {:.2}s, xp={}, gold={}",
                    start.elapsed().as_secs_f32(),
                    me.xp,
                    me.gold
                );
            }
            if let Some(dead_at) = first_death[index] {
                if !respawned[index] {
                    assert_eq!(
                        lane_rewards[index].jungle_xp(me),
                        55,
                        "absent camp cannot pay repeated rewards, even while lanes award XP"
                    );
                    if let Some(mob) = mob {
                        assert!(
                            dead_at.elapsed() >= Duration::from_secs(39),
                            "early respawn"
                        );
                        assert!((mob.hp - mob.max_hp).abs() < 0.01);
                        assert!((mob.x - anchors[index][0]).hypot(mob.z - anchors[index][1]) < 0.2);
                        respawned[index] = true;
                        eprintln!(
                            "team {index}: same ID {target} respawned after {:.2}s",
                            dead_at.elapsed().as_secs_f32()
                        );
                    } else {
                        // Stale attack retries against an absent entity must remain harmless.
                        strike(bot, &packet, target, &mut request_id[index]);
                        continue;
                    }
                } else if mob.is_none() {
                    assert_eq!(
                        lane_rewards[index].jungle_xp(me),
                        110,
                        "two Spitters grant exactly 110 jungle XP independently of lane kills"
                    );
                    assert!(
                        me.level >= 2,
                        "110 jungle XP crosses level 2 even without lane rewards"
                    );
                    assert!(me.gold >= first_gold[index] + 70);
                    eprintln!(
                        "team {index}: two camps grant 110 XP; independently observed {} enemy-minion rewards, level={}, xp={}",
                        lane_rewards[index].rewarded.len(),
                        me.level,
                        me.xp
                    );
                    finished[index] = true;
                    continue;
                }
            }
            if finished[index] {
                continue;
            }
            let mob = mob.expect("living target");
            let distance = (mob.x - me.x).hypot(mob.z - me.z);
            if distance > 9.0 {
                // Normal 4m/s source movement follows the same forest route map.
                if let Some(next) = navigators[index].next([me.x, me.z], anchors[index], &[]) {
                    let dx = next[0] - me.x;
                    let dz = next[1] - me.z;
                    let length = dx.hypot(dz);
                    let step = length.min(0.20);
                    if length > 0.001 {
                        bot.send_transform(
                            me.x + dx / length * step,
                            0.5,
                            me.z + dz / length * step,
                            dx.atan2(dz),
                        );
                    }
                }
            } else {
                strike(bot, &packet, target, &mut request_id[index]);
                bot.send_raw(serde_json::to_string(&serde_json::json!({"type":"cast","target":{"kind":"neutral","id":target},"slot":0})).unwrap().as_bytes());
            }
        }
    }
    assert!(saw_six.into_iter().all(|seen| seen));
    assert_ne!(
        target_ids[0], target_ids[1],
        "each side farms its mirrored camp"
    );
    assert!(
        finished.into_iter().all(|done| done),
        "two real kills per side were not completed within deadline"
    );
}

fn strike(bot: &Bot, packet: &ServerPacket, id: u64, request: &mut u64) {
    *request += 1;
    let meta = packet.meta();
    bot.send_raw(serde_json::to_string(&serde_json::json!({"type":"basic_attack","target":{"kind":"neutral","id":id},"server_epoch":meta.server_epoch,"match_id":meta.match_id,"request_id":request})).unwrap().as_bytes());
}
