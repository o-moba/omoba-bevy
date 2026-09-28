//! Opt-in capture of the result screen's states and the loading shell's
//! failure (`OMOBA_RESULT_QA_OUTPUT=<dir>`), the Verdant Crown P0-B evidence
//! the other harnesses cannot reach without a finished career match.
//!
//! Run it without a server (point `GAME_SERVER_ADDR` at a closed port). The
//! result frames are labelled presentation fixtures — a synthetic Victory,
//! live score row and career receipt written into the client's own
//! resources, nothing sent — through the production screen, systems and
//! kit: finalizing, saved, a guest's defeat on a rematch server, abandoned.
//! The loading frames are real: the connecting body while the session
//! waits, then its failure once the session gives up (`T_WAIT_MAX`).
//! Size and profile come from `OMOBA_QA_WIDTH`/`OMOBA_QA_HEIGHT` and
//! `OMOBA_TOUCH_CONTROLS`; the language from `OMOBA_LANGUAGE`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    ui::FocusPolicy,
    window::PrimaryWindow,
};
use shared::career::{MatchOutcome, MatchResult, ProfileSummary};
use shared::live_score::{LiveScorePlayer, LiveScoreboard};

use crate::career::CareerClient;
use crate::frontend::AppScreen;
use crate::net::{GameState, GameStateSnapshot, NetworkAvatar};
use crate::player::Player;
use crate::team::Team;

/// Frames a state is given after it is on screen (entry motion is 220 ms).
const SETTLE_FRAMES: u32 = 40;
/// The failure frame waits for the session to give up; the whole run is
/// bounded well past `T_WAIT_MAX`.
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fixture {
    Finalizing,
    Saved,
    DefeatGuest,
    Abandoned,
    LoadingConnecting,
    LoadingFailed,
    /// Saved, with a controller: Right from Play again focuses Details.
    FocusDetails,
    /// The failure with a controller: Retry appears and takes the focus.
    FocusRetry,
}

const STAGES: [(&str, Fixture); 8] = [
    ("r1-result-finalizing.png", Fixture::Finalizing),
    ("r2-result-saved.png", Fixture::Saved),
    ("r3-result-defeat-guest.png", Fixture::DefeatGuest),
    ("r4-result-abandoned.png", Fixture::Abandoned),
    ("l1-loading-connecting.png", Fixture::LoadingConnecting),
    ("l2-loading-failed.png", Fixture::LoadingFailed),
    ("f1-result-focus-details.png", Fixture::FocusDetails),
    ("f2-loading-focus-retry.png", Fixture::FocusRetry),
];

impl Fixture {
    fn screen(self) -> AppScreen {
        match self {
            Fixture::LoadingConnecting | Fixture::LoadingFailed | Fixture::FocusRetry => {
                AppScreen::Loading
            }
            _ => AppScreen::PostMatch,
        }
    }
}

pub(crate) struct ResultQaPlugin;

impl Plugin for ResultQaPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("OMOBA_RESULT_QA_OUTPUT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let dimension = |name: &str, fallback| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(fallback)
                .clamp(200, 3840)
        };
        app.insert_resource(ResultQa {
            directory,
            pixels: UVec2::new(
                dimension("OMOBA_QA_WIDTH", 1280),
                dimension("OMOBA_QA_HEIGHT", 720),
            ),
            started: Instant::now(),
            stage: 0,
            entered: false,
            settled: 0,
            in_flight: false,
            done: Vec::new(),
        })
        .insert_resource(bevy::winit::WinitSettings::continuous())
        .add_systems(Startup, fixture_label)
        .add_systems(
            Update,
            drive
                .after(crate::net::ClientNetPipeline::ApplySnapshot)
                .before(crate::frontend::FrontendSet),
        )
        .add_systems(
            Update,
            drive_focus
                .before(crate::ui::UiSet::Focus)
                .in_set(crate::input_context::InputContextSet::Modal),
        )
        .add_systems(PostUpdate, shoot.after(bevy::ui::UiSystems::Layout));
    }
}

#[derive(Resource)]
struct ResultQa {
    directory: PathBuf,
    pixels: UVec2,
    started: Instant,
    stage: usize,
    /// The stage's screen and fixture are in place.
    entered: bool,
    settled: u32,
    in_flight: bool,
    done: Vec<usize>,
}

