//! Focused, explicitly synthetic presentation fixtures; never multiplayer proof.
use crate::{
    combat::CombatStats,
    net::{GameState, GameStateSnapshot, PlayerProgression},
    player::Player,
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{path::PathBuf, time::Instant};

pub(crate) struct CombatPolishQaPlugin;
impl Plugin for CombatPolishQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_COMBAT_POLISH_QA_DIR").map(PathBuf::from)
        else {
            return;
        };
        app.insert_resource(Qa {
            directory,
            started: Instant::now(),
            frames: 0,
            stage: 0,
            pending: false,
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(
            Update,
            prepare
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::mobile_controls::MobileControlsSet::Input),
        )
        .add_systems(PostUpdate, capture.after(bevy::ui::UiSystems::Layout));
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    started: Instant,
    frames: u32,
    stage: usize,
    pending: bool,
}
const FILES: [&str; 3] = [
    "01-vitals-upgrades-kill-feed.png",
    "02-concealed.png",
    "03-rocket.png",
];
fn prepare(
    mut qa: ResMut<Qa>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut game: ResMut<GameStateSnapshot>,
    mut hero: Query<(&mut Transform, &mut CombatStats, &mut PlayerProgression), With<Player>>,
    mut presses: crate::qa::TestIdPresses,
    mut touch: MessageWriter<bevy::input::touch::TouchInput>,
    mobile: Res<crate::mobile_controls::MobileControls>,
    mut exit: MessageWriter<AppExit>,
    _main_thread: bevy::ecs::system::NonSendMarker,
) {
    if qa.started.elapsed().as_secs() > 120 {
        exit.write(AppExit::error());
        return;
    }
    let Ok((window_id, mut window)) = windows.single_mut() else {
        return;
    };
    window.resolution.set_scale_factor_override(Some(1.0));
    window.resolution.set(1180.0, 820.0);
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(window_id)
            && !window.has_focus()
        {
            window.focus_window();
        }
    });
    presses.press("HelpDismissButton");
    let Ok((mut pose, mut stats, mut progression)) = hero.single_mut() else {
        return;
    };
    if !matches!(game.state, GameState::Running) {
        return;
    }
    qa.frames += 1;
    pose.translation.x = -22.0;
    pose.translation.z = -10.0;
    stats.hp = 65.0;
    stats.max_hp = 100.0;
    stats.mana = 40.0;
    stats.max_mana = 100.0;
    progression.level = 10;
    progression.skill_points = 4;
    progression.ranks = [1; 4];
    game.vision = Some(shared::vision::TeamVision {
        sources: vec![shared::vision::VisionSource {
            position: [-22.0, -10.0],
            radius: 32.0,
        }],
        local_brush: Some(1),
        local_hidden: qa.stage == 1,
    });
    let player = |id, name: &str, team| shared::live_score::LiveScorePlayer {
        player_id: id,
        nickname: name.into(),
        avatar: Some("agnes".into()),
        team,
        hero_class: shared::HeroClass::Wildspark,
        kills: 0,
        deaths: 0,
        assists: 0,
        earned_gold: 0,
        level: 10,
        connected: true,
    };
    game.scoreboard = Some(shared::live_score::LiveScoreboard {
        players: vec![
            player(game.your_id, "Wildspark", shared::map::Team::Green),
            player(999, "Target dummy", shared::map::Team::Blue),
        ],
        kills: if qa.frames >= 150 {
            vec![shared::live_score::KillNotice {
                event_id: (qa.frames / 90) as u64,
                killer_id: game.your_id,
                victim_id: 999,
            }]
        } else {
            vec![]
        },
    });
    if qa.stage == 0 && qa.frames == 150 {
        touch.write(bevy::input::touch::TouchInput {
            window: window_id,
            phase: bevy::input::touch::TouchPhase::Started,
            position: mobile.layout().attack_center,
            force: None,
            id: 998,
        });
    }
    if qa.stage == 1 && qa.frames == 1 {
        touch.write(bevy::input::touch::TouchInput {
            window: window_id,
            phase: bevy::input::touch::TouchPhase::Canceled,
            position: mobile.layout().attack_center,
            force: None,
            id: 998,
        });
    }
    game.skill_effects = if qa.stage == 2 {
        vec![shared::loadout::SkillEffectState {
            id: 9999,
            owner_id: game.your_id,
            owner_team: shared::map::Team::Green,
            skill: shared::loadout::SkillId::WildRocket,
            kind: shared::loadout::EffectVisualKind::Rocket,
            position: [-16.0, -10.0],
            end: [-15.0, -10.0],
            radius: 2.5,
            remaining_secs: 2.0,
            armed: true,
            consumed_segments: 0,
        }]
    } else {
        vec![]
    };
}
fn capture(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    roots: Query<(&SceneRoot, Option<&bevy::scene::SceneInstance>)>,
    assets: Res<AssetServer>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Res<crate::mobile_controls::MobileControls>,
) {
    if !context.gameplay_allowed() || !mobile.focused {
        if qa.frames.is_multiple_of(120) {
            info!(
                "COMBAT_POLISH_QA waiting for gameplay: context={context:?}, focused={}",
                mobile.focused
            );
        }
        return;
    }
    if qa.stage >= FILES.len() || qa.pending || qa.frames < if qa.stage == 0 { 180 } else { 45 } {
        return;
    }
    if roots.iter().any(|(scene, instance)| {
        instance.is_none()
            || !matches!(
                assets.recursive_dependency_load_state(scene.0.id()),
                bevy::asset::RecursiveDependencyLoadState::Loaded
            )
    }) {
        return;
    }
    let path = qa.directory.join(FILES[qa.stage]);
    qa.pending = true;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(readback);
}
fn readback(shot: On<ScreenshotCaptured>, mut qa: ResMut<Qa>, mut exit: MessageWriter<AppExit>) {
    if shot.image.width() != 1180 || shot.image.height() != 820 {
        exit.write(AppExit::error());
        return;
    }
    qa.pending = false;
    qa.stage += 1;
    qa.frames = 0;
    if qa.stage == FILES.len() {
        let report = serde_json::json!({"presentation_fixture":true,"physical_device_verified":false,"viewport":[1180,820],"language":"en","files":FILES});
        let _ = std::fs::write(
            qa.directory.join("capture.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
        exit.write(AppExit::Success);
    }
}
