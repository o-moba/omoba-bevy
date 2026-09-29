//! One English desktop viewport; live casts and screenshot readbacks, not manual input proof.
use crate::{
    frontend::{AppScreen, ScreenDriverPaused},
    net::{ClientSession, GameStateSnapshot, NetworkCommand, NetworkHeroClass, PlayerLoadout},
    player::Player,
    team::{CharacterChoice, Team, TeamSelection},
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use shared::{
    HeroClass,
    sandbox::{ActorConfig, SandboxCommand, SandboxConfig, SandboxRequest},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
const CLASSES: [HeroClass; 9] = [
    HeroClass::Cinderforge,
    HeroClass::Edgeweaver,
    HeroClass::Stormfist,
    HeroClass::Veilstalker,
    HeroClass::Emberveil,
    HeroClass::Orbitwright,
    HeroClass::Riftshot,
    HeroClass::Chainkeeper,
    HeroClass::Frostguard,
];
pub(crate) struct RosterQaPlugin;
impl Plugin for RosterQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = std::env::var_os("OMOBA_ROSTER_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let followup = std::env::var_os("OMOBA_ROSTER_QA_FOLLOWUP").is_some();
        let inspection = std::env::var_os("OMOBA_ROSTER_QA_INSPECTION").is_some();
        let classes = if inspection {
            vec![HeroClass::Dawnweaver]
        } else if followup {
            vec![
                HeroClass::Cinderforge,
                HeroClass::Stormfist,
                HeroClass::Veilstalker,
                HeroClass::Emberveil,
                HeroClass::Chainkeeper,
            ]
        } else {
            CLASSES.to_vec()
        };
        app.insert_resource(Qa {
            dir,
            classes,
            followup,
            inspection,
            hold_started: None,
            hold_phase: 0,
            stage: 0,
            index: 0,
            frames: 0,
            started: Instant::now(),
            readbacks: vec![],
            captures: vec![],
            request: 0,
            cast_ack: 0,
        })
        .insert_resource(ScreenDriverPaused(true))
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(
            PreUpdate,
            (
                super::combat_qa::focus_capture_window,
                prepare.after(bevy::ui::UiSystems::Focus),
            ),
        )
        .add_systems(
            PostUpdate,
            step.after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
#[derive(Resource)]
struct Qa {
    dir: PathBuf,
    classes: Vec<HeroClass>,
    followup: bool,
    inspection: bool,
    hold_started: Option<Instant>,
    hold_phase: usize,
    stage: u8,
    index: usize,
    frames: u32,
    started: Instant,
    readbacks: Vec<String>,
    captures: Vec<serde_json::Value>,
    request: u64,
    cast_ack: u64,
}
fn prepare(
    mut qa: ResMut<Qa>,
    mut selection: ResMut<TeamSelection>,
    mut screen: ResMut<NextState<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    help: Res<crate::help_overlay::HelpOverlayVisible>,
    mut buttons: crate::qa::TestIdPresses,
    sandbox: Res<crate::sandbox::SandboxClient>,
) {
    if let Ok(mut w) = windows.single_mut() {
        w.resolution.set_scale_factor_override(Some(1.0));
        if w.physical_width() != 1280 || w.physical_height() != 720 {
            w.resolution.set_physical_resolution(1280, 720);
        }
    }
    if qa.stage == 0 {
        selection.hero_class = qa.classes[0];
        selection.character = CharacterChoice::Cube;
        selection.avatar = Some("agnes".into());
        screen.set(AppScreen::HeroSelect);
        qa.stage = 1;
    }
    if help.0 {
        buttons.press("HelpDismissButton");
    }
    if sandbox.enabled && sandbox.open {
        if sandbox.overlay {
            if !buttons.press("CombatTestToggle-Overlay") {
                buttons.press("CombatTestTab-World");
            }
        } else {
            buttons.press("CombatTestClose");
        }
    }
}
fn step(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    local: Query<(&Transform, &NetworkHeroClass, &PlayerLoadout), With<Player>>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
    context: Res<crate::input_context::GameplayInputContext>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    inspected: Res<crate::combat::inspection::SkillInspection>,
    cards: Query<&crate::combat::skill_card::SkillCardView>,
    panels: Query<(&Name, &Node)>,
) {
    if qa.stage == 255 {
        return;
    }
    qa.frames += 1;
    if qa.started.elapsed() > Duration::from_secs(180) {
        let _ = std::fs::write(
            qa.dir.join("failure.json"),
            serde_json::json!({"stage":qa.stage,"index":qa.index,"captures":qa.captures})
                .to_string(),
        );
        qa.stage = 255;
        exit.write(AppExit::error());
        return;
    }
    match qa.stage {
        1 if *screen.get() == AppScreen::HeroSelect && qa.frames > 150 => {
            if qa.followup || qa.inspection {
                qa.readbacks.push("selection-reused-from-native-1".into());
                qa.stage = 2;
                return;
            }
            capture(
                &mut commands,
                &mut qa,
                "00-selection".into(),
                serde_json::json!({"screen":"HeroSelect","classes":HeroClass::ALL.map(|c|c.id())}),
            );
            qa.stage = 2;
        }
        2 if qa.readbacks.len() == 1 && session.is_connected() => {
            outgoing.write(NetworkCommand::Join {
                team: Team::Green,
                character: CharacterChoice::Cube,
                hero_class: qa.classes[0],
                avatar: Some("agnes".into()),
                sprite_character: None,
            });
            next.set(AppScreen::InMatch);
            qa.stage = 3;
        }
        3 if session.join_confirmed() && snapshot.sandbox.is_some() => {
            let class = qa.classes[qa.index];
            let mut config = SandboxConfig {
                player: ActorConfig {
                    hero: class,
                    avatar: Some("agnes".into()),
                    level: 10,
                    position: [-8.0, -8.0],
                    max_hp: shared::hero_balance::base_hp(class),
                    ..default()
                },
                ..default()
            };
            config.dummy.enabled = true;
            config.dummy.position = if qa.followup && class == HeroClass::Chainkeeper {
                [-3.0, -8.0]
            } else {
                [-2.0, -8.0]
            };
            config.environment.minions = false;
            qa.request = snapshot.sandbox.as_ref().unwrap().last_request_id + 1;
            outgoing.write(NetworkCommand::Sandbox(SandboxRequest {
                server_epoch: snapshot.meta.server_epoch,
                match_id: snapshot.meta.match_id,
                request_id: qa.request,
                command: SandboxCommand::ApplyConfig { config },
            }));
            qa.stage = 4;
            qa.frames = 0;
        }
        4 if qa.frames > 20
            && snapshot.sandbox.as_ref().is_some_and(|s| {
                s.ack
                    .as_ref()
                    .is_some_and(|a| a.request_id == qa.request && a.accepted)
            }) =>
        {
            let Ok((pose, class, loadout)) = local.single() else {
                return;
            };
            if class.0 != qa.classes[qa.index] || !context.gameplay_allowed() {
                return;
            }
            if qa.inspection {
                keys.press(KeyCode::KeyQ);
                qa.hold_started = Some(Instant::now());
                qa.stage = 5;
                return;
            }
            qa.cast_ack = loadout.0.as_ref().map_or(0, |s| s.cast_request_id);
            let slot = if qa.followup && class.0 == HeroClass::Chainkeeper {
                3
            } else {
                match class.0 {
                    HeroClass::Edgeweaver | HeroClass::Stormfist | HeroClass::Chainkeeper => 1,
                    HeroClass::Emberveil | HeroClass::Riftshot => 3,
                    HeroClass::Frostguard => 2,
                    _ => 0,
                }
            };
            let aim = pose.translation.xz()
                + Vec2::new(
                    if class.0 == HeroClass::Emberveil {
                        3.0
                    } else {
                        6.0
                    },
                    0.0,
                );
            outgoing.write(NetworkCommand::CastSkill { slot, aim });
            qa.frames = 0;
            qa.stage = 5;
        }
        5 if qa.inspection => {
            let elapsed = qa.hold_started.unwrap().elapsed().as_secs_f32();
            let expected = (qa.hold_phase + 1).min(3);
            if qa.readbacks.len() < expected {
                return;
            }
            let phase = qa.hold_phase;
            let ready = match phase {
                0 => elapsed >= 0.3,
                1 => elapsed >= 1.8,
                2 => {
                    keys.release(KeyCode::KeyQ);
                    qa.frames = 0;
                    qa.hold_phase = 3;
                    false
                }
                3 => qa.frames >= 4,
                _ => false,
            };
            if ready {
                let shown = phase == 1;
                let card = cards.iter().any(|c| c.visible);
                let panel = panels
                    .iter()
                    .any(|(n, p)| n.as_str() == "StandardKitStatus" && p.display != Display::None);
                if inspected.slot.is_some() != shown || card != shown || panel != shown {
                    let _ = std::fs::write(qa.dir.join("failure.json"),
                        serde_json::json!({"phase":phase,"inspection":inspected.slot,"card":card,"panel":panel}).to_string());
                    qa.stage = 255;
                    exit.write(AppExit::error());
                    return;
                }
                let name = match phase {
                    0 => "01-short-hold",
                    1 => "02-long-hold",
                    _ => "03-released",
                };
                capture(
                    &mut commands,
                    &mut qa,
                    name.into(),
                    serde_json::json!({"held_seconds":elapsed,"description_visible":shown,"card_visible":card,"panel_visible":panel}),
                );
                qa.hold_phase += 1;
            }
            if qa.hold_phase == 4 && qa.readbacks.len() == 4 {
                let _ = std::fs::write(qa.dir.join("summary.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({"pass":true,"locale":"en","viewport":[1280,720],"scripted":true,"captures":qa.captures})).unwrap());
                qa.stage = 255;
                exit.write(AppExit::Success);
            }
        }
        5 if qa.frames >= 12 => {
            let Ok((_, class, loadout)) = local.single() else {
                return;
            };
            let Some(state) = loadout
                .0
                .as_ref()
                .filter(|s| s.cast_request_id > qa.cast_ack)
            else {
                return;
            };
            let index = qa.index;
            let file = format!("{:02}-{}", index + 1, class.0.id());
            capture(
                &mut commands,
                &mut qa,
                file,
                serde_json::json!({"class":class.0.id(),"snapshot_tick":snapshot.meta.snapshot_tick,"loadout":state,"effects":snapshot.skill_effects,"input":"scripted normal CastSkill"}),
            );
            qa.stage = 6;
        }
        6 if qa.readbacks.len() == qa.index + 2 => {
            qa.index += 1;
            if qa.index < qa.classes.len() {
                qa.stage = 3;
            } else {
                let summary = serde_json::json!({"pass":true,"locale":"en","viewport":[1280,720],"scripted":true,"manual_input":false,"physical_device":false,"captures":qa.captures});
                let _ = std::fs::write(
                    qa.dir.join("summary.json"),
                    serde_json::to_vec_pretty(&summary).unwrap(),
                );
                qa.stage = 255;
                exit.write(AppExit::Success);
            }
        }
        _ => {}
    }
}
#[derive(Component)]
struct Shot(String);
fn capture(commands: &mut Commands, qa: &mut Qa, name: String, mut data: serde_json::Value) {
    let file = format!("{name}.png");
    data["file"] = file.clone().into();
    qa.captures.push(data);
    commands
        .spawn((Screenshot::primary_window(), Shot(file.clone())))
        .observe(save_to_disk(qa.dir.join(file)))
        .observe(readback);
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Qa>) {
    if let Ok(shot) = shots.get(event.entity)
        && event.image.width() == 1280
        && event.image.height() == 720
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0.clone());
    }
}