#[derive(Component)]
struct FixtureLabel;

fn fixture_label(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(4.0),
            right: Val::Px(8.0),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        Text::new("QA FIXTURE: result states, synthetic data"),
        TextFont {
            font_size: 11.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.8, 0.3)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        FocusPolicy::Pass,
        GlobalZIndex(3000),
        FixtureLabel,
    ));
}

const YOUR_ID: u64 = 11;

fn live_row(team: shared::map::Team) -> LiveScoreboard {
    LiveScoreboard {
        players: vec![LiveScorePlayer {
            player_id: YOUR_ID,
            nickname: "Guest".into(), // i18n-allow: fixture nickname
            team,
            hero_class: shared::HeroClass::Mage,
            kills: 7,
            deaths: 2,
            assists: 11,
            earned_gold: 12480,
            level: 12,
            connected: true,
        }],
    }
}

fn receipt(outcome: MatchOutcome, rated: bool) -> MatchResult {
    serde_json::from_value(serde_json::json!({
        "result_id": format!("qa-{outcome:?}"), "server_epoch": 7, "match_id": 2,
        "started_at_ms": 0, "ended_at_ms": 720_000, "duration_ms": 720_000,
        "map_profile": "verdant_default", "ruleset": "public-casual-v1",
        "outcome": outcome, "winner": "green", "rated": rated,
        "unrated_reason": if rated { None } else { Some("server_interrupted") },
        "participants": [{
            "player_id": YOUR_ID, "profile_id": "qa-profile", "nickname": "Guest",
            "team": "green", "hero_class": "mage", "character": "ipfs",
            "avatar": "agnes", "sprite_character": null,
            "stats": {"kills": 7, "deaths": 2, "assists": 11,
                "damage_to_heroes": 38762.0, "damage_to_structures": 0.0,
                "damage_to_creeps": 0.0, "damage_taken": 0.0, "minion_last_hits": 0,
                "jungle_last_hits": 0, "structures_destroyed": 0, "final_level": 12},
            "disconnected": false,
            "rating": if rated { Some(serde_json::json!({"before": 1200, "after": 1216, "delta": 16})) } else { None },
            "progression_xp_gained": 150
        }],
        "saved": true
    }))
    .expect("fixture receipt")
}

