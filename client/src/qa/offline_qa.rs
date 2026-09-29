//! Opt-in native smoke. Synthetic actions use production UI/network handlers;
//! screenshots come from the real renderer, without starting any server.
use crate::{
    frontend::AppScreen,
    net::{ClientSession, NetworkCommand, NetworkPlayerId, SessionUiCommand, TargetId, TargetKind},
    pause_menu::PauseMenuState,
    player::{MovementTarget, Player},
};
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) struct OfflineQaPlugin;
#[derive(Resource)]
struct Qa {
    directory: PathBuf,
    stage: u8,
    started: Instant,
    since: Instant,
    origin: Vec3,
    moved: f32,
    hit: bool,
    restored: bool,
    expected_server: String,
    target_id: u64,
    kit_feedback: bool,
    reentered: bool,
}
impl Plugin for OfflineQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_OFFLINE_SMOKE_DIR").map(PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&directory).expect("QA directory");
        app.insert_resource(Qa {
            directory,
            stage: 0,
            started: Instant::now(),
            since: Instant::now(),
            origin: Vec3::ZERO,
            moved: 0.0,
            hit: false,
            restored: false,
            target_id: 0,
            kit_feedback: false,
            reentered: false,
            expected_server: std::env::var("GAME_SERVER_ADDR").unwrap_or_default(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(PreUpdate, (focus, drive.after(bevy::ui::UiSystems::Focus)));
    }
}
fn focus(windows: Query<Entity, With<PrimaryWindow>>, _main: bevy::ecs::system::NonSendMarker) {
    let Ok(entity) = windows.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(w) = windows.get_window(entity)
            && !w.has_focus()
        {
            w.focus_window();
        }
    });
}
fn capture(world: &mut World, qa: &Qa, name: &str) {
    // Reuse already verified states when resuming a failed smoke in this directory.
    if qa.directory.join(name).exists() {
        return;
    }
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(name)));
}
/// Presses the UI-kit button carrying `id` with a `SyntheticPress`.
fn press(world: &mut World, id: &str) -> bool {
    let entity = world
        .query_filtered::<(Entity, &crate::ui::TestId), With<crate::ui::Pressable>>()
        .iter(world)
        .find(|(_, test_id)| test_id.as_str() == id)
        .map(|(entity, _)| entity);
    entity
        .map(|entity| world.write_message(crate::ui::SyntheticPress(entity)))
        .is_some()
}
fn drive(world: &mut World) {
    let Some(mut qa) = world.remove_resource::<Qa>() else {
        return;
    };
    if qa.started.elapsed() > Duration::from_secs(180) {
        std::fs::write(
            qa.directory.join("result.json"),
            format!("{{\"pass\":false,\"timeout_stage\":{}}}", qa.stage),
        )
        .unwrap();
        world.write_message(AppExit::error());
        return;
    }
    if qa.stage == 0 {
        if let Ok(mut window) = world
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .single_mut(world)
        {
            let width = std::env::var("OMOBA_QA_WIDTH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1180);
            let height = std::env::var("OMOBA_QA_HEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(820);
            window.resolution.set(width as f32, height as f32);
        }
    }
    let screen = *world.resource::<State<AppScreen>>().get();
    // First-run accounts show the normal controls guide after admission.
    // Dismiss it through its production button before attempting gameplay.
    if qa.stage == 5
        && world
            .resource::<crate::help_overlay::HelpOverlayVisible>()
            .0
    {
        press(world, "HelpDismissButton");
        qa.since = Instant::now();
        world.insert_resource(qa);
        return;
    }
    let age = qa.since.elapsed().as_secs_f32();
    let mut advance = false;
    match qa.stage {
        0 if age > 4.0 && screen == AppScreen::Home => {
            capture(world, &qa, "01-home.png");
            advance = true;
        }
        1 if age > 0.5 => {
            advance = press(world, "HomeOfflinePractice");
        }
        2 if age > 4.0 && screen == AppScreen::HeroSelect => {
            press(world, "ClassButton-dawnweaver");
            advance = true;
        }
        3 if age > 2.0 => {
            capture(world, &qa, "02-hero-picker.png");
            advance = true;
        }
        4 if age > 0.5 => {
            advance = press(world, "FindMatchButton");
        }
        5 if age > 12.0 && screen == AppScreen::InMatch => {
            if let Ok((entity, t)) = world
                .query_filtered::<(Entity, &Transform), With<Player>>()
                .single(world)
            {
                qa.origin = t.translation;
                let destination = t.translation + Vec3::new(4.0, 0.0, 3.0);
                world.entity_mut(entity).insert(MovementTarget {
                    target: destination,
                });
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::ClearBots,
                    ),
                ));
                world.write_message(NetworkCommand::Debug(
                    shared::debug::DebugCommand::Practice(
                        shared::practice::PracticeCommand::SpawnDummy,
                    ),
                ));
                capture(world, &qa, "03-practice.png");
                advance = true;
            }
        }
        6 if age > 3.0 => {
            if let Ok(t) = world
                .query_filtered::<&Transform, With<Player>>()
                .single(world)
            {
                qa.moved = t.translation.distance(qa.origin);
            }
            let dummy = world
                .query::<(
                    Entity,
                    &NetworkPlayerId,
                    &Transform,
                    &crate::combat::CombatStats,
                )>()
                .iter(world)
                .find(|(_, id, _, stats)| id.0 != 1 && stats.max_hp >= shared::debug::DUMMY_MAX_HP)
                .map(|(e, id, t, _)| (e, id.0, t.translation));
            if let Some((entity, id, position)) = dummy {
                qa.target_id = id;
                let mut target = world.resource_mut::<crate::combat::TargetState>();
                target.selected_entity = Some(entity);
                target.selected_target = Some(TargetId {
                    kind: TargetKind::Player,
                    id,
                });
                world.write_message(NetworkCommand::CastSkill {
                    slot: 0,
                    aim: Vec2::new(position.x, position.z),
                });
            }
            advance = true;
        }
        7 if age > 1.0 => {
            qa.hit = world
                .query::<(&NetworkPlayerId, &crate::combat::CombatStats)>()
                .iter(world)
                .any(|(id, c)| id.0 == qa.target_id && c.hp < c.max_hp);
            qa.kit_feedback = world
                .query::<(
                    &NetworkPlayerId,
                    &crate::net::PlayerLoadout,
                    &crate::net::PlayerSkillCooldowns,
                )>()
                .iter(world)
                .any(|(id, kit, cooldowns)| {
                    id.0 == 1 && kit.0.is_some() && cooldowns.remaining_secs[0] > 0.0
                });
            capture(world, &qa, "04-attacked-target.png");
            advance = true;
        }
        8 if age > 0.5 => {
            world.resource_mut::<PauseMenuState>().open = true;
            advance = true;
        }
        9 if age > 1.0 => {
            capture(world, &qa, "05-game-menu.png");
            advance = true;
        }
        10 if age > 0.5 => {
            advance = press(world, "PauseMenuPracticeButton");
        }
        11 if age > 1.0 => {
            capture(world, &qa, "06-practice-controls.png");
            advance = true;
        }
        12 if age > 0.5 => {
            advance = press(world, "PauseMenuPracticeGodModeButton");
        }
        13 if age > 0.5 => {
            advance = true;
        }
        14 if age > 0.5 => {
            world.resource_mut::<PauseMenuState>().open = false;
            world.write_message(SessionUiCommand::LeaveMatch);
            advance = true;
        }
        15 if age > 2.0 && screen == AppScreen::Home => {
            qa.restored = world.resource::<ClientSession>().server_addr() == qa.expected_server
                && !world.resource::<ClientSession>().is_offline()
                && world
                    .query_filtered::<Entity, With<Player>>()
                    .iter(world)
                    .count()
                    == 0;
            capture(world, &qa, "08-return-home.png");
            advance = true;
        }
        16 if age > 0.5 => {
            advance = press(world, "HomeOfflinePractice");
        }
        17 if age > 2.0 && screen == AppScreen::HeroSelect => {
            press(world, "ClassButton-wildspark");
            advance = true;
        }
        18 if age > 1.0 => {
            advance = press(world, "FindMatchButton");
        }
        19 if age > 5.0 && screen == AppScreen::InMatch => {
            qa.reentered = world.resource::<ClientSession>().is_offline()
                && world.resource::<ClientSession>().join_confirmed()
                && world
                    .query_filtered::<&crate::net::NetworkHeroClass, With<Player>>()
                    .iter(world)
                    .any(|class| class.0 == shared::HeroClass::Wildspark);
            capture(world, &qa, "09-reentered-wildspark.png");
            advance = true;
        }
        20 if age > 1.0 => {
            world.write_message(SessionUiCommand::LeaveMatch);
            advance = true;
        }
        21 if age > 2.0 && screen == AppScreen::Home => {
            let files = [
                "01-home.png",
                "02-hero-picker.png",
                "03-practice.png",
                "04-attacked-target.png",
                "05-game-menu.png",
                "06-practice-controls.png",
                "09-reentered-wildspark.png",
                "08-return-home.png",
            ];
            let passed = qa.moved > 1.0
                && qa.hit
                && qa.restored
                && qa.kit_feedback
                && qa.reentered
                && files.iter().all(|f| qa.directory.join(f).is_file());
            let report = serde_json::json!({"pass":passed,"standard_kit_cooldown_feedback":qa.kit_feedback,"reentered_wildspark":qa.reentered,"moved_metres":qa.moved,"target_damaged":qa.hit,"returned_home_and_restored_online":qa.restored,"screenshots":files,"expected_online_endpoint":qa.expected_server,"restored_online_endpoint":world.resource::<ClientSession>().server_addr(),"physical_ipad_verified":false,"method":"Native renderer, production UI and local packet handlers, synthetic actions; one English 852x393 viewport."});
            std::fs::write(
                qa.directory.join("result.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            world.write_message(if passed {
                AppExit::Success
            } else {
                AppExit::error()
            });
            return;
        }
        _ => {}
    }
    if advance {
        info!("Offline QA completed stage {}", qa.stage);
        qa.stage += 1;
        qa.since = Instant::now();
    }
    world.insert_resource(qa);
}
