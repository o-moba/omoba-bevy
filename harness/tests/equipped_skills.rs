//! Real framed UDP proof of authoring through the existing loopback-only lab.
//! Slow simulation keeps receipts observable. Mana/clock bounds use measured
//! simulation time; casts traverse ordinary round/request admission and execution.
use harness::{
    Bot, Character, HeroClass, PlayerState, ServerPacket, ServerProcess, SnapshotView, Team,
};
use shared::SkillSlot;
use shared::loadout::{BuildRecipe, CoreId, EquippedSkills, SkillId};
use shared::sandbox::{
    SandboxActor, SandboxCommand, SandboxConfig, SandboxRequest, SandboxSnapshot,
};
use shared::wire::ClientPacket;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

fn wait(bot: &mut Bot, accept: impl Fn(&ServerPacket) -> bool) -> ServerPacket {
    let deadline = Instant::now() + TIMEOUT;
    let mut last = None;
    while Instant::now() < deadline {
        bot.ping();
        if let Some(packet) = bot.recv_snapshot(deadline) {
            if accept(&packet) {
                return packet;
            }
            last = Some(packet);
        }
    }
    panic!("authoritative condition timed out; last snapshot: {last:?}");
}

fn lab(packet: &ServerPacket) -> &SandboxSnapshot {
    match packet {
        ServerPacket::Snapshot {
            sandbox: Some(sandbox),
            ..
        } => sandbox,
        _ => panic!("expected opt-in sandbox telemetry"),
    }
}

fn me(packet: &ServerPacket) -> &PlayerState {
    packet.player(packet.your_id()).expect("joined owner")
}

fn send(bot: &Bot, packet: &ClientPacket) {
    bot.send_raw(&serde_json::to_vec(packet).unwrap());
}

fn edit(
    bot: &mut Bot,
    before: &ServerPacket,
    command: SandboxCommand,
    accepted: bool,
) -> ServerPacket {
    let request_id = lab(before).last_request_id + 1;
    let request = ClientPacket::Sandbox {
        request: SandboxRequest {
            server_epoch: before.meta().server_epoch,
            match_id: before.meta().match_id,
            request_id,
            command,
        },
    };
    send(bot, &request);
    let packet = wait(bot, |p| {
        lab(p)
            .ack
            .as_ref()
            .is_some_and(|ack| ack.request_id == request_id)
    });
    let ack = lab(&packet).ack.as_ref().unwrap();
    assert_eq!(ack.accepted, accepted, "{}", ack.message);
    packet
}

fn cast(bot: &mut Bot, before: &ServerPacket, slot: u8, aim: [f32; 2]) -> ServerPacket {
    let request_id = me(before).loadout.as_ref().unwrap().cast_request_id + 1;
    send(
        bot,
        &ClientPacket::CastSkill {
            slot,
            aim,
            server_epoch: before.meta().server_epoch,
            match_id: before.meta().match_id,
            request_id,
        },
    );
    wait(bot, |p| {
        me(p)
            .loadout
            .as_ref()
            .is_some_and(|loadout| loadout.cast_request_id == request_id)
    })
}

fn apply(bot: &mut Bot, before: &ServerPacket, config: &SandboxConfig) -> ServerPacket {
    edit(
        bot,
        before,
        SandboxCommand::ApplyConfig {
            config: config.clone(),
        },
        true,
    )
}

fn assert_recipe(packet: &ServerPacket, id: u64, recipe: &BuildRecipe) {
    assert_eq!(packet.your_id(), id);
    assert_eq!(me(packet).hero_class, HeroClass::Dawnweaver);
    assert_eq!(me(packet).team, Team::Green);
    assert_eq!(
        me(packet).loadout.as_ref().unwrap().recipe.as_ref(),
        Some(recipe)
    );
    assert_eq!(lab(packet).config.player.recipe.as_ref(), Some(recipe));
}

fn elapsed(before: &ServerPacket, after: &ServerPacket) -> f32 {
    (lab(after).simulation_secs - lab(before).simulation_secs).max(0.0) as f32
}

fn assert_cost(before: &ServerPacket, after: &ServerPacket, cost: f32) {
    let regen = shared::hero_balance::MANA_REGEN_PER_SECOND * elapsed(before, after);
    let spent = me(before).mana - me(after).mana;
    // Regen before a cast can be capped at full mana, so only the amount
    // actually available in the observed simulation interval is allowed.
    assert!(
        spent >= cost - regen - 0.01 && spent <= cost + 0.01,
        "expected cost {cost} with at most {regen} regen; observed {spent}"
    );
}