/// Writes the stage's fixture into the client's resources.
fn apply_fixture(
    fixture: Fixture,
    game: &mut GameStateSnapshot,
    career: &mut CareerClient,
    team: &mut Team,
) {
    game.meta = shared::protocol::SnapshotMeta::new(7, 2, 20);
    game.your_id = YOUR_ID;
    game.state = GameState::Victory {
        winner: shared::map::Team::Green,
    };
    game.rematch_in_secs = None;
    game.scoreboard = Some(live_row(shared::map::Team::Green));
    *team = Team::Green;
    career.view.storage_enabled = true;
    career.view.profile = Some(ProfileSummary {
        progression_xp: 2450,
        ..ProfileSummary::new("qa-profile".into(), "Guest".into()) // i18n-allow: fixture
    });
    career.public_profile_id = Some("qa-profile".into());
    career.view.last_result = None;
    match fixture {
        Fixture::Finalizing
        | Fixture::LoadingConnecting
        | Fixture::LoadingFailed
        | Fixture::FocusRetry => {}
        Fixture::Saved | Fixture::FocusDetails => {
            career.view.last_result = Some(receipt(MatchOutcome::Completed, true));
        }
        Fixture::Abandoned => {
            career.view.last_result = Some(receipt(MatchOutcome::Abandoned, false));
        }
        Fixture::DefeatGuest => {
            *team = Team::Blue;
            game.scoreboard = Some(live_row(shared::map::Team::Blue));
            game.rematch_in_secs = Some(7);
            career.view.profile = None;
            career.public_profile_id = None;
        }
    }
    if fixture.screen() == AppScreen::Loading {
        // No roster, no round: the connecting body.
        game.state = GameState::Lobby;
        game.prematch = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut qa: ResMut<ResultQa>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut game: ResMut<GameStateSnapshot>,
    mut career: ResMut<CareerClient>,
    session: Res<crate::net::ClientSession>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut players: Query<&mut Team, With<Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    if qa.started.elapsed() > TIMEOUT {
        error!("RESULT_QA failed: timeout at stage {}", qa.stage);
        exit.write(AppExit::error());
        return;
    }
    if let Ok(mut window) = windows.single_mut()
        && (window.resolution.physical_width() != qa.pixels.x
            || window.resolution.physical_height() != qa.pixels.y)
    {
        window.resolution.set_scale_factor_override(Some(1.0));
        window
            .resolution
            .set_physical_resolution(qa.pixels.x, qa.pixels.y);
    }
    let Some(&(_, fixture)) = STAGES.get(qa.stage) else {
        return;
    };
    if players.is_empty() {
        commands.spawn((
            Player,
            Team::Green,
            NetworkAvatar(Some("agnes".into())),
            Name::new("ResultQaPlayer"),
        ));
        return;
    }
    let wanted = fixture.screen();
    if !qa.entered {
        if *screen.get() == wanted {
            // Leave first, so the next state enters (and latches) afresh.
            next.set(AppScreen::InMatch);
            return;
        }
        let Ok(mut team) = players.single_mut() else {
            return;
        };
        apply_fixture(fixture, &mut game, &mut career, &mut team);
        next.set(wanted);
        qa.entered = true;
        qa.settled = 0;
        return;
    }
    // Keep the fixture in place (nothing else writes it without a server).
    if matches!(fixture, Fixture::LoadingFailed | Fixture::FocusRetry)
        && session.state() != crate::net::ClientConnectionState::Disconnected
    {
        qa.settled = 0;
    }
}

fn shoot(
    mut commands: Commands,
    mut qa: ResMut<ResultQa>,
    screen: Res<State<AppScreen>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(&(file, fixture)) = STAGES.get(qa.stage) else {
        return;
    };
    if qa.in_flight {
        if qa.done.contains(&qa.stage)
            && qa
                .directory
                .join(file)
                .metadata()
                .is_ok_and(|meta| meta.len() > 32)
        {
            info!("RESULT_QA captured {file}");
            qa.in_flight = false;
            qa.entered = false;
            qa.stage += 1;
            if qa.stage == STAGES.len() {
                info!("RESULT_QA completed");
                // Close the window first, as the real Exit button does: the
                // runner can hang on a live surface otherwise.
                if let Ok((window, _)) = windows.single() {
                    commands.entity(window).despawn();
                }
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    let on_screen = fixture.screen();
    let sized = windows.single().is_ok_and(|(_, window)| {
        window.resolution.physical_width() == qa.pixels.x
            && window.resolution.physical_height() == qa.pixels.y
    });
    if !qa.entered || *screen.get() != on_screen || !sized {
        qa.settled = 0;
        return;
    }
    qa.settled += 1;
    if qa.settled < SETTLE_FRAMES {
        return;
    }
    if std::fs::create_dir_all(&qa.directory).is_err() {
        error!("RESULT_QA failed: cannot create the output directory");
        exit.write(AppExit::error());
        return;
    }
    let stage = qa.stage;
    commands
        .spawn((Screenshot::primary_window(), Shot(stage)))
        .observe(save_to_disk(qa.directory.join(file)))
        .observe(readback);
    qa.in_flight = true;
}

/// Drives the kit focus as a connected controller does on the focus stages
/// (before `UiSet::Focus`, after the gamepad's own sampling).
fn drive_focus(
    qa: Res<ResultQa>,
    mut focus: ResMut<crate::ui::UiFocus>,
    mut nav: MessageWriter<crate::ui::FocusNav>,
    mut moved: Local<Option<usize>>,
) {
    let Some(&(_, fixture)) = STAGES.get(qa.stage) else {
        return;
    };
    if !matches!(fixture, Fixture::FocusDetails | Fixture::FocusRetry) {
        return;
    }
    focus.set_enabled(true);
    if fixture == Fixture::FocusDetails
        && qa.settled == SETTLE_FRAMES / 2
        && *moved != Some(qa.stage)
    {
        *moved = Some(qa.stage);
        nav.write(crate::ui::FocusNav::Right);
    }
}

#[derive(Component)]
struct Shot(usize);

fn readback(captured: On<ScreenshotCaptured>, shots: Query<&Shot>, mut qa: ResMut<ResultQa>) {
    if let Ok(shot) = shots.get(captured.entity) {
        qa.done.push(shot.0);
    }
}
