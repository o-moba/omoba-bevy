//! Focused, opt-in native captures using live sandbox setup and normal skill commands.
use crate::{
    frontend::{AppScreen, ScreenDriverPaused},
    help_overlay::HelpOverlayVisible,
    net::{
        ClientSession, GameStateSnapshot, NetworkCommand, NetworkHeroClass, PlayerLoadout,
        PlayerProgression,
    },
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
    loadout::{EffectVisualKind, WeaponMode},
    sandbox::{ActorConfig, SandboxCommand, SandboxConfig, SandboxRequest},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) struct StandardKitsQaPlugin;
impl Plugin for StandardKitsQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_STANDARD_QA_DIR").map(PathBuf::from) else {
            return;
        };
        let class = std::env::var("OMOBA_STANDARD_QA_CLASS")
            .ok()
            .and_then(|s| HeroClass::from_id(&s))
            .filter(|c| c.is_standard())
            .unwrap_or(HeroClass::Dawnweaver);
        app.insert_resource(Qa {
            directory,
            class,
            ux: std::env::var_os("OMOBA_COMBAT_UX_QA").is_some(),
            hud: std::env::var_os("OMOBA_COMBAT_UX_HUD_QA").is_some(),
            stage: 0,
            prepared: false,
            frames: 0,
            aim_ack: 0,
            started: Instant::now(),
            captures: Vec::new(),
            readbacks: Vec::new(),
            requests: Vec::new(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .insert_resource(ScreenDriverPaused(true))
        .add_systems(Startup, label)
        .add_systems(
            PreUpdate,
            (
                super::combat_qa::focus_capture_window,
                prepare
                    .after(bevy::ui::UiSystems::Focus)
                    .after(bevy::input::InputSystems),
            ),
        )
        .add_systems(
            PostUpdate,
            observe
                .after(bevy::ui::UiSystems::Layout)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    class: HeroClass,
    ux: bool,
    hud: bool,
    stage: u8,
    prepared: bool,
    frames: u32,
    aim_ack: u64,
    started: Instant,
    captures: Vec<serde_json::Value>,
    readbacks: Vec<usize>,
    requests: Vec<serde_json::Value>,
}
fn label(mut commands: Commands) {
    commands.spawn((
        Text::new("QA · live sandbox · scripted commands and held-key input"),
        TextFont {
            font_size: 10.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::BLACK),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(3.0),
            left: Val::Percent(35.0),
            ..default()
        },
        ZIndex(200),
        bevy::ui::FocusPolicy::Pass,
    ));
}
fn prepare(
    mut qa: ResMut<Qa>,
    mut selection: ResMut<TeamSelection>,
    mut screen: ResMut<NextState<AppScreen>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    help: Res<HelpOverlayVisible>,
    mut buttons: crate::qa::TestIdPresses,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    sandbox: Res<crate::sandbox::SandboxClient>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        let (width, height) = if qa.ux { (1180, 820) } else { (1280, 720) };
        if window.physical_width() != width || window.physical_height() != height {
            window.resolution.set_physical_resolution(width, height);
        }
        if (9..=11).contains(&qa.stage) {
            window.set_cursor_position(Some(Vec2::new(850.0, 300.0)));
            let key = if qa.ux {
                KeyCode::KeyR
            } else if qa.class == HeroClass::Dawnweaver {
                KeyCode::KeyQ
            } else {
                KeyCode::KeyW
            };
            if qa.stage == 11 {
                keyboard.release(key);
            } else {
                keyboard.press(key);
            }
        }
    }
    if !qa.prepared {
        qa.prepared = true;
        selection.hero_class = qa.class;
        selection.character = CharacterChoice::Cube;
        selection.avatar = Some("agnes".into());
        screen.set(AppScreen::HeroSelect);
    }
    if help.0 {
        buttons.press("HelpDismissButton");
    }
    if qa.stage >= 2 && sandbox.enabled && sandbox.open {
        if sandbox.overlay {
            if !buttons.press("CombatTestToggle-Overlay") {
                buttons.press("CombatTestTab-World");
            }
        } else {
            buttons.press("CombatTestClose");
        }
    }
}
fn observe(
    mut commands: Commands,
    mut qa: ResMut<Qa>,
    snapshot: Res<GameStateSnapshot>,
    session: Res<ClientSession>,
    screen: Res<State<AppScreen>>,
    mut next_screen: ResMut<NextState<AppScreen>>,
    local: Query<
        (
            &Transform,
            &NetworkHeroClass,
            &PlayerProgression,
            &PlayerLoadout,
        ),
        With<Player>,
    >,
    cooldowns: Query<&crate::net::PlayerSkillCooldowns, With<Player>>,
    nodes: Query<(crate::qa::QaName, &ComputedNode, &InheritedVisibility)>,
    help: Res<HelpOverlayVisible>,
    mut outgoing: MessageWriter<NetworkCommand>,
    mut exit: MessageWriter<AppExit>,
    assets: Res<AssetServer>,
    scenes: Query<&SceneRoot>,
    context: Res<crate::input_context::GameplayInputContext>,
) {
    if qa.stage == 255 {
        return;
    }
    if qa.started.elapsed() > Duration::from_secs(120) {
        fail(
            &mut qa,
            &mut exit,
            "Timed out waiting for live server state or screenshot readback",
        );
        return;
    }
    qa.frames += 1;
    let frame = serde_json::json!({"snapshot_tick":snapshot.meta.snapshot_tick,"server_epoch":snapshot.meta.server_epoch,"match_id":snapshot.meta.match_id,"screen":format!("{:?}", screen.get()),"gameplay_allowed":context.gameplay_allowed(),"scripted_aim_held":matches!(qa.stage,9|10),"effects":snapshot.skill_effects,"loadout":local.single().ok().and_then(|(_,_,_,l)|l.0.as_ref()),"nodes":nodes.iter().filter(|(n,_,_)| n.as_str().starts_with("ClassButton") || ["StandardKitStatus","HeroOverheadPlate","AlliedVitals","MinimapSkillVector","KillFeed","MinimapTrap"].contains(&n.as_str())).map(|(n,c,v)|serde_json::json!({"name":n.as_str(),"size":c.size().to_array(),"visible":v.get()})).collect::<Vec<_>>()});
    match qa.stage {
        0 if *screen.get() == AppScreen::HeroSelect && qa.frames >= 120 && !help.0 => {
            if scenes.is_empty()
                || !scenes.iter().all(|root| {
                    matches!(
                        assets.get_recursive_dependency_load_state(root.0.id()),
                        Some(bevy::asset::RecursiveDependencyLoadState::Loaded)
                    )
                })
            {
                return;
            }
            capture(&mut commands, &mut qa, 0, frame);
            qa.stage = 1;
        }
        1 if qa.readbacks.contains(&0) && session.is_connected() => {
            outgoing.write(NetworkCommand::Join {
                team: Team::Green,
                character: CharacterChoice::Cube,
                hero_class: qa.class,
                avatar: Some("agnes".into()),
                sprite_character: None,
            });
            let class_id = qa.class.id();
            qa.requests
                .push(serde_json::json!({"command":"join","class":class_id}));
            next_screen.set(AppScreen::InMatch);
            qa.stage = 2;
        }
        2 if session.join_confirmed() && local.single().is_ok() => {
            if qa.hud {
                qa.stage = 12;
                qa.frames = 0;
                return;
            }
            let Some(sandbox) = snapshot.sandbox.as_ref() else {
                return;
            };
            let mut config = SandboxConfig {
                player: ActorConfig {
                    hero: qa.class,
                    avatar: Some("agnes".into()),
                    level: 10,
                    no_cooldowns: qa.ux,
                    position: [-8.0, -8.0],
                    max_hp: shared::hero_balance::base_hp(qa.class),
                    ..default()
                },
                ..default()
            };
            config.dummy.enabled = true;
            config.dummy.position = [-2.0, -8.0];
            let request = SandboxRequest {
                server_epoch: snapshot.meta.server_epoch,
                match_id: snapshot.meta.match_id,
                request_id: sandbox.last_request_id.saturating_add(1),
                command: SandboxCommand::ApplyConfig { config },
            };
            qa.requests
                .push(serde_json::json!({"command":"sandbox_setup","request":request}));
            outgoing.write(NetworkCommand::Sandbox(request));
            qa.stage = 3;
            qa.frames = 0;
        }
        3 => {
            let Ok((pose, class, progression, _)) = local.single() else {
                return;
            };
            if class.0 != qa.class
                || progression.level != 10
                || qa.frames < 60
                || (qa.ux && qa.started.elapsed() < Duration::from_secs(14))
                || help.0
                || !context.gameplay_allowed()
            {
                return;
            }
            cast(
                &mut qa,
                &mut outgoing,
                2,
                pose.translation.xz() + Vec2::new(3.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 4;
        }
        4 => {
            if !cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) {
                return;
            }
            let kind = if qa.class == HeroClass::Dawnweaver {
                EffectVisualKind::Field
            } else {
                EffectVisualKind::Trap
            };
            if !snapshot
                .skill_effects
                .iter()
                .any(|e| e.kind == kind && e.armed)
            {
                return;
            }
            let Ok((pose, _, _, _)) = local.single() else {
                return;
            };
            let slot = if qa.class == HeroClass::Dawnweaver {
                1
            } else {
                0
            };
            cast(
                &mut qa,
                &mut outgoing,
                slot,
                pose.translation.xz() + Vec2::new(10.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 5;
        }
        5 => {
            let ready = if qa.class == HeroClass::Dawnweaver {
                snapshot
                    .skill_effects
                    .iter()
                    .any(|e| e.kind == EffectVisualKind::Barrier)
            } else {
                local
                    .single()
                    .ok()
                    .and_then(|(_, _, _, l)| l.0.as_ref())
                    .is_some_and(|s| s.weapon_mode == WeaponMode::Rockets)
            };
            if ready {
                capture(&mut commands, &mut qa, 1, frame);
                qa.stage = 6;
            }
        }
        6 if qa.readbacks.contains(&1) => {
            if !cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) {
                return;
            }
            let Ok((pose, _, _, _)) = local.single() else {
                return;
            };
            cast(
                &mut qa,
                &mut outgoing,
                3,
                pose.translation.xz() + Vec2::new(12.0, 0.0),
                snapshot.meta.snapshot_tick,
            );
            qa.stage = 7;
        }
        7 => {
            let kind = if qa.class == HeroClass::Dawnweaver {
                EffectVisualKind::BeamWarning
            } else {
                EffectVisualKind::Rocket
            };
            if snapshot.skill_effects.iter().any(|e| e.kind == kind) {
                capture(&mut commands, &mut qa, 2, frame);
                qa.stage = 8;
            }
        }
        8 if qa.readbacks.contains(&2)
            && context.gameplay_allowed()
            && cooldowns.single().is_ok_and(|cd| cd.recovery_secs <= 0.0) =>
        {
            qa.aim_ack = local
                .single()
                .ok()
                .and_then(|(_, _, _, l)| l.0.as_ref())
                .map_or(0, |l| l.cast_request_id);
            let key = if qa.ux {
                "R"
            } else if qa.class == HeroClass::Dawnweaver {
                "Q"
            } else {
                "W"
            };
            qa.requests
                .push(serde_json::json!({"input":"hold_key","key":key,"cursor":[850,300]}));
            qa.stage = 9;
            qa.frames = 0;
        }
        9 if qa.frames >= 5 && context.gameplay_allowed() => {
            capture(&mut commands, &mut qa, 3, frame);
            qa.stage = 10;
        }
        10 if qa.readbacks.contains(&3) => {
            qa.requests.push(serde_json::json!({"input":"release_key","snapshot_tick":snapshot.meta.snapshot_tick}));
            qa.stage = 11;
        }
        11 if snapshot.skill_effects.iter().any(|e| {
            (qa.ux && e.kind == EffectVisualKind::Rocket)
                || e.kind == EffectVisualKind::Bolt
                    && matches!(
                        e.skill,
                        shared::loadout::SkillId::DawnBind | shared::loadout::SkillId::WildZap
                    )
        }) && local
            .single()
            .ok()
            .and_then(|(_, _, _, l)| l.0.as_ref())
            .is_some_and(|l| l.cast_request_id > qa.aim_ack)
            && qa.readbacks.len() == 4
            && FILES
                .iter()
                .all(|f| qa.directory.join(f).metadata().is_ok_and(|m| m.len() > 32)) =>
        {
            let summary = serde_json::json!({"pass":true,"scenario":"standard_kits","class":qa.class.id(),"locale":"en","pixels":if qa.ux {vec![1180,820]} else {vec![1280,720]},"touch_hud":qa.ux,"scripted_commands":true,"scripted_key_release_accepted":true,"manual_interaction_verified":false,"physical_device_verified":false,"setup":if qa.ux {"live sandbox; level 10; rank 1; infinite dummy; cooldowns disabled; keyboard-held aim on touch HUD"} else {"live development sandbox; level 10; rank 1; stationary infinite-health dummy; normal costs, cooldowns and time"},"requests":qa.requests,"captures":qa.captures});
            if std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .is_err()
            {
                fail(&mut qa, &mut exit, "Cannot write evidence summary");
                return;
            }
            qa.stage = 255;
            exit.write(AppExit::Success);
        }
        12 if qa.frames > 90
            && nodes
                .iter()
                .any(|(n, c, v)| n.as_str() == "AlliedVitals" && v.get() && c.size().x > 0.0) =>
        {
            capture(&mut commands, &mut qa, 1, frame);
            qa.stage = 13;
        }
        13 if qa.readbacks.contains(&1)
            && nodes
                .iter()
                .any(|(n, c, v)| n.as_str() == "KillFeed" && v.get() && c.size().y > 0.0) =>
        {
            capture(&mut commands, &mut qa, 2, frame);
            qa.stage = 14;
        }
        14 if qa.readbacks.contains(&2) => {
            let summary = serde_json::json!({"pass":true,"scenario":"combat_ux_hud",
                "locale":"en","pixels":[1180,820],"touch_hud":true,
                "physical_device_verified":false,"manual_interaction_verified":false,
                "setup":"ordinary practice; authoritative lane bots; real hero kills; no synthetic damage or HUD state",
                "captures":qa.captures,"scoreboard":snapshot.scoreboard});
            std::fs::write(
                qa.directory.join("qa-summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .unwrap();
            qa.stage = 255;
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
fn cast(qa: &mut Qa, outgoing: &mut MessageWriter<NetworkCommand>, slot: u8, aim: Vec2, tick: u64) {
    qa.requests.push(serde_json::json!({"command":"cast_skill","slot":slot,"aim":aim.to_array(),"snapshot_tick":tick}));
    outgoing.write(NetworkCommand::CastSkill { slot, aim });
}
const FILES: [&str; 4] = [
    "01-selection.png",
    "02-persistent-effects.png",
    "03-ultimate-flight.png",
    "04-held-aim.png",
];
#[derive(Component)]
struct Shot(usize);
fn capture(commands: &mut Commands, qa: &mut Qa, index: usize, mut frame: serde_json::Value) {
    frame["file"] = FILES[index].into();
    qa.captures.push(frame);
    commands
        .spawn((Screenshot::primary_window(), Shot(index)))
        .observe(save_to_disk(qa.directory.join(FILES[index])))
        .observe(readback);
}
fn readback(event: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<Qa>) {
    if let Ok(shot) = shots.get(event.entity)
        && event.image.width() == if qa.ux { 1180 } else { 1280 }
        && event.image.height() == if qa.ux { 820 } else { 720 }
        && !qa.readbacks.contains(&shot.0)
    {
        qa.readbacks.push(shot.0);
    }
}
fn fail(qa: &mut Qa, exit: &mut MessageWriter<AppExit>, reason: &str) {
    let _=std::fs::write(qa.directory.join("qa-failure.json"),serde_json::to_vec_pretty(&serde_json::json!({"stage":qa.stage,"reason":reason,"captures":qa.captures,"requests":qa.requests})).unwrap());
    error!("STANDARD_KITS_QA stage={}: {reason}", qa.stage);
    qa.stage = 255;
    exit.write(AppExit::error());
}