fn assert_clock(before: &ServerPacket, after: &ServerPacket, remaining_before: f32, slot: usize) {
    let actual = me(after).skill_cooldown_remaining_secs[slot];
    let earliest = (remaining_before - elapsed(before, after)).max(0.0);
    assert!(
        actual >= earliest - 0.01 && actual <= remaining_before + 0.01,
        "slot {slot} remaining {actual} must be in [{earliest}, {remaining_before}]"
    );
}

#[test]
fn mixed_reordered_and_four_ultimate_recipes_keep_authority_through_real_server() {
    let server = ServerProcess::spawn_with_env(&[
        ("OMOBA_MATCH_MODE", "dev"),
        ("OMOBA_COMBAT_SANDBOX", "1"),
    ]);
    let mut bot = Bot::connect_framed(server.addr());
    bot.join_with_loadout(Team::Green, Character::Cube, HeroClass::Dawnweaver, None);
    let mut packet = wait(&mut bot, |p| {
        p.player(p.your_id()).is_some_and(|me| me.loadout.is_some())
            && matches!(
                p,
                ServerPacket::Snapshot {
                    sandbox: Some(_),
                    ..
                }
            )
    });
    let id = packet.your_id();
    let mut config = lab(&packet).config.clone();
    config.environment.paused = false;
    config.environment.time_scale = 0.1;
    config.player.level = 1;
    config.player.ranks = [1; 4];
    config.player.unlock_all = false;
    config.player.no_cooldowns = false;
    config.player.infinite_resource = false;
    let mut recipe = CoreId::Dawnweaver.preset();
    recipe.skills = [
        SkillId::WildRocket,
        SkillId::DawnField,
        SkillId::DawnBarrier,
        SkillId::DawnBind,
    ];
    config.player.recipe = Some(recipe.clone());
    packet = apply(&mut bot, &packet, &config);
    assert_recipe(&packet, id, &recipe);
    assert_eq!(
        lab(&packet)
            .actors
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .unlocked,
        [false, false, false, true]
    );
    let action = me(&packet).action_sequence;
    let before = packet.clone();
    packet = cast(&mut bot, &packet, 0, [3.0, 0.0]);
    assert_eq!(
        me(&packet).action_sequence,
        action,
        "ultimate on Q stays locked at level one"
    );
    assert_cost(&before, &packet, 0.0);
    assert_eq!(me(&packet).skill_cooldown_remaining_secs, [0.0; 4]);

    config.player.level = 2;
    packet = apply(&mut bot, &packet, &config);
    send(&bot, &ClientPacket::UpgradeSkill { slot: 0 });
    // The next accepted lab receipt is a processing barrier for the preceding
    // ordinary upgrade packet, with no mutation of ranks or skill points.
    packet = edit(
        &mut bot,
        &packet,
        SandboxCommand::Refill {
            actor: SandboxActor::Player,
        },
        true,
    );
    assert_eq!((me(&packet).ranks, me(&packet).skill_points), ([1; 4], 1));
    send(&bot, &ClientPacket::UpgradeSkill { slot: 3 });
    packet = wait(&mut bot, |p| me(p).ranks[3] == 2);
    assert_eq!(
        me(&packet).skill_points,
        0,
        "authored Q on R can be upgraded before level six"
    );

    config.player.level = 6;
    config.player.ranks[3] = 2;
    packet = apply(&mut bot, &packet, &config);
    let equipped = EquippedSkills::resolve(HeroClass::Dawnweaver, Some(&recipe)).unwrap();
    let field = equipped.ability(SkillSlot::W);
    let origin = [me(&packet).x, me(&packet).z];
    let action = me(&packet).action_sequence;
    let before = packet.clone();
    packet = cast(
        &mut bot,
        &packet,
        1,
        [
            origin[0] + shared::scaled_cast_range(field, 1) + 1.0,
            origin[1],
        ],
    );
    assert_eq!(
        me(&packet).action_sequence,
        action,
        "equipped point range rejects distant casts without cost"
    );
    assert_cost(&before, &packet, 0.0);
    let before = packet.clone();
    packet = cast(&mut bot, &packet, 1, [origin[0] + 3.0, origin[1]]);
    assert_eq!(me(&packet).action_sequence, action + 1);
    assert_eq!(me(&packet).action_slot, 1);
    assert_cost(&before, &packet, shared::scaled_mana_cost(field, 1));
    let cooldown = equipped
        .cooldown(me(&packet).level, 1, SkillSlot::W, me(&packet).item_bonuses)
        .as_secs_f32();
    assert_clock(&before, &packet, cooldown, 1);
    assert!(
        me(&packet).loadout.as_ref().unwrap().slots[1].can_recast,
        "recast follows physical W binding"
    );
    assert!(
        matches!(&packet, ServerPacket::Snapshot { skill_effects, .. } if skill_effects.iter().any(|e| e.owner_id == id && e.skill == SkillId::DawnField))
    );
    let before = packet.clone();
    packet = cast(&mut bot, &packet, 1, [origin[0] + 3.0, origin[1]]);
    assert_eq!(me(&packet).action_sequence, action + 2);
    assert_cost(&before, &packet, 0.0);
    assert!(!me(&packet).loadout.as_ref().unwrap().slots[1].can_recast);
    let before = packet.clone();
    packet = cast(&mut bot, &packet, 1, [origin[0] + 3.0, origin[1]]);
    assert_cost(&before, &packet, 0.0);
    assert_eq!(
        me(&packet).action_sequence,
        action + 2,
        "cooldown/recovery rejects the third press"
    );

    // Invalid edits are atomic, including core identity; they cannot erase the
    // already accepted bindings, costs, cooldown or request high-water mark.
    let accepted = packet.clone();
    let mut invalid = config.clone();
    invalid.player.recipe = Some(CoreId::Wildspark.preset());
    packet = edit(
        &mut bot,
        &packet,
        SandboxCommand::ApplyConfig { config: invalid },
        false,
    );
    assert_recipe(&packet, id, &recipe);
    assert_cost(&accepted, &packet, 0.0);
    assert_eq!(me(&packet).action_sequence, me(&accepted).action_sequence);
    assert_eq!(
        me(&packet).loadout.as_ref().unwrap().cast_request_id,
        me(&accepted).loadout.as_ref().unwrap().cast_request_id
    );
    for slot in 0..4 {
        assert_clock(
            &accepted,
            &packet,
            me(&accepted).skill_cooldown_remaining_secs[slot],
            slot,
        );
    }

    recipe.skills = [
        SkillId::DawnRay,
        SkillId::WildRocket,
        SkillId::HorizonWave,
        SkillId::WinterDivide,
    ];
    config.player.recipe = Some(recipe.clone());
    config.player.level = 5;
    config.player.ranks = [1; 4];
    packet = apply(&mut bot, &packet, &config);
    assert_recipe(&packet, id, &recipe);
    assert_eq!(
        lab(&packet)
            .actors
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .unlocked,
        [false; 4]
    );
    let action = me(&packet).action_sequence;
    for slot in 0..4 {
        let before = packet.clone();
        packet = cast(&mut bot, &packet, slot, [3.0, 0.0]);
        assert_eq!(me(&packet).action_sequence, action);
        assert_cost(&before, &packet, 0.0);
    }
    config.player.level = 6;
    packet = apply(&mut bot, &packet, &config);
    packet = edit(
        &mut bot,
        &packet,
        SandboxCommand::Refill {
            actor: SandboxActor::Player,
        },
        true,
    );
    assert_eq!(
        lab(&packet)
            .actors
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .unlocked,
        [true; 4]
    );
    packet = cast(&mut bot, &packet, 0, [3.0, 0.0]);
    assert_eq!(me(&packet).action_sequence, action + 1);
    assert_eq!(me(&packet).action_slot, 0);
    assert!(
        matches!(&packet, ServerPacket::Snapshot { skill_effects, .. } if skill_effects.iter().any(|e| e.owner_id == id && e.skill == SkillId::DawnRay))
    );
    assert_recipe(&packet, id, &recipe);
    eprintln!(
        "R16 live UDP: mixed/reordered/four-ultimate recipes, simulation-bounded cost/cooldown plus range/recast, early-ultimate rejection, bound upgrade, atomic core rejection, same player {id}"
    );
}
