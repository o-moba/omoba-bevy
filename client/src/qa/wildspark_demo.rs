//! Focused live sandbox director. It only sends ordinary casts/attacks and lab setup.
use super::*;

const LENGTHS: [f64; 6] = [4.5, 6.0, 4.5, 5.0, 2.5, 3.0];
const NAMES: [&str; 6] = [
    "repeater",
    "rockets",
    "slow",
    "traps",
    "rocket-near",
    "rocket-far",
];

#[derive(Resource, Default)]
pub(super) struct Demo {
    chapter: usize,
    stage: u8,
    wire: Wire,
    began: f64,
    last_shot: f64,
    recording: Option<Instant>,
    next_attack: f64,
    casts: u8,
    frames: usize,
    first: usize,
    receipt_floor: u64,
    records: Vec<serde_json::Value>,
}

pub(super) fn drive(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    mut demo: ResMut<Demo>,
    world: PhaseWorld,
    mut focus: ResMut<crate::minimap::MinimapNavigationState>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage != PHASE_STAGE {
        return;
    }
    let Some(sandbox) = world.game.sandbox.as_ref() else {
        return;
    };
    if let Some(file) = qa.black.take() {
        fail(&mut qa, &mut exit, "Black demo frame", file.into());
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(180) {
        fail(
            &mut qa,
            &mut exit,
            "Demo timed out",
            serde_json::json!({"chapter":demo.chapter,"stage":demo.stage}),
        );
        return;
    }
    let acknowledged = match demo.wire.poll(sandbox, &mut outgoing) {
        Some(Ok(())) => true,
        Some(Err(reason)) => {
            fail(&mut qa, &mut exit, &reason, serde_json::Value::Null);
            return;
        }
        None => false,
    };
    let now = sandbox.simulation_secs;
    let distance = match demo.chapter {
        0 | 1 => 9.0,
        2 | 3 => 13.0,
        4 => 6.0,
        _ => 22.0,
    };
    let middle = if matches!(demo.chapter, 2 | 3) && demo.stage >= 2 {
        world
            .actor(SandboxActor::Enemy)
            .map(|actor| (STAGE_HOME + Vec2::from_array(actor.position)) * 0.5)
            .unwrap_or(STAGE_HOME + STAGE_LANE * distance * 0.5)
    } else {
        STAGE_HOME + STAGE_LANE * distance * 0.5
    };
    focus.focus_target = Some(Vec3::new(middle.x, 0.0, middle.y));
    qa.zoom = if demo.chapter == 5 { 1.1 } else { 0.65 };
    match demo.stage {
        0 => {
            let mut config = stage_config(&qa);
            config.player.infinite_resource = false;
            config.player.no_cooldowns = false;
            config.player.god_mode = false;
            config.player.max_hp = 1000.0;
            config.enemy.actor.hero = HeroClass::Warrior;
            config.enemy.actor.avatar = Some("good-knight".into());
            config.enemy.actor.position = (STAGE_HOME + STAGE_LANE * distance).to_array();
            config.enemy.actor.max_hp = 400.0;
            config.enemy.actor.damage_multiplier = 0.1;
            config.enemy.actor.move_speed = 0.7;
            config.enemy.behavior = if matches!(demo.chapter, 2 | 3) {
                shared::sandbox::BotBehavior::Attack
            } else {
                shared::sandbox::BotBehavior::Stationary
            };
            config.enemy.attack_distance = 1.0;
            config.enemy.aggression_range = 40.0;
            config.dummy.enabled = matches!(demo.chapter, 0 | 1 | 4 | 5);
            config.dummy.infinite_hp = false;
            config.dummy.max_hp = 400.0;
            config.dummy.position =
                (STAGE_HOME + STAGE_LANE * distance + STAGE_LANE.perp() * 1.5).to_array();
            demo.wire.send(
                &world.game,
                sandbox,
                SandboxCommand::ApplyConfig { config },
                &mut outgoing,
            );
            demo.stage = 1;
        }
        1 if acknowledged => {
            demo.wire.send(
                &world.game,
                sandbox,
                SandboxCommand::ResetDuel,
                &mut outgoing,
            );
            demo.stage = 2;
            demo.began = now;
        }
        2 if acknowledged && now - demo.began > 0.6 => {
            demo.began = now;
            demo.last_shot = -1.0;
            demo.recording = Some(Instant::now());
            demo.next_attack = now + 0.5;
            demo.casts = 0;
            demo.first = demo.frames;
            demo.receipt_floor = world
                .game
                .combat_events
                .iter()
                .map(|event| event.id)
                .max()
                .unwrap_or(0);
            demo.stage = 3;
        }
        3 => {
            let Some(enemy) = world.actor(SandboxActor::Enemy) else {
                return;
            };
            let elapsed = now - demo.began;
            let aim = Vec2::from_array(enemy.position);
            if demo.chapter == 1 && demo.casts == 0 && elapsed > 0.15 {
                cast(
                    &mut qa,
                    &mut outgoing,
                    0,
                    aim,
                    world.game.meta.snapshot_tick,
                );
                demo.casts = 1;
                demo.next_attack = now + 0.45;
            }
            if demo.chapter <= 1 && now >= demo.next_attack && elapsed < LENGTHS[demo.chapter] - 0.7
            {
                outgoing.write(NetworkCommand::BasicAttack {
                    target: shared::wire::TargetId::player(enemy.id),
                });
                demo.next_attack = now + 0.15;
            }
            if demo.chapter >= 2
                && demo.casts == 0
                && elapsed > if demo.chapter == 3 { 0.15 } else { 0.65 }
            {
                let slot = match demo.chapter {
                    2 => 1,
                    3 => 2,
                    _ => 3,
                };
                let aim = if demo.chapter == 3 {
                    STAGE_HOME + STAGE_LANE * 5.0
                } else {
                    aim
                };
                cast(
                    &mut qa,
                    &mut outgoing,
                    slot,
                    aim,
                    world.game.meta.snapshot_tick,
                );
                demo.casts = 1;
            }
            let recording_secs = demo.recording.unwrap().elapsed().as_secs_f64();
            if recording_secs - demo.last_shot >= 1.0 / 30.0 {
                demo.last_shot = recording_secs;
                let file = format!("demo-{:04}.png", demo.frames);
                let action = world.local.single().ok().map(|(_,_,_,action,_)| serde_json::json!({"sequence":action.sequence,"slot":action.slot}));
                let record = serde_json::json!({
                    "chapter":NAMES[demo.chapter], "chapter_index":demo.chapter,
                    "seconds":recording_secs, "simulation_secs":now, "action":action,
                    "actors":sandbox.actors, "states":world.hero_states(),
                    "receipts":world.game.combat_events.iter().filter(|event|event.id>demo.receipt_floor).collect::<Vec<_>>(),
                    "effects":world.game.skill_effects, "audio":world.audio.row_voices,
                    "particles":world.particle_sources(), "animation":world.animation(),
                });
                let index = 20_000 + demo.frames;
                shoot(&mut commands, &mut qa, index, file, record);
                demo.frames += 1;
            }
            if elapsed >= LENGTHS[demo.chapter] {
                demo.stage = 4;
            }
        }
        4 if (demo.first..demo.frames).all(|index| qa.readbacks.contains(&(20_000 + index))) => {
            let chapter = demo.chapter;
            let records: Vec<_> = qa
                .captures
                .iter()
                .filter(|r| r["chapter_index"].as_u64() == Some(chapter as u64))
                .cloned()
                .collect();
            demo.records
                .push(serde_json::json!({"chapter":NAMES[chapter],"frames":records}));
            demo.chapter += 1;
            demo.stage = 0;
            if demo.chapter == LENGTHS.len() {
                let report = serde_json::json!({"capture_complete":true,"avatar":qa.avatar,"viewport":qa.pixels(),"locale":"en","source":"live sandbox; normal mana/cooldowns; scripted accepted casts; setup resets cut out","chapters":demo.records});
                std::fs::write(
                    qa.directory.join("demo.json"),
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .unwrap();
                qa.stage = 254;
                exit.write(AppExit::Success);
            }
        }
        _ => {}
    }
}
