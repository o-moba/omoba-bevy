//! Opt-in native presentation fixtures, never a claim of transport correctness.
use super::*;
use crate::frontend::{AppScreen, ScreenDriverPaused};
use crate::net::recovery::{ResumeMatchState, SavedResume};
use crate::net::session::CommittedJoin;
use crate::net::{ClientConnectionState, GameStateSnapshot};
use bevy::{
    app::AppExit,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::PrimaryWindow,
};
use std::{path::PathBuf, time::Instant};

const FILES: [&str; 4] = [
    "01-home-resume.png",
    "02-disconnected-home.png",
    "03-team-control-collapsed.png",
    "04-team-control-expanded.png",
];
#[derive(Resource)]
struct Capture {
    directory: PathBuf,
    stage_started: Instant,
    stage: usize,
    frames: u32,
    in_flight: bool,
    completed: bool,
    toggled: bool,
    help_dismiss_requested: bool,
}
pub(super) fn configure(app: &mut App) {
    let Some(directory) = std::env::var_os("OMOBA_RECOVERY_QA_OUTPUT").filter(|v| !v.is_empty())
    else {
        return;
    };
    app.insert_resource(Capture {
        directory: directory.into(),
        stage_started: Instant::now(),
        stage: 0,
        frames: 0,
        in_flight: false,
        completed: false,
        toggled: false,
        help_dismiss_requested: false,
    })
    .insert_resource(ScreenDriverPaused(true))
    .insert_resource(bevy::winit::WinitSettings::continuous())
    .add_systems(Startup, label)
    .add_systems(
        Update,
        drive
            .after(crate::net::ClientNetPipeline::SessionLifecycle)
            .before(super::render),
    )
    .add_systems(PostUpdate, shoot.after(bevy::ui::UiSystems::Layout));
}
fn label(mut commands: Commands) {
    commands.spawn((
        Text::new("SYNTHETIC RECOVERY UI · local fixture"),
        TextFont {
            font_size: 12.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(4.0),
            left: Val::Px(8.0),
            ..default()
        },
        GlobalZIndex(3000),
    ));
}
fn join() -> CommittedJoin {
    CommittedJoin {
        handheld: Default::default(),
        prematch: true,
        team: crate::team::Team::Blue,
        character: shared::wire::CharacterChoice::Ipfs,
        hero_class: shared::HeroClass::Mage,
        avatar: None,
        sprite_character: None,
    }
}
#[allow(clippy::too_many_arguments)]
fn drive(
    mut qa: ResMut<Capture>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut session: ResMut<ClientSession>,
    mut career: ResMut<crate::career::CareerClient>,
    mut resume: ResMut<ResumeMatchState>,
    mut game: ResMut<GameStateSnapshot>,
    mut ui: ResMut<RecoveryUi>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut presses: crate::qa::TestIdPresses,
    modals: Option<Res<ModalStack>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.stage_started.elapsed().as_secs() > 60 {
        error!(
            "RECOVERY_QA timeout stage {}: frames={}, in_flight={}, completed={}, screen={:?}, modal={:?}",
            qa.stage,
            qa.frames,
            qa.in_flight,
            qa.completed,
            screen.get(),
            modals.as_ref().and_then(|stack| stack.top())
        );
        exit.write(AppExit::error());
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(852, 393);
    }
    let wanted = if qa.stage == 0 {
        AppScreen::Home
    } else {
        AppScreen::InMatch
    };
    if *screen.get() != wanted {
        next.set(wanted);
    }
    session.state = if qa.stage == 1 {
        ClientConnectionState::Disconnected
    } else {
        ClientConnectionState::Connected
    };
    session.admitted = qa.stage >= 2;
    session.last_join = (qa.stage != 0).then(join);
    session.join_flow_committed = qa.stage >= 2;
    session.last_qualifying_snapshot_wall = Some(Instant::now());
    session.reconnect = Default::default();
    game.state = shared::wire::GameState::Running;
    game.meta = shared::protocol::SnapshotMeta::new(71, 1, 1);
    career.view.takeovers = if qa.stage >= 2 {
        (1..=4)
            .map(|id| TakeoverSeatView {
                player_id: id,
                generation: 2,
                nickname: format!("Teammate {id}"),
                policy: TakeoverPolicy::Bot,
                idle_votes: 0,
                bot_votes: 1,
                eligible_voters: 2,
                my_vote: Some(TakeoverPolicy::Bot),
            })
            .collect()
    } else {
        Vec::new()
    };
    if qa.stage == 0 && resume.saved.is_none() {
        resume.saved = Some(SavedResume {
            allocation: shared::match_service::MatchAllocation {
                allocation_id: "a".repeat(32),
                endpoint: "127.0.0.1:41000".into(),
                preference: Default::default(),
                team: shared::map::Team::Blue,
                human_count: 5,
                bot_count: 5,
                rated: false,
                join_deadline_ms: 1,
            },
            lobby: "127.0.0.1:4000".into(),
            session_id: "synthetic-fixture".into(),
            server_epoch: 71,
            match_id: 1,
            join: join(),
        });
    }
    if qa.stage < 3 {
        ui.expanded = false;
    }
    let modal = modals.as_ref().and_then(|stack| stack.top());
    // First admission opens the real controls guide. Dismiss its real button
    // once through the recognizer; never alter or bypass the modal resources.
    if qa.stage >= 2
        && modal == Some(crate::ui::ModalId::Help)
        && !qa.help_dismiss_requested
        && presses.press("HelpDismissButton")
    {
        qa.help_dismiss_requested = true;
        warn!("RECOVERY_QA pressed the first-match guide's dismiss button");
    }
    if qa.stage == 3 && modal.is_none() && !qa.toggled && presses.press("TakeoverToggle") {
        qa.toggled = true;
    }
}
fn shoot(
    mut commands: Commands,
    mut qa: ResMut<Capture>,
    screen: Res<State<AppScreen>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    nodes: Query<(
        &crate::ui::TestId,
        &ComputedNode,
        &UiGlobalTransform,
        &crate::ui::Pressable,
    )>,
    status: Query<
        (&ComputedNode, &UiGlobalTransform, &Node, &Visibility),
        With<crate::net::status_ui::ConnectionStatusRoot>,
    >,
    modals: Option<Res<ModalStack>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(file) = FILES.get(qa.stage) else {
        return;
    };
    if qa.in_flight {
        if qa.completed
            && qa
                .directory
                .join(file)
                .metadata()
                .is_ok_and(|m| m.len() > 32)
        {
            warn!("RECOVERY_QA captured {file}");
            qa.stage += 1;
            qa.stage_started = Instant::now();
            qa.frames = 0;
            qa.in_flight = false;
            qa.completed = false;
            if qa.stage == FILES.len() {
                warn!("RECOVERY_QA complete");
                if let Ok(window) = windows.single() {
                    commands.entity(window).despawn();
                }
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    if *screen.get()
        != if qa.stage == 0 {
            AppScreen::Home
        } else {
            AppScreen::InMatch
        }
    {
        return;
    }
    qa.frames += 1;
    // The control and physical bounds below are the readiness gate. A long
    // frame-count delay consumes minutes under macOS background throttling.
    if qa.frames < 8 {
        return;
    }
    let expected = [
        "HomePlay",
        "RecoveryHome",
        "TakeoverToggle",
        "Takeover1Idle",
    ][qa.stage];
    let Some((_, node, transform, pressable)) =
        nodes.iter().find(|(id, ..)| id.as_str() == expected)
    else {
        if qa.frames.is_multiple_of(8) {
            warn!(
                "RECOVERY_QA waiting for {expected} on {:?}; available controls: {:?}",
                screen.get(),
                nodes.iter().map(|(id, ..)| id.as_str()).collect::<Vec<_>>()
            );
        }
        return;
    };
    let rect = physical_rect(node, transform);
    let size = rect.size();
    if pressable.disabled
        || pressable.blocked
        || modals.as_ref().is_some_and(|stack| stack.top().is_some())
        || size.x < 40.0
        || size.y < 43.0
        || rect.min.x < 0.0
        || rect.min.y < 0.0
        || rect.max.x > 852.5
        || rect.max.y > 393.5
    {
        error!("RECOVERY_QA invalid hit target {expected}: {rect:?}");
        exit.write(AppExit::error());
        return;
    }
    let banner = if qa.stage == 1 {
        let Ok((node, transform, style, visibility)) = status.single() else {
            error!("RECOVERY_QA missing actual connection banner");
            exit.write(AppExit::error());
            return;
        };
        let banner = physical_rect(node, transform);
        if style.display == Display::None
            || *visibility == Visibility::Hidden
            || banner.height() <= 0.0
            || rect.min.y < banner.max.y + 8.0
        {
            error!(
                "RECOVERY_QA Home must be below visible banner: home={rect:?}, banner={banner:?}"
            );
            exit.write(AppExit::error());
            return;
        }
        Some([banner.min.x, banner.min.y, banner.max.x, banner.max.y])
    } else {
        None
    };
    if std::fs::create_dir_all(&qa.directory).is_err() {
        exit.write(AppExit::error());
        return;
    }
    let report = serde_json::json!({ "synthetic": true, "control": expected, "bounds": [rect.min.x, rect.min.y, rect.max.x, rect.max.y], "pressable": true, "modal_open": false, "help_dismiss_requested": qa.help_dismiss_requested, "connection_banner_bounds": banner });
    let _ = std::fs::write(
        qa.directory.join(format!("{file}.json")),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(|_: On<ScreenshotCaptured>, mut qa: ResMut<Capture>| {
            qa.completed = true;
        });
    qa.in_flight = true;
}
