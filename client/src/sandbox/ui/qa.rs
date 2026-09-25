//! Explicit native evidence harness: real button handlers and UDP authority.
//! It injects `SyntheticPress` messages through the UI kit, not physical
//! mouse input; reports that limit.
use super::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use std::path::PathBuf;
#[derive(Resource)]
struct Qa {
    dir: PathBuf,
    started: Instant,
    step: usize,
    at: Instant,
    trace: Vec<serde_json::Value>,
    capture: Vec<String>,
    baseline: Option<(f64, u64, u64)>,
    warm: Option<f64>,
    finished: bool,
}
pub(super) fn install(app: &mut App) {
    let Some(dir) = std::env::var_os("OMOBA_SANDBOX_QA_OUTPUT").map(PathBuf::from) else {
        return;
    };
    if !requested() {
        return;
    }
    std::fs::create_dir_all(&dir).expect("sandbox QA output");
    app.insert_resource(Qa {
        dir,
        started: Instant::now(),
        step: if std::env::var_os("OMOBA_SANDBOX_QA_RESTORE").is_some() {
            100
        } else {
            0
        },
        at: Instant::now(),
        trace: vec![],
        capture: vec![],
        baseline: None,
        warm: None,
        finished: false,
    })
    .insert_resource(bevy::winit::WinitSettings::continuous())
    .add_systems(Update, drive.before(keys));
}
/// The panel's kit buttons and the press channel into the recognizer.
#[derive(bevy::ecs::system::SystemParam)]
struct Buttons<'w, 's> {
    query: Query<
        'w,
        's,
        (
            Entity,
            &'static UiAction<Action>,
            &'static ComputedNode,
            &'static UiGlobalTransform,
        ),
    >,
    presses: MessageWriter<'w, crate::ui::SyntheticPress>,
}
fn press(
    buttons: &mut Buttons,
    panels: &mut Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<Body>>,
    predicate: impl Fn(&Action) -> bool,
) -> bool {
    for (entity, UiAction(a), node, transform) in buttons.query.iter() {
        if !predicate(a) {
            continue;
        }
        if !matches!(a, Action::Tab(_) | Action::Close)
            && let Ok((body, body_transform, mut scroll)) = panels.single_mut()
        {
            let center = transform.translation.y;
            let min = body_transform.translation.y - body.size().y * 0.5;
            let max = min + body.size().y;
            if center - node.size().y * 0.5 < min || center + node.size().y * 0.5 > max {
                let limit = ((body.content_size().y - body.size().y) * body.inverse_scale_factor())
                    .max(0.0);
                scroll.y = (scroll.y + (center - (min + max) * 0.5) * body.inverse_scale_factor())
                    .clamp(0.0, limit);
                return false;
            }
        }
        buttons.presses.write(crate::ui::SyntheticPress(entity));
        return true;
    }
    false
}
fn shot(commands: &mut Commands, qa: &mut Qa, name: &str) {
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.dir.join(name)));
    qa.capture.push(name.into());
}
fn finish(qa: &mut Qa, success: bool, reason: &str, exit: &mut MessageWriter<AppExit>) {
    qa.finished = true;
    let result = serde_json::json!({"status":if success{"PASS"}else{"FAIL"},"reason":reason,"version":env!("CARGO_PKG_VERSION"),"method":"SyntheticPress through the UI kit on actual panel actions, live UDP authoritative sandbox, native rendering", "manual_input_verified":false,"warm_entry_seconds":qa.warm,"trace":qa.trace,"screenshots":qa.capture});
    std::fs::write(
        qa.dir.join("qa-summary.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    exit.write(if success {
        AppExit::Success
    } else {
        AppExit::error()
    });
}
fn drive(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    mut state: ResMut<SandboxClient>,
    game: Res<GameStateSnapshot>,
    session: Res<crate::net::ClientSession>,
    animations: Res<AnimationReadout>,
    playback: Query<(Entity, &AnimationPlayer)>,
    mut buttons: Buttons,
    mut panels: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<Body>>,
    mut network: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.finished {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(150) {
        let reason = format!("Timed out at step {}: {}", qa.step, state.status);
        finish(&mut qa, false, &reason, &mut exit);
        return;
    }
    if qa.at.elapsed() < Duration::from_millis(250) {
        return;
    }
    let Some(s) = game.sandbox.as_ref() else {
        return;
    };
    if state.pending.is_some() || !state.queue.is_empty() {
        return;
    }
    let next = match qa.step {
        0 => {
            if session.join_confirmed() && animations.0.contains_key(&game.your_id) {
                qa.warm = Some(qa.started.elapsed().as_secs_f64());
                shot(&mut commands, &mut qa, "01-entry.png");
                true
            } else {
                false
            }
        }
        1 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Presets))
        }),
        2 => press(
            &mut buttons,
            &mut panels,
            |a| matches!(a,Action::Load(n)if n=="dps"),
        ),
        3 => s.config.dummy.enabled && s.config.player.level == 10,
        4 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Player))
        }),
        5 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Hero(shared::HeroClass::Mage))
        }),
        6 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::God))
        }),
        7 => press(
            &mut buttons,
            &mut panels,
            |a| matches!(a,Action::Step(Field::Hp,d)if *d>0.0),
        ),
        8 => {
            s.config.player.hero == shared::HeroClass::Mage
                && s.config.player.max_hp == 200.0
                && s.config.player.god_mode
        }
        9 => press(&mut buttons, &mut panels, |a| {
            matches!(
                a,
                Action::Command(SandboxCommand::ForceCast { slot: 0, .. })
            )
        }),
        10 => {
            if s.analytics.damage > 0.0 {
                if let Some(dummy) = s.actors.iter().find(|a| a.actor == SandboxActor::Dummy) {
                    network.write(NetworkCommand::BasicAttack {
                        target: crate::net::TargetId {
                            kind: crate::net::TargetKind::Player,
                            id: dummy.id,
                        },
                    });
                }
                true
            } else {
                false
            }
        }
        11 => s
            .analytics
            .breakdown
            .iter()
            .any(|r| r.slot == Some(shared::BASIC_ATTACK_ACTION_SLOT)),
        12 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Damage))
        }),
        13 => {
            shot(&mut commands, &mut qa, "02-damage.png");
            true
        }
        14 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Dummy))
        }),
        15 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::DummyMoving))
        }),
        16 => {
            if s.config.dummy.moving {
                shot(&mut commands, &mut qa, "03-dummy.png");
                true
            } else {
                false
            }
        }
        17 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Enemy))
        }),
        18 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::Enemy))
        }),
        19 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Hero(shared::HeroClass::Ranger))
        }),
        20 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Behavior(BotBehavior::Fight))
        }),
        21 => {
            if s.config.enemy.enabled
                && s.config.enemy.actor.hero == shared::HeroClass::Ranger
                && s.config.enemy.behavior == BotBehavior::Fight
            {
                shot(&mut commands, &mut qa, "04-enemy.png");
                true
            } else {
                false
            }
        }
        22 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::World))
        }),
        23 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::Pause))
        }),
        24 => {
            if s.config.environment.paused {
                qa.baseline = Some((s.simulation_secs, s.frame, game.meta.snapshot_tick));
                shot(&mut commands, &mut qa, "05-paused.png");
                true
            } else {
                false
            }
        }
        25 => {
            if qa.at.elapsed() > Duration::from_secs(7) {
                let (t, frame, tick) = qa.baseline.unwrap();
                if s.simulation_secs != t
                    || s.frame != frame
                    || game.meta.snapshot_tick <= tick
                    || !session.join_confirmed()
                {
                    finish(
                        &mut qa,
                        false,
                        "Pause changed simulation or stopped live snapshots",
                        &mut exit,
                    );
                    return;
                }
                true
            } else {
                false
            }
        }
        26 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Command(SandboxCommand::FrameStep))
        }),
        27 => {
            let (t, frame, _) = qa.baseline.unwrap();
            s.frame == frame + 1 && (s.simulation_secs - t - 1.0 / 60.0).abs() < 0.00001
        }
        28 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Animation))
        }),
        29 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Preview(PreviewKind::Run))
        }),
        30 => {
            if animations
                .0
                .get(&game.your_id)
                .is_some_and(|a| a.0.contains("Run"))
            {
                shot(&mut commands, &mut qa, "06-animation.png");
                true
            } else {
                false
            }
        }
        31 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Command(SandboxCommand::FrameStep))
        }),
        32 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::StopPreview)
        }),
        33 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::Pause))
        }),
        34 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Presets))
        }),
        35 => {
            state.preset_name = "qa-roundtrip".into();
            press(&mut buttons, &mut panels, |a| matches!(a, Action::Save))
        }
        36 => press(
            &mut buttons,
            &mut panels,
            |a| matches!(a,Action::Load(n)if n=="duel"),
        ),
        37 => {
            if s.config.enemy.enabled && !s.config.dummy.enabled && s.config.player.level == 10 {
                shot(&mut commands, &mut qa, "07-duel.png");
                true
            } else {
                false
            }
        }
        38 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::LoadNamed)
        }),
        39 => {
            if s.config.player.hero == shared::HeroClass::Mage
                && s.config.player.max_hp == 200.0
                && s.config.enemy.enabled
                && s.config.dummy.enabled
            {
                shot(&mut commands, &mut qa, "08-restored.png");
                true
            } else {
                false
            }
        }
        40 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::World))
        }),
        41 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::Geometry))
        }),
        42 => {
            shot(&mut commands, &mut qa, "09-geometry.png");
            true
        }
        43 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Tab(Tab::Animation))
        }),
        44 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Actor(SandboxActor::Player))
        }),
        45 => press(&mut buttons, &mut panels, |a| {
            matches!(a, Action::Toggle(Toggle::Pause))
        }),
        46..=69 => {
            let n = qa.step - 46;
            let kind = [
                PreviewKind::Attack,
                PreviewKind::Cast,
                PreviewKind::Hit,
                PreviewKind::Death,
                PreviewKind::Run,
                PreviewKind::Walk,
            ][n / 4];
            match n % 4 {
                0 => press(
                    &mut buttons,
                    &mut panels,
                    |a| matches!(a, Action::Preview(k) if *k == kind),
                ),
                1 => {
                    let name = format!("motion-{kind:?}-before.png");
                    shot(&mut commands, &mut qa, &name);
                    true
                }
                2 => press(&mut buttons, &mut panels, |a| {
                    matches!(a, Action::Command(SandboxCommand::FrameStep))
                }),
                _ => {
                    let name = format!("motion-{kind:?}-stepped.png");
                    shot(&mut commands, &mut qa, &name);
                    true
                }
            }
        }
        70 => press(&mut buttons, &mut panels, |a| matches!(a, Action::Repeat)),
        71 => {
            shot(&mut commands, &mut qa, "motion-repeat.png");
            true
        }
        72 | 101 => {
            if qa.capture.iter().all(|f| qa.dir.join(f).is_file()) {
                finish(
                    &mut qa,
                    true,
                    "All live workflow stages completed",
                    &mut exit,
                );
            }
            false
        }
        100 => {
            if session.join_confirmed()
                && animations.0.contains_key(&game.your_id)
                && s.config.player.hero == shared::HeroClass::Mage
                && s.config.player.max_hp == 200.0
                && s.config.enemy.enabled
                && s.config.dummy.enabled
            {
                qa.warm = Some(qa.started.elapsed().as_secs_f64());
                shot(&mut commands, &mut qa, "restart-restored.png");
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if next {
        let step = qa.step;
        let elapsed = qa.started.elapsed().as_secs_f64();
        let clips: Vec<_> = playback.iter().map(|(entity, player)| {
            let active: Vec<_> = player.playing_animations().map(|(index, animation)|
                serde_json::json!({"node":index.index(),"seek":animation.seek_time(),"paused":animation.is_paused()})).collect();
            serde_json::json!({"entity":entity.to_bits(),"clips":active})
        }).collect();
        qa.trace.push(serde_json::json!({"step":step,"elapsed":elapsed,"snapshot_tick":game.meta.snapshot_tick,"state":s,"motion":animations.0,"playback":clips}));
        qa.step += 1;
        qa.at = Instant::now();
    }
}
