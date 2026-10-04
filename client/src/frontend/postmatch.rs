//! Result screen: the one screen shown when a base falls (DECISIONS R2.2 —
//! it replaced the in-match round-over card and the old post-match panel).
//! The layout is `omoba-ui/handoff/screens/result.md`; when each region can
//! fill, how the player leaves and what the overlays do is `post-match.md`.
//!
//! What the screen knows on the Victory frame (winner, the local team, the
//! local live score row, the avatar) is latched on entry
//! ([`PostMatchLatch`]): a transport teardown resets the snapshot, the numbers
//! must stay. The career `MatchResult` fills the rest when it arrives (the
//! body is rebuilt; the buttons and the focus stay). Where no result will ever
//! come (guest, storage off, rematch server) nothing waits for one.
//!
//! Text comes from the `postmatch` dictionary plus the career, edge and state
//! keys it shares; the screen is rebuilt on a language change.
// i18n-strict

use bevy::prelude::*;
use shared::career::{MatchOutcome, MatchResult, ParticipantResult};
use shared::live_score::LiveScorePlayer;

use super::AppScreen;
use crate::career::CareerClient;
use crate::i18n::{Locale, locale_changed, tr, trf};
use crate::mobile_controls::MobileControls;
use crate::net::{
    GameState, GameStateSnapshot, LinkStatus, NetworkAvatar, NetworkCommand, SessionUiCommand,
};
use crate::team::Team;
use crate::ui::kit_assets::{Frame, Icon, KitImage};
use crate::ui::living_background::{self, LivingBands, LivingScene};
use crate::ui::theme::{self, ButtonKind, Form, TextStyle};
use crate::ui::tokens::{TextRole, border, color, motion, radius, size, space};
use crate::ui::widgets::{
    ButtonSize, KitParts, button_node, game, icon_node, spawn_button,
    status::{self, skeleton, skeleton_ring, spinner},
    surfaces,
};
use crate::ui::{Activated, Pressable, TestId, UiActionAppExt, UiSet};

pub struct PostMatchScreenPlugin;

impl Plugin for PostMatchScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_ui_action::<PostMatchAction>()
            .init_resource::<PostMatchLatch>()
            .add_systems(
                OnEnter(AppScreen::PostMatch),
                (latch_post_match, spawn_post_match).chain(),
            )
            .add_systems(OnExit(AppScreen::PostMatch), release_post_match)
            .add_systems(
                Update,
                (
                    post_match_actions.after(UiSet::Dispatch),
                    refresh_post_match,
                    (sync_post_match, animate_post_match)
                        .after(refresh_post_match)
                        .before(UiSet::Paint),
                )
                    .run_if(in_state(AppScreen::PostMatch)),
            );
    }
}

/// The rendered result (`None` before it arrives) and whether career storage
/// was on when the screen was built.
#[derive(Component)]
struct PostMatchRoot(Option<MatchResult>, bool);

/// What else decides the body's regions: a change rebuilds the body.
#[derive(Component, Clone, PartialEq, Debug)]
struct PostMatchShape {
    expects_result: bool,
    profile_xp: Option<u64>,
    has_live_row: bool,
}

/// The screen's long-lived parts (the body is rebuilt, these are not).
#[derive(Component, Clone, Copy)]
struct PostMatchParts {
    banner: Entity,
    body: Entity,
    status: Entity,
    play_again: Entity,
    play_again_spinner: Entity,
    details: Entity,
    timer: Option<Entity>,
    timer_text: Option<Entity>,
}

/// The status line and the key of what it shows now.
#[derive(Component, Default)]
struct StatusLine(Option<StatusSpec>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostMatchAction {
    PlayAgain,
    Details,
    BackToMenu,
}

/// What the screen latched on entry: a teardown resets the snapshot, these
/// stay (post-match.md, Entry and timing).
#[derive(Resource, Clone, Default, Debug)]
pub(crate) struct PostMatchLatch {
    winner: Option<Team>,
    namespace: Option<(u64, u64)>,
    local_team: Option<Team>,
    live: Option<LiveScorePlayer>,
    roster: Vec<LiveScorePlayer>,
    avatar: Option<String>,
    /// The server sent `rematch_in_secs`: a rematch server (no career flow).
    rematch_seen: bool,
    /// `Time::elapsed_secs` on entry (input lock and entry motion).
    entered_at: f32,
    /// Play again was pressed; it stays disabled with a spinner until the
    /// screen leaves.
    play_again_pressed: bool,
}

/// Title of the banner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Victory,
    Defeat,
    Complete,
}

impl Outcome {
    fn of(winner: Option<Team>, local_team: Option<Team>) -> Self {
        match (winner, local_team) {
            (Some(winner), Some(local)) if winner == local => Outcome::Victory,
            (Some(_), Some(_)) => Outcome::Defeat,
            _ => Outcome::Complete,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Outcome::Victory => "postmatch.outcome.victory",
            Outcome::Defeat => "postmatch.outcome.defeat",
            Outcome::Complete => "postmatch.outcome.complete",
        }
    }
}

/// Headline for the result, from the player's point of view.
pub fn outcome_headline(winner: Option<Team>, local_team: Option<Team>) -> &'static str {
    tr(Outcome::of(winner, local_team).key())
}

/// A cell that may still be waiting for the career result.
#[derive(Clone, Debug, PartialEq)]
enum Cell<T> {
    Hidden,
    Pending,
    Value(T),
}

/// The player's numbers (stats panel / strip).
#[derive(Clone, Debug, PartialEq)]
struct StatsModel {
    art: Option<String>,
    class: shared::HeroClass,
    level: u32,
    kda: (u32, u32, u32),
    damage: Cell<String>,
    gold: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
struct TeamPlayerStats {
    player_id: u64,
    nickname: String,
    team: shared::map::Team,
    local: bool,
    kda: (u32, u32, u32),
    gold: Option<u32>,
}

fn team_stats(latch: &PostMatchLatch, result: Option<&MatchResult>) -> Vec<TeamPlayerStats> {
    let local_id = latch.live.as_ref().map(|row| row.player_id);
    if let Some(result) = result.filter(|result| !result.participants.is_empty()) {
        return result
            .participants
            .iter()
            .map(|p| TeamPlayerStats {
                player_id: p.player_id,
                nickname: p.nickname.clone(),
                team: p.team,
                local: local_id == Some(p.player_id),
                kda: (p.stats.kills, p.stats.deaths, p.stats.assists),
                gold: p.stats.earned_gold.or_else(|| {
                    latch
                        .roster
                        .iter()
                        .find(|live| live.player_id == p.player_id)
                        .map(|live| live.earned_gold)
                }),
            })
            .collect();
    }
    latch
        .roster
        .iter()
        .map(|p| TeamPlayerStats {
            player_id: p.player_id,
            nickname: p.nickname.clone(),
            team: p.team,
            local: local_id == Some(p.player_id),
            kda: (p.kills, p.deaths, p.assists),
            gold: Some(p.earned_gold),
        })
        .collect()
}

fn team_totals(
    rows: &[TeamPlayerStats],
    team: shared::map::Team,
) -> ((u32, u32, u32), Option<u32>) {
    let mut kda = (0_u32, 0_u32, 0_u32);
    let mut gold = Some(0_u32);
    for row in rows.iter().filter(|row| row.team == team) {
        kda.0 = kda.0.saturating_add(row.kda.0);
        kda.1 = kda.1.saturating_add(row.kda.1);
        kda.2 = kda.2.saturating_add(row.kda.2);
        gold = gold
            .zip(row.gold)
            .map(|(total, amount)| total.saturating_add(amount));
    }
    (kda, gold)
}

/// The career progress strip once the result is in.
#[derive(Clone, Debug, PartialEq)]
struct ProgressModel {
    level: u64,
    line: String,
    /// XP inside the current level (`progression_xp % 1000`).
    in_level: u32,
    /// XP this match added inside the current level (the gained segment).
    gained_in_level: u32,
    level_up: bool,
}

/// Everything the body shows, decided from the latch, the result and the
/// career view (pure: see the tests).
#[derive(Clone, Debug, PartialEq)]
struct ResultModel {
    outcome: Outcome,
    /// The banner title (`outcome_headline`, or "Match complete" for an
    /// abandoned or interrupted match).
    title: &'static str,
    summary: String,
    stats: Option<StatsModel>,
    teams: Vec<TeamPlayerStats>,
    progress: Cell<ProgressModel>,
    details: Cell<()>,
    expects_result: bool,
}

/// XP per career level (`ProfileSummary::level`).
const LEVEL_XP: u64 = 1000;

/// A career result will come for this match: a career profile, storage on
/// and not a rematch server (which never runs the career flow).
fn expects_result(latch: &PostMatchLatch, career: &CareerClient) -> bool {
    career.view.profile.is_some() && career.view.storage_enabled && !latch.rematch_seen
}

fn result_model(
    latch: &PostMatchLatch,
    result: Option<&MatchResult>,
    career: &CareerClient,
) -> ResultModel {
    let expects = expects_result(latch, career);
    let participant = result.and_then(|result| crate::career::local_participant(result, career));
    let local_team = latch
        .local_team
        .or_else(|| participant.map(|p| p.team.into()));
    let (winner, summary) = match result {
        Some(result) => match result.outcome {
            MatchOutcome::Completed => (
                result.winner.map(Into::into),
                trf(
                    match result.winner {
                        Some(shared::map::Team::Green) => "postmatch.summary.green",
                        Some(shared::map::Team::Blue) => "postmatch.summary.blue",
                        None => "postmatch.summary.nobody",
                    },
                    &[("minutes", &(result.duration_ms / 60_000))],
                ),
            ),
            MatchOutcome::Abandoned => (None, tr("postmatch.summary.abandoned").to_owned()),
            MatchOutcome::Interrupted => (None, tr("postmatch.summary.interrupted").to_owned()),
        },
        None => (
            latch.winner,
            match latch.winner {
                Some(Team::Green) => tr("postmatch.summary.green_short").to_owned(),
                Some(Team::Blue) => tr("postmatch.summary.blue_short").to_owned(),
                None => String::new(),
            },
        ),
    };
    let stats = stats_model(latch, result, participant, expects);
    let progress = match (result, career.view.profile.as_ref()) {
        (Some(_), Some(profile)) => Cell::Value(progress_model(profile, participant)),
        (None, Some(_)) if expects => Cell::Pending,
        _ => Cell::Hidden,
    };
    let details = match result {
        Some(_) if career.view.profile.is_some() => Cell::Value(()),
        None if expects => Cell::Pending,
        _ => Cell::Hidden,
    };
    ResultModel {
        outcome: Outcome::of(winner, local_team),
        title: outcome_headline(winner, local_team),
        summary,
        stats,
        teams: team_stats(latch, result),
        progress,
        details,
        expects_result: expects,
    }
}

fn stats_model(
    latch: &PostMatchLatch,
    result: Option<&MatchResult>,
    participant: Option<&ParticipantResult>,
    expects: bool,
) -> Option<StatsModel> {
    let live = latch.live.as_ref();
    let (class, level, kda) = match (participant, live) {
        (Some(p), _) => (
            p.hero_class,
            p.stats.final_level,
            (p.stats.kills, p.stats.deaths, p.stats.assists),
        ),
        (None, Some(live)) => (
            live.hero_class,
            live.level,
            (live.kills, live.deaths, live.assists),
        ),
        (None, None) => return None,
    };
    let avatar = participant
        .and_then(|p| p.avatar.clone())
        .or_else(|| latch.avatar.clone());
    let art = avatar.as_deref().and_then(|slug| {
        omoba_passport::avatars::avatar_definition(slug)
            .and_then(crate::passport::thumbnail_asset_path)
    });
    let damage = match (result, participant) {
        (Some(_), Some(p)) => Cell::Value(crate::career::number(p.stats.damage_to_heroes)),
        (None, _) if expects => Cell::Pending,
        _ => Cell::Hidden,
    };
    Some(StatsModel {
        art,
        class,
        level,
        kda,
        damage,
        gold: participant
            .and_then(|p| p.stats.earned_gold)
            .or_else(|| live.map(|live| live.earned_gold)),
    })
}

fn progress_model(
    profile: &shared::career::ProfileSummary,
    participant: Option<&ParticipantResult>,
) -> ProgressModel {
    let gained = participant.map_or(0, |p| p.progression_xp_gained);
    let in_level = (profile.progression_xp % LEVEL_XP) as u32;
    let rating = participant.and_then(|p| p.rating.as_ref()).map_or_else(
        || tr("career.rating.unrated").to_owned(),
        |change| format!("{} ({:+})", change.after, change.delta),
    );
    ProgressModel {
        level: profile.level(),
        line: trf(
            "career.result.my_progress",
            &[("rating", &rating), ("xp", &gained)],
        ),
        in_level,
        gained_in_level: gained.min(in_level),
        level_up: gained > in_level,
    }
}

pub(super) fn current_result<'a>(
    career: &'a CareerClient,
    game: &GameStateSnapshot,
) -> Option<&'a MatchResult> {
    career.view.last_result.as_ref().filter(|result| {
        result.server_epoch == game.meta.server_epoch && result.match_id == game.meta.match_id
    })
}

/// Navigation must not depend on persistence or a worker surviving retirement.
/// Saving is server-owned; leaving the result screen cannot discard that job.
fn play_again_enabled(pressed: bool) -> bool {
    !pressed
}

impl PostMatchLatch {
    pub(super) fn contains_round(&self, game: &GameStateSnapshot) -> bool {
        self.namespace == Some((game.meta.server_epoch, game.meta.match_id))
    }
}

// --- Status line ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusIcon {
    None,
    Saved,
    Saving,
    Reconnecting,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
struct StatusSpec {
    icon: StatusIcon,
    text: String,
    ink: Color,
    /// Phone: the next-round countdown after a separator.
    timer: Option<String>,
}

fn status_spec(
    expects: bool,
    result: Option<&MatchResult>,
    link: LinkStatus,
    next_round: Option<String>,
    phone: bool,
) -> StatusSpec {
    let reason = result
        .and_then(|result| result.unrated_reason.as_deref())
        .map(crate::career::unrated_reason);
    let with_reason = |text: &str| match reason {
        Some(reason) => format!("{text} · {reason}"), // i18n-allow: joins two dictionary lines
        None => text.to_owned(),
    };
    let muted = |icon, text| StatusSpec {
        icon,
        text,
        ink: color::TEXT_MUTED,
        timer: None,
    };
    if result.is_some_and(|result| result.saved) {
        return muted(StatusIcon::Saved, with_reason(tr("postmatch.saved")));
    }
    if expects || result.is_some() {
        return match link {
            LinkStatus::Reconnecting { .. } => StatusSpec {
                icon: StatusIcon::Reconnecting,
                text: link.detail().unwrap_or_default(),
                ink: color::STATE_WARNING,
                timer: None,
            },
            LinkStatus::Compatibility(_)
            | LinkStatus::Disconnected
            | LinkStatus::Rejected(_)
            | LinkStatus::Unconfirmed => StatusSpec {
                icon: StatusIcon::Failed,
                text: link.detail().unwrap_or_default(),
                ink: color::TEXT_DANGER,
                timer: None,
            },
            _ => muted(StatusIcon::Saving, with_reason(tr("postmatch.saving"))),
        };
    }
    StatusSpec {
        timer: next_round.filter(|_| phone),
        ..muted(StatusIcon::None, tr("postmatch.local_result").to_owned())
    }
}

// --- Systems ---

/// Latches what the Victory frame knows and holds the career page back.
fn latch_post_match(
    mut latch: ResMut<PostMatchLatch>,
    game: Res<GameStateSnapshot>,
    time: Option<Res<Time>>,
    mut career: Option<ResMut<CareerClient>>,
    local: Query<(&Team, Option<&NetworkAvatar>), With<crate::player::Player>>,
) {
    let local = local.iter().next();
    *latch = PostMatchLatch {
        namespace: Some((game.meta.server_epoch, game.meta.match_id)),
        winner: match game.state {
            GameState::Victory { winner } => Some(winner.into()),
            _ => None,
        },
        local_team: local.map(|(team, _)| *team),
        live: game.scoreboard.as_ref().and_then(|board| {
            board
                .players
                .iter()
                .find(|player| player.player_id == game.your_id)
                .cloned()
        }),
        roster: game
            .scoreboard
            .as_ref()
            .map_or_else(Vec::new, |board| board.players.clone()),
        avatar: local.and_then(|(_, avatar)| avatar.and_then(|avatar| avatar.0.clone())),
        rematch_seen: game.rematch_in_secs.is_some(),
        entered_at: time.map_or(0.0, |time| time.elapsed_secs()),
        play_again_pressed: false,
    };
    if let Some(career) = career.as_mut() {
        career.hold_result_modal = true;
    }
}

fn release_post_match(mut career: Option<ResMut<CareerClient>>) {
    if let Some(career) = career.as_mut() {
        career.hold_result_modal = false;
    }
}

/// What the screen reads to draw itself.
#[derive(bevy::ecs::system::SystemParam)]
struct ScreenInputs<'w> {
    game: Res<'w, GameStateSnapshot>,
    career: Res<'w, CareerClient>,
    latch: Option<Res<'w, PostMatchLatch>>,
    mobile: Option<Res<'w, MobileControls>>,
}

impl ScreenInputs<'_> {
    fn latch(&self) -> PostMatchLatch {
        self.latch.as_deref().cloned().unwrap_or_default()
    }

    fn form(&self) -> Form {
        Form::from_mobile(self.mobile.as_deref())
    }

    fn shape(&self) -> PostMatchShape {
        let latch = self.latch();
        PostMatchShape {
            expects_result: expects_result(&latch, &self.career),
            profile_xp: self
                .career
                .view
                .profile
                .as_ref()
                .map(|profile| profile.progression_xp),
            has_live_row: latch.live.is_some(),
        }
    }
}

fn spawn_post_match(mut commands: Commands, inputs: ScreenInputs) {
    build_screen(&mut commands, &inputs);
}

fn build_screen(commands: &mut Commands, inputs: &ScreenInputs) {
    let latch = inputs.latch();
    let result = current_result(&inputs.career, &inputs.game).cloned();
    let model = result_model(&latch, result.as_ref(), &inputs.career);
    let form = inputs.form();
    let safe = inputs.mobile.as_deref().map(|mobile| mobile.safe);
    let background = match model.outcome {
        Outcome::Defeat => LivingScene::Defeat,
        Outcome::Victory | Outcome::Complete => LivingScene::Victory,
    };
    let mut parts = None;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                ..default()
            },
            ZIndex(theme::SCREEN_Z),
            bevy::state::state_scoped::DespawnOnExit(AppScreen::PostMatch),
            Name::new("PostMatchScreen"),
            PostMatchRoot(result.clone(), inputs.career.view.storage_enabled),
            inputs.shape(),
        ))
        .with_children(|root| {
            living_background::spawn(
                root,
                background,
                LivingBands {
                    header: None,
                    footer: (form == Form::Desktop).then_some(64.0),
                },
                form,
            );
            parts = Some(match form {
                Form::Desktop => {
                    root.spawn(surfaces::ornament_frame());
                    desktop_layout(root, &model)
                }
                Form::Phone => phone_layout(root, &model, safe),
            });
        })
        .insert(parts.expect("layout spawned"));
}

/// Desktop (result.md): a 1280×720 stage centred in the window (the screen
/// scales with R2.3 around it), one column from y 48.
fn desktop_layout(root: &mut ChildSpawnerCommands, model: &ResultModel) -> PostMatchParts {
    let form = Form::Desktop;
    let mut parts = PostMatchParts {
        banner: Entity::PLACEHOLDER,
        body: Entity::PLACEHOLDER,
        status: Entity::PLACEHOLDER,
        play_again: Entity::PLACEHOLDER,
        play_again_spinner: Entity::PLACEHOLDER,
        details: Entity::PLACEHOLDER,
        timer: None,
        timer_text: None,
    };
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(REFERENCE.x),
            height: Val::Px(REFERENCE.y),
            margin: UiRect {
                left: Val::Px(-REFERENCE.x / 2.0),
                top: Val::Px(-REFERENCE.y / 2.0),
                ..default()
            },
            ..default()
        },
        Name::new("PostMatchStage"),
    ))
    .with_children(|stage| {
        stage
            .spawn(Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(48.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|column| {
                parts.body = column
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        Name::new("PostMatchBody"),
                    ))
                    .with_children(|body| parts.banner = spawn_body(body, model, form))
                    .id();
                parts.status = spawn_status_line(column, form, 760.0, 12.0);
                column
                    .spawn((
                        Node {
                            height: Val::Px(size::BUTTON_LG_HEIGHT.desktop),
                            margin: UiRect::top(Val::Px(20.0)),
                            column_gap: Val::Px(space::S16),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        Name::new("PostMatchActions"),
                    ))
                    .with_children(|row| {
                        (parts.play_again, parts.play_again_spinner) =
                            spawn_play_again(row, form, 260.0);
                        parts.details = spawn_details(row, form, 200.0, model);
                    });
                column
                    .spawn(Node {
                        margin: UiRect::top(Val::Px(space::S12)),
                        ..default()
                    })
                    .with_children(|row| spawn_back(row, form, 200.0));
                let (timer, text) = spawn_round_timer(column);
                parts.timer = Some(timer);
                parts.timer_text = Some(text);
            });
        stage.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(40.0),
                top: Val::Px(668.0),
                width: Val::Px(360.0),
                height: Val::Px(20.0),
                ..default()
            },
            Text::new(tr("state.exit_hint")),
            theme::role_text(TextRole::Caption),
            TextColor(color::TEXT_MUTED),
            Name::new("PostMatchExitHint"),
        ));
    });
    parts
}

/// The reference the desktop layout is drawn on (result.md).
const REFERENCE: Vec2 = Vec2::new(1280.0, 720.0);
/// The R2.3 floor this screen may go down to (it uses text roles only).
const RESULT_SCALE_MIN: f32 = 0.8;

/// Desktop `UiScale` on the result screen: `clamp(min(w/1280, h/720), 0.8,
/// 2.0)` (DECISIONS R2.3; the 1.0 floor of R2.3a is for screens that still
/// have legacy text).
pub(crate) fn result_ui_scale(width: f32, height: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 1.0;
    }
    (width / REFERENCE.x)
        .min(height / REFERENCE.y)
        .clamp(RESULT_SCALE_MIN, theme::metric::DESKTOP_SCALE_MAX)
}

/// Phone (result.md phone): banner, summary, strips and status from the top
/// inside the safe area; the action row on the safe bottom.
fn phone_layout(
    root: &mut ChildSpawnerCommands,
    model: &ResultModel,
    safe: Option<crate::mobile_controls::MobileSafeInsets>,
) -> PostMatchParts {
    let form = Form::Phone;
    let margin = space::SCREEN_MARGIN.phone;
    let (left, right, top, bottom) = safe.map_or((32.0, 32.0, 0.0, 20.0), |safe| {
        (safe.left, safe.right, safe.top, safe.bottom)
    });
    let mut parts = PostMatchParts {
        banner: Entity::PLACEHOLDER,
        body: Entity::PLACEHOLDER,
        status: Entity::PLACEHOLDER,
        play_again: Entity::PLACEHOLDER,
        play_again_spinner: Entity::PLACEHOLDER,
        details: Entity::PLACEHOLDER,
        timer: None,
        timer_text: None,
    };
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left + margin),
            right: Val::Px(right + margin),
            top: Val::Px(top + space::S8),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        },
        Name::new("PostMatchStage"),
    ))
    .with_children(|column| {
        parts.body = column
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                Name::new("PostMatchBody"),
            ))
            .with_children(|body| parts.banner = spawn_body(body, model, form))
            .id();
        parts.status = spawn_status_line(column, form, 0.0, 6.0);
    });
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left + margin),
            right: Val::Px(right + margin),
            bottom: Val::Px(bottom + space::S8),
            height: Val::Px(size::BUTTON_LG_HEIGHT.phone),
            align_items: AlignItems::Center,
            column_gap: Val::Px(space::S16),
            ..default()
        },
        Name::new("PostMatchActions"),
    ))
    .with_children(|row| {
        spawn_back(row, form, 150.0);
        row.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        parts.details = spawn_details(row, form, 160.0, model);
        (parts.play_again, parts.play_again_spinner) = spawn_play_again(row, form, 200.0);
    });
    parts
}

/// Banner, summary, stats and progress (the part rebuilt when the result
/// arrives). Returns the banner.
fn spawn_body(body: &mut ChildSpawnerCommands, model: &ResultModel, form: Form) -> Entity {
    let banner = spawn_banner(body, model.outcome, model.title, form);
    spawn_summary(body, &model.summary, form);
    if model.teams.is_empty() {
        if let Some(stats) = &model.stats {
            spawn_stats(body, stats, form);
        }
    } else {
        spawn_team_stats(body, &model.teams, form);
        if let Some(stats) = &model.stats
            && form == Form::Desktop
        {
            spawn_damage_line(body, &stats.damage);
        }
    }
    if !matches!(model.progress, Cell::Hidden) {
        spawn_progress(body, &model.progress, form);
    }
    banner
}

fn spawn_banner(
    body: &mut ChildSpawnerCommands,
    outcome: Outcome,
    title: &str,
    form: Form,
) -> Entity {
    let (width, height) = match form {
        Form::Desktop => (640.0, 80.0),
        Form::Phone => (420.0, 48.0),
    };
    let title_ink = if outcome == Outcome::Victory {
        color::TEXT_GOLD
    } else {
        color::TEXT_PRIMARY
    };
    body.spawn((
        Node {
            width: Val::Px(width),
            height: Val::Px(height),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            // The title sits 14 px below the panel centre under the crest.
            padding: UiRect::top(Val::Px(if form == Form::Desktop { 28.0 } else { 0.0 })),
            ..default()
        },
        KitImage::frame(Frame::Panel),
        UiTransform::IDENTITY,
        Name::new("PostMatchBanner"),
    ))
    .with_children(|banner| {
        banner.spawn((
            Text::new(title),
            theme::styled_text(if form == Form::Phone {
                // Match the phone role immediately, before the theme pass, so
                // the first measured frame cannot expand this compact banner.
                TextStyle::new(TextRole::TitleXl).sized(crate::ui::tokens::Metric::new(32.0, 32.0))
            } else {
                TextStyle::new(TextRole::TitleXl)
            }),
            TextColor(title_ink),
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            Name::new("PostMatchOutcome"),
        ));
        if form == Form::Desktop {
            let crest = if outcome == Outcome::Defeat {
                color::TEXT_MUTED
            } else {
                color::TEXT_GOLD
            };
            banner.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(-(size::ICON_XL / 2.0) + border::FRAME),
                    left: Val::Percent(50.0),
                    margin: UiRect::left(Val::Px(-size::ICON_XL / 2.0)),
                    ..default()
                },
                children![icon_node(Icon::NavCrown, size::ICON_XL, crest)],
            ));
        }
        if outcome == Outcome::Defeat {
            banner.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(space::S24),
                    right: Val::Px(space::S24),
                    bottom: Val::Px(-(space::S4 + border::FRAME)),
                    height: Val::Px(border::FRAME),
                    ..default()
                },
                BackgroundColor(color::STATE_DANGER),
            ));
        }
    })
    .id()
}

fn spawn_summary(body: &mut ChildSpawnerCommands, summary: &str, form: Form) {
    if summary.is_empty() {
        return;
    }
    let (width, height, gap, role) = match form {
        Form::Desktop => (Val::Px(800.0), 28.0, space::S16, TextRole::Heading),
        Form::Phone => (Val::Px(600.0), 20.0, 6.0, TextRole::Label),
    };
    body.spawn((
        Node {
            width,
            max_width: Val::Percent(100.0),
            height: Val::Px(height),
            margin: UiRect::top(Val::Px(gap)),
            padding: UiRect::axes(
                Val::Px(space::S12),
                Val::Px(if form == Form::Desktop {
                    space::S4
                } else {
                    0.0
                }),
            ),
            column_gap: Val::Px(space::S16),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        Name::new("PostMatchSummary"),
    ))
    .with_children(|row| {
        let rule = || {
            (
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(border::HAIRLINE),
                    ..default()
                },
                BackgroundColor(theme::perceptual(color::BORDER_HAIRLINE)),
            )
        };
        if form == Form::Desktop {
            row.spawn(rule());
        }
        row.spawn((
            Text::new(summary),
            theme::role_text(role),
            TextColor(color::TEXT_PRIMARY),
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            Name::new("PostMatchSummaryText"),
        ));
        if form == Form::Desktop {
            row.spawn(rule());
        }
    });
}

fn spawn_team_stats(parent: &mut ChildSpawnerCommands, rows: &[TeamPlayerStats], form: Form) {
    let phone = form == Form::Phone;
    parent
        .spawn((
            Node {
                width: if phone {
                    Val::Percent(100.0)
                } else {
                    Val::Px(880.0)
                },
                max_width: Val::Percent(100.0),
                column_gap: Val::Px(12.0),
                margin: UiRect::top(Val::Px(6.0)),
                flex_shrink: 0.0,
                ..default()
            },
            Name::new("PostMatchTeams"),
        ))
        .with_children(|teams| {
            for (team, key, ink, identity) in [
                (
                    shared::map::Team::Green,
                    "edge.team.green",
                    color::TEAM_GREEN,
                    "Green", // i18n-allow: canonical node identity suffix
                ),
                (
                    shared::map::Team::Blue,
                    "edge.team.blue",
                    color::TEAM_BLUE,
                    "Blue", // i18n-allow: canonical node identity suffix
                ),
            ] {
                teams
                    .spawn((
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            min_width: Val::Px(0.0),
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(4.0)),
                            border_radius: BorderRadius::all(Val::Px(radius::MD)),
                            ..default()
                        },
                        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
                        Name::new(format!("PostMatchTeam{identity}")),
                    ))
                    .with_children(|panel| {
                        team_row(
                            panel,
                            tr(key),
                            tr("edge.kda"),
                            tr("postmatch.gold_earned"),
                            phone,
                            ink,
                            &format!("PostMatchHeader{identity}"),
                            false,
                        );
                        for row in rows.iter().filter(|row| row.team == team) {
                            team_row(
                                panel,
                                &row.nickname,
                                &format!("{} / {} / {}", row.kda.0, row.kda.1, row.kda.2),
                                &row.gold
                                    .map_or_else(|| "—".to_owned(), |gold| gold.to_string()),
                                phone,
                                if row.local {
                                    color::TEXT_GOLD
                                } else {
                                    color::TEXT_PRIMARY
                                },
                                &format!("PostMatchPlayer{}", row.player_id),
                                row.local,
                            );
                        }
                        let (kda, gold) = team_totals(rows, team);
                        team_row(
                            panel,
                            tr("postmatch.total"),
                            &format!("{} / {} / {}", kda.0, kda.1, kda.2),
                            &gold.map_or_else(|| "—".to_owned(), |gold| gold.to_string()),
                            phone,
                            ink,
                            &format!("PostMatchTotal{identity}"),
                            false,
                        );
                    });
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn team_row(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    kda: &str,
    gold: &str,
    phone: bool,
    ink: Color,
    identity: &str,
    local: bool,
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(if phone { 18.0 } else { 24.0 }),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            },
            Name::new(identity.to_owned()),
        ))
        .with_children(|row| {
            for (value, width, child_name) in [
                (name, Val::Auto, format!("{identity}Name")), // i18n-allow: canonical node identity
                (
                    kda,
                    Val::Px(if phone { 72.0 } else { 90.0 }),
                    if local {
                        "PostMatchKdaValue".to_owned()
                    } else {
                        format!("{identity}Kda") // i18n-allow: canonical node identity
                    },
                ),
                (
                    gold,
                    Val::Px(if phone { 76.0 } else { 104.0 }),
                    if local {
                        "PostMatchGoldValue".to_owned()
                    } else {
                        format!("{identity}Gold") // i18n-allow: canonical node identity
                    },
                ),
            ] {
                row.spawn((
                    Text::new(value),
                    TextFont {
                        font_size: if phone { 11.0 } else { 14.0 },
                        ..default()
                    },
                    TextColor(ink),
                    TextLayout::new(
                        if width == Val::Auto {
                            Justify::Left
                        } else {
                            Justify::Right
                        },
                        LineBreak::NoWrap,
                    ),
                    Node {
                        width,
                        min_width: Val::Px(0.0),
                        flex_grow: if width == Val::Auto { 1.0 } else { 0.0 },
                        flex_shrink: if width == Val::Auto { 1.0 } else { 0.0 },
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Name::new(child_name),
                ));
            }
        });
}

fn spawn_damage_line(parent: &mut ChildSpawnerCommands, damage: &Cell<String>) {
    if matches!(damage, Cell::Hidden) {
        return;
    }
    parent
        .spawn((
            Node {
                height: Val::Px(20.0),
                flex_shrink: 0.0,
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            },
            Name::new("PostMatchDamage"),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(tr("career.table.hero_damage")),
                theme::role_text(TextRole::Caption),
                TextColor(color::TEXT_MUTED),
            ));
            match damage {
                Cell::Value(value) => {
                    row.spawn((
                        Text::new(value),
                        theme::role_text(TextRole::Caption),
                        TextColor(color::TEXT_PRIMARY),
                        Name::new("PostMatchDamageValue"),
                    ));
                }
                Cell::Pending => {
                    row.spawn(skeleton(Val::Px(56.0), Val::Px(12.0)));
                }
                Cell::Hidden => {}
            }
        });
}

fn spawn_stats(body: &mut ChildSpawnerCommands, stats: &StatsModel, form: Form) {
    let desktop = form == Form::Desktop;
    let mut panel = body.spawn((
        Node {
            width: if desktop {
                Val::Px(760.0)
            } else {
                Val::Percent(100.0)
            },
            max_width: Val::Percent(100.0),
            height: Val::Px(if desktop { 136.0 } else { 88.0 }),
            margin: UiRect::top(Val::Px(if desktop { space::S16 } else { 6.0 })),
            padding: if desktop {
                UiRect::new(
                    Val::Px(space::S24),
                    Val::Px(space::S16),
                    Val::Px(space::S16),
                    Val::Px(space::S16),
                )
            } else {
                UiRect::all(Val::Px(space::S12))
            },
            column_gap: Val::Px(if desktop { space::S24 } else { space::S12 }),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            border: UiRect::all(Val::Px(if desktop { 0.0 } else { border::HAIRLINE })),
            border_radius: BorderRadius::all(Val::Px(if desktop { 0.0 } else { radius::LG })),
            ..default()
        },
        Name::new("PostMatchStats"),
    ));
    if desktop {
        panel.insert(KitImage::frame(Frame::Panel));
    } else {
        // The phone strip is glass (result.md phone), not a framed panel.
        panel.insert((
            BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
            BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
        ));
    }
    panel.with_children(|panel| {
        spawn_portrait(
            panel,
            stats,
            if desktop {
                size::PORTRAIT_LG
            } else {
                size::PORTRAIT_MD
            },
        );
        panel
            .spawn(Node {
                flex_grow: 1.0,
                height: Val::Percent(100.0),
                column_gap: Val::Px(space::S8),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|columns| {
                let (kills, deaths, assists) = stats.kda;
                let kda = format!("{kills} / {deaths} / {assists}"); // i18n-allow: numbers only
                stat_column(
                    columns,
                    form,
                    232.0,
                    Icon::HudKill,
                    tr("edge.kda"),
                    StatValue::Text(kda, color::TEXT_PRIMARY),
                    "PostMatchKda",
                );
                match &stats.damage {
                    Cell::Hidden => {}
                    Cell::Pending => stat_column(
                        columns,
                        form,
                        188.0,
                        Icon::HudAttack,
                        tr("career.table.hero_damage"),
                        StatValue::Pending,
                        "PostMatchDamage",
                    ),
                    Cell::Value(damage) => stat_column(
                        columns,
                        form,
                        188.0,
                        Icon::HudAttack,
                        tr("career.table.hero_damage"),
                        StatValue::Text(damage.clone(), color::TEXT_PRIMARY),
                        "PostMatchDamage",
                    ),
                }
                if let Some(gold) = stats.gold {
                    stat_column(
                        columns,
                        form,
                        180.0,
                        Icon::HudGold,
                        tr("edge.column.gold"),
                        StatValue::Text(gold.to_string(), color::TEXT_GOLD),
                        "PostMatchGold",
                    );
                }
            });
    });
}

/// The portrait: roster art in a 2 px `color.gold.500` rim (the class icon on
/// `color.surface.3` without art, never initials) and the level disc.
fn spawn_portrait(parent: &mut ChildSpawnerCommands, stats: &StatsModel, side: f32) {
    parent
        .spawn((
            Node {
                width: Val::Px(side),
                height: Val::Px(side),
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(border::FRAME)),
                border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(color::GOLD_500),
            BackgroundColor(color::SURFACE_3),
            Name::new("PostMatchPortrait"),
        ))
        .with_children(|portrait| {
            match &stats.art {
                Some(path) => {
                    portrait.spawn(game::round_art(path.clone(), side - 2.0 * border::FRAME));
                }
                None => {
                    portrait.spawn(icon_node(
                        game::class_icon(stats.class),
                        size::ICON_XL,
                        color::TEXT_MUTED,
                    ));
                }
            }
            portrait
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(-space::S4),
                        bottom: Val::Px(-space::S4),
                        width: Val::Px(game::LEVEL_DISC),
                        height: Val::Px(game::LEVEL_DISC),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(border::HAIRLINE)),
                        border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                        ..default()
                    },
                    BackgroundColor(color::SURFACE_1_OPAQUE),
                    BorderColor::all(color::GOLD_500),
                ))
                .with_child((
                    Text::new(stats.level.to_string()),
                    theme::role_text(TextRole::NumberSm),
                    TextColor(color::TEXT_GOLD),
                ));
        });
}

enum StatValue {
    Text(String, Color),
    Pending,
}

/// One stats column: icon (desktop), eyebrow label, big number (or a
/// skeleton while the result is on its way).
fn stat_column(
    columns: &mut ChildSpawnerCommands,
    form: Form,
    width: f32,
    icon: Icon,
    label: &str,
    value: StatValue,
    name: &'static str,
) {
    let desktop = form == Form::Desktop;
    columns
        .spawn((
            Node {
                width: if desktop { Val::Px(width) } else { Val::Auto },
                flex_grow: if desktop { 0.0 } else { 1.0 },
                flex_basis: if desktop { Val::Auto } else { Val::Px(0.0) },
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(space::S4),
                ..default()
            },
            Name::new(name),
        ))
        .with_children(|column| {
            if desktop {
                column.spawn(icon_node(icon, size::ICON_LG, color::TEXT_GOLD));
            }
            column.spawn((
                Text::new(label),
                theme::role_text(TextRole::Eyebrow),
                TextColor(color::TEXT_MUTED),
                TextLayout::new(Justify::Center, LineBreak::NoWrap),
            ));
            match value {
                StatValue::Text(text, ink) => {
                    column.spawn((
                        Text::new(text),
                        theme::role_text(if desktop {
                            TextRole::NumberXl
                        } else {
                            TextRole::NumberLg
                        }),
                        TextColor(ink),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                        Name::new(format!("{name}Value")), // i18n-allow: node identity
                    ));
                }
                StatValue::Pending => {
                    let (w, h) = if desktop { (120.0, 40.0) } else { (96.0, 26.0) };
                    column.spawn(skeleton(Val::Px(w), Val::Px(h)));
                }
            }
        });
}

/// The gained part of the XP bar grows over `motion.duration.bar_trail`
/// once the strip shows the result.
#[derive(Component, Clone, Copy)]
struct GainedSegment {
    from: f32,
    to: f32,
    started: Option<f32>,
}

/// The level badge flashes once on a level-up.
#[derive(Component, Clone, Copy)]
struct LevelFlash {
    started: Option<f32>,
}

fn spawn_progress(body: &mut ChildSpawnerCommands, progress: &Cell<ProgressModel>, form: Form) {
    let desktop = form == Form::Desktop;
    let badge = if desktop { 48.0 } else { 32.0 };
    body.spawn((
        Node {
            width: if desktop {
                Val::Px(760.0)
            } else {
                Val::Percent(100.0)
            },
            max_width: Val::Percent(100.0),
            // The phone keeps both five-player tables above its fixed56px
            // actions. A32px level disc plus padding/border needs42px.
            height: Val::Px(if desktop { 72.0 } else { 42.0 }),
            margin: UiRect::top(Val::Px(if desktop { space::S12 } else { space::S4 })),
            padding: UiRect::axes(
                Val::Px(if desktop { space::S16 } else { space::S12 }),
                Val::Px(if desktop { space::S12 } else { space::S4 }),
            ),
            column_gap: Val::Px(space::S16),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::LG)),
            ..default()
        },
        BackgroundColor(color::SURFACE_1),
        BorderColor::all(color::BORDER_SUBTLE),
        Name::new("PostMatchProgress"),
    ))
    .with_children(|strip| {
        let Cell::Value(model) = progress else {
            strip.spawn(skeleton_ring(badge));
            strip
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(space::S8),
                    ..default()
                })
                .with_children(|column| {
                    column.spawn(skeleton(Val::Px(300.0), Val::Px(14.0)));
                    column.spawn(xp_track());
                });
            return;
        };
        strip
            .spawn((
                Node {
                    width: Val::Px(badge),
                    height: Val::Px(badge),
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(border::FRAME)),
                    border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                    ..default()
                },
                BackgroundColor(color::SURFACE_2),
                BorderColor::all(color::GOLD_500),
                LevelFlash {
                    started: (!model.level_up).then_some(f32::NEG_INFINITY),
                },
                Name::new("PostMatchCareerLevel"),
            ))
            .with_child((
                Text::new(model.level.to_string()),
                theme::role_text(TextRole::NumberLg),
                TextColor(color::TEXT_GOLD),
            ));
        strip
            .spawn(Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(if desktop { space::S8 } else { space::S4 }),
                ..default()
            })
            .with_children(|column| {
                column
                    .spawn(Node {
                        column_gap: Val::Px(space::S12),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|line| {
                        line.spawn((
                            Text::new(&model.line),
                            theme::role_text(TextRole::Label),
                            TextColor(color::TEXT_ACCENT),
                            TextLayout::new(Justify::Left, LineBreak::NoWrap),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                            Name::new("PostMatchCareerLine"),
                        ));
                        line.spawn((
                            Text::new(format!("{} / {LEVEL_XP}", model.in_level)), // i18n-allow: numbers only
                            theme::role_text(TextRole::NumberSm),
                            TextColor(color::TEXT_SECONDARY),
                        ));
                    });
                let total = LEVEL_XP as f32;
                let before = (model.in_level - model.gained_in_level) as f32 / total;
                let now = model.in_level as f32 / total;
                column.spawn(xp_track()).with_children(|track| {
                    track.spawn((xp_fill(0.0, before), BackgroundColor(color::BAR_XP)));
                    track.spawn((
                        xp_fill(before, 0.0),
                        BackgroundColor(color::GOLD_300),
                        GainedSegment {
                            from: before,
                            to: now,
                            started: None,
                        },
                    ));
                });
            });
    });
}

fn xp_track() -> impl Bundle {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(size::BAR_XP),
            border: UiRect::all(Val::Px(border::HAIRLINE)),
            border_radius: BorderRadius::all(Val::Px(radius::SM)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::BAR_TRACK)),
        BorderColor::all(color::SCRIM.with_alpha(1.0)),
    )
}

fn xp_fill(left: f32, width: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(left * 100.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        width: Val::Percent(width * 100.0),
        ..default()
    }
}

fn spawn_status_line(
    column: &mut ChildSpawnerCommands,
    form: Form,
    width: f32,
    gap: f32,
) -> Entity {
    column
        .spawn((
            Node {
                width: if form == Form::Desktop {
                    Val::Px(width)
                } else {
                    Val::Percent(100.0)
                },
                height: Val::Px(20.0),
                margin: UiRect::top(Val::Px(gap)),
                column_gap: Val::Px(space::S8),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..default()
            },
            BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
            StatusLine::default(),
            Name::new("PostMatchStatus"),
        ))
        .id()
}

fn fill_status_line(line: &mut ChildSpawnerCommands, spec: &StatusSpec) {
    match spec.icon {
        StatusIcon::None => {}
        StatusIcon::Saved => {
            line.spawn(icon_node(
                Icon::NavCheck,
                size::ICON_SM,
                color::STATE_SUCCESS,
            ));
        }
        StatusIcon::Saving => {
            spinner(line, status::SPINNER_SM);
        }
        StatusIcon::Reconnecting => {
            line.spawn(icon_node(
                Icon::NavWifiOff,
                size::ICON_SM,
                color::STATE_WARNING,
            ));
        }
        StatusIcon::Failed => {
            line.spawn(icon_node(
                Icon::NavAlertTriangle,
                size::ICON_SM,
                color::TEXT_DANGER,
            ));
        }
    }
    line.spawn((
        Text::new(&spec.text),
        theme::role_text(TextRole::Caption),
        TextColor(spec.ink),
        TextLayout::new(Justify::Center, LineBreak::NoWrap),
        Name::new("PostMatchStatusText"),
    ));
    if let Some(timer) = &spec.timer {
        line.spawn((
            Text::new("·"),
            theme::role_text(TextRole::Caption),
            TextColor(color::TEXT_DISABLED),
        ));
        line.spawn(icon_node(Icon::NavTimer, size::ICON_SM, color::TEXT_MUTED));
        line.spawn((
            Text::new(timer),
            theme::role_text(TextRole::Caption),
            TextColor(color::TEXT_MUTED),
            Name::new("PostMatchNextRound"),
        ));
    }
}

/// Play again (primary large, the default focus) with a hidden spinner that
/// replaces the label once pressed.
fn spawn_play_again(row: &mut ChildSpawnerCommands, form: Form, width: f32) -> (Entity, Entity) {
    let button = spawn_button(
        row,
        Node {
            width: Val::Px(width),
            ..button_node(ButtonSize::Large, ButtonKind::Primary, form)
        },
        tr("postmatch.button.play_again"),
        TextStyle::new(TextRole::ButtonLg),
        ButtonKind::Primary,
        None,
        PostMatchAction::PlayAgain,
        TestId::new("PostMatchPlayAgain"),
        (),
    );
    let mut spinner_entity = Entity::PLACEHOLDER;
    row.commands().entity(button).with_children(|button| {
        spinner_entity = spinner(button, status::SPINNER_MD);
        button.commands().entity(spinner_entity).insert(Node {
            width: Val::Px(status::SPINNER_MD),
            height: Val::Px(status::SPINNER_MD),
            flex_shrink: 0.0,
            display: Display::None,
            ..default()
        });
    });
    (button, spinner_entity)
}

fn spawn_details(
    row: &mut ChildSpawnerCommands,
    form: Form,
    width: f32,
    model: &ResultModel,
) -> Entity {
    let button = spawn_button(
        row,
        Node {
            width: Val::Px(width),
            ..button_node(ButtonSize::Regular, ButtonKind::Secondary, form)
        },
        tr("career.button.details"),
        TextStyle::new(TextRole::Button),
        ButtonKind::Secondary,
        None,
        PostMatchAction::Details,
        TestId::new("PostMatchDetails"),
        (),
    );
    let (visibility, disabled) = details_state(&model.details);
    row.commands()
        .entity(button)
        .insert(visibility)
        .insert(Pressable {
            disabled,
            ..default()
        });
    button
}

/// Hidden (the gap stays), disabled until the result, enabled with it.
fn details_state(details: &Cell<()>) -> (Visibility, bool) {
    match details {
        Cell::Hidden => (Visibility::Hidden, true),
        Cell::Pending => (Visibility::Inherited, true),
        Cell::Value(()) => (Visibility::Inherited, false),
    }
}

fn spawn_back(row: &mut ChildSpawnerCommands, form: Form, width: f32) {
    row.spawn((
        Node {
            width: Val::Px(width),
            border_radius: BorderRadius::all(Val::Px(radius::MD)),
            ..default()
        },
        BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
        Name::new("PostMatchBackPlate"),
    ))
    .with_children(|plate| {
        spawn_button(
            plate,
            Node {
                width: Val::Percent(100.0),
                ..button_node(ButtonSize::Regular, ButtonKind::Link, form)
            },
            tr("postmatch.button.back_to_menu"),
            TextStyle::new(TextRole::Button).sized(crate::ui::widgets::TERTIARY_LABEL),
            ButtonKind::Link,
            None,
            PostMatchAction::BackToMenu,
            TestId::new("PostMatchBackToMenu"),
            (),
        );
    });
}

/// The next-round badge (desktop; rematch servers only): muted, 28 high,
/// `nav/timer` + `type.caption` semibold.
fn spawn_round_timer(column: &mut ChildSpawnerCommands) -> (Entity, Entity) {
    let mut text = Entity::PLACEHOLDER;
    let timer = column
        .spawn((
            Node {
                width: Val::Px(300.0),
                height: Val::Px(28.0),
                margin: UiRect::top(Val::Px(space::S16)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            Name::new("PostMatchRoundTimer"),
        ))
        .with_children(|slot| {
            slot.spawn((
                Node {
                    height: Val::Px(28.0),
                    padding: UiRect::horizontal(Val::Px(space::S12)),
                    column_gap: Val::Px(space::S8),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(Val::Px(radius::PILL)),
                    ..default()
                },
                BackgroundColor(color::SURFACE_3),
            ))
            .with_children(|badge| {
                badge.spawn(icon_node(
                    Icon::NavTimer,
                    size::ICON_SM,
                    color::TEXT_SECONDARY,
                ));
                text = badge
                    .spawn((
                        Text::new(""),
                        theme::styled_text(
                            TextStyle::keep_case(TextRole::Label)
                                .sized(TextRole::Caption.style().size),
                        ),
                        TextColor(color::TEXT_SECONDARY),
                    ))
                    .id();
            });
        })
        .id();
    (timer, text)
}

/// Rebuilds the screen on a language change and the body when the result,
/// storage or the career profile changes; a teardown leaves it as it is.
fn refresh_post_match(
    mut commands: Commands,
    inputs: ScreenInputs,
    roots: Query<(
        Entity,
        &PostMatchRoot,
        Option<&PostMatchShape>,
        Option<&PostMatchParts>,
    )>,
    locale: Option<Res<Locale>>,
) {
    let Ok((entity, root, shape, parts)) = roots.single() else {
        return;
    };
    let game = &inputs.game;
    // A retired worker clears transport state. Keep the already validated
    // terminal receipt and its Victory/Defeat presentation until the player leaves.
    if game.meta.server_epoch == 0
        || root.0.as_ref().is_some_and(|result| {
            result.server_epoch != game.meta.server_epoch || result.match_id != game.meta.match_id
        })
    {
        return;
    }
    let result = current_result(&inputs.career, game);
    let new_shape = inputs.shape();
    let body_changed = root.0.as_ref() != result
        || root.1 != inputs.career.view.storage_enabled
        || shape.is_some_and(|shape| *shape != new_shape);
    match parts {
        Some(parts) if body_changed && !locale_changed(&locale) => {
            let model = result_model(&inputs.latch(), result, &inputs.career);
            let form = inputs.form();
            commands.entity(parts.body).despawn_related::<Children>();
            let mut banner = Entity::PLACEHOLDER;
            commands
                .entity(parts.body)
                .with_children(|body| banner = spawn_body(body, &model, form));
            commands.entity(entity).insert((
                PostMatchRoot(result.cloned(), inputs.career.view.storage_enabled),
                new_shape,
                PostMatchParts { banner, ..*parts },
            ));
            let (visibility, disabled) = details_state(&model.details);
            let details = parts.details;
            commands.entity(details).insert(visibility);
            commands.queue(move |world: &mut World| {
                if let Some(mut pressable) = world.get_mut::<Pressable>(details) {
                    pressable.disabled = disabled;
                }
            });
        }
        _ if body_changed || locale_changed(&locale) => {
            commands.entity(entity).despawn();
            build_screen(&mut commands, &inputs);
        }
        _ => {}
    }
}

/// Per-frame state that does not move a box: the status line, Play again,
/// Details, the next-round badge.
#[allow(clippy::too_many_arguments)]
fn sync_post_match(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    career: Res<CareerClient>,
    mobile: Option<Res<MobileControls>>,
    session: Option<Res<crate::net::ClientSession>>,
    mut latch: Option<ResMut<PostMatchLatch>>,
    roots: Query<(&PostMatchRoot, &PostMatchParts)>,
    mut lines: Query<&mut StatusLine>,
    mut pressables: Query<&mut Pressable>,
    mut nodes: Query<(&mut Node, Option<&KitParts>)>,
    mut visibility: Query<&mut Visibility>,
    mut texts: Query<&mut Text>,
) {
    let Ok((root, parts)) = roots.single() else {
        return;
    };
    if let Some(latch) = latch.as_mut()
        && game.rematch_in_secs.is_some()
        && !latch.rematch_seen
    {
        latch.rematch_seen = true;
    }
    let latch = latch.as_deref().cloned().unwrap_or_default();
    let form = Form::from_mobile(mobile.as_deref());
    let expects = expects_result(&latch, &career);
    // The receipt this screen shows (kept through a worker retirement).
    let result = root.0.as_ref();
    let link = session
        .as_deref()
        .map_or(LinkStatus::Connected, crate::net::link_status);
    let next_round = latch.rematch_seen.then(|| {
        game.rematch_in_secs.map_or_else(
            || tr("state.next_round.preparing").to_owned(),
            |seconds| trf("state.next_round.countdown", &[("seconds", &seconds)]),
        )
    });
    let spec = status_spec(
        expects,
        result,
        link,
        next_round.clone(),
        form == Form::Phone,
    );
    if let Ok(mut line) = lines.get_mut(parts.status)
        && line.0.as_ref() != Some(&spec)
    {
        commands.entity(parts.status).despawn_related::<Children>();
        commands
            .entity(parts.status)
            .with_children(|line| fill_status_line(line, &spec));
        line.0 = Some(spec);
    }
    let enabled = play_again_enabled(latch.play_again_pressed);
    if let Ok(mut pressable) = pressables.get_mut(parts.play_again)
        && pressable.disabled == enabled
    {
        pressable.disabled = !enabled;
    }
    let label = nodes
        .get(parts.play_again)
        .ok()
        .and_then(|(_, kit)| kit.and_then(|kit| kit.label));
    let pressed = latch.play_again_pressed;
    for (entity, shown) in [(label, !pressed), (Some(parts.play_again_spinner), pressed)] {
        if let Some((mut node, _)) = entity.and_then(|entity| nodes.get_mut(entity).ok()) {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let (Some(timer), Some(text)) = (parts.timer, parts.timer_text) {
        let shown = next_round.is_some();
        if let Ok(mut timer) = visibility.get_mut(timer) {
            let next = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *timer != next {
                *timer = next;
            }
        }
        if let (Some(line), Ok(mut text)) = (next_round, texts.get_mut(text))
            && text.0 != line
        {
            text.0 = line;
        }
    }
}

/// Entry motion: the background crossfades in over
/// `motion.duration.screen_fade`, the banner opens (scale from
/// `motion.panel_open.scale_from`) over `motion.duration.panel_open`; the
/// gained XP grows over `motion.duration.bar_trail`; a level-up flashes the
/// badge for `motion.duration.cooldown_ready_flash`.
#[allow(clippy::type_complexity)]
fn animate_post_match(
    time: Option<Res<Time>>,
    latch: Option<Res<PostMatchLatch>>,
    roots: Query<&PostMatchParts>,
    mut transforms: Query<&mut UiTransform>,
    mut segments: Query<(&mut Node, &mut GainedSegment)>,
    mut flashes: Query<(&mut BorderColor, &mut LevelFlash)>,
) {
    let (Some(time), Ok(parts)) = (time, roots.single()) else {
        return;
    };
    let now = time.elapsed_secs();
    let since = now - latch.as_ref().map_or(now, |latch| latch.entered_at);
    let open = (since / motion::DURATION_PANEL_OPEN.as_secs_f32()).clamp(0.0, 1.0);
    if let Ok(mut transform) = transforms.get_mut(parts.banner) {
        let from = motion::PANEL_OPEN_SCALE_FROM;
        let scale = Vec2::splat(from + (1.0 - from) * motion::EASING_ENTER.ease(open));
        if transform.scale != scale {
            transform.scale = scale;
        }
    }
    let trail = motion::DURATION_BAR_TRAIL.as_secs_f32();
    for (mut node, mut segment) in &mut segments {
        let started = *segment.started.get_or_insert(now);
        let t = ((now - started) / trail).clamp(0.0, 1.0);
        let width =
            Val::Percent((segment.to - segment.from) * motion::EASING_STANDARD.ease(t) * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
    let flash = motion::DURATION_COOLDOWN_READY_FLASH.as_secs_f32();
    for (mut border, mut level) in &mut flashes {
        let started = *level.started.get_or_insert(now);
        let lit = now - started < flash;
        let ink = if lit {
            color::GOLD_300
        } else {
            color::GOLD_500
        };
        if border.top != ink {
            *border = BorderColor::all(ink);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn post_match_actions(
    mut commands: MessageWriter<NetworkCommand>,
    mut session_ui: MessageWriter<SessionUiCommand>,
    mut activated: MessageReader<Activated<PostMatchAction>>,
    mut flow: Option<ResMut<crate::match_service::MatchServiceClient>>,
    mut career: Option<ResMut<CareerClient>>,
    mut latch: Option<ResMut<PostMatchLatch>>,
    party: Option<Res<crate::party::PartyClient>>,
    session: Option<Res<crate::net::ClientSession>>,
    game: Option<Res<GameStateSnapshot>>,
    time: Option<Res<Time>>,
    roots: Query<&PostMatchRoot>,
) {
    // No press counts for the first screen fade: a held attack or confirm
    // from the match must not press Play again (post-match.md).
    let locked = match (latch.as_ref(), time.as_ref()) {
        (Some(latch), Some(time)) => {
            time.elapsed_secs() - latch.entered_at < motion::DURATION_SCREEN_FADE.as_secs_f32()
                && roots.iter().next().is_some()
        }
        _ => false,
    };
    for Activated { action, .. } in activated.read() {
        if locked {
            continue;
        }
        match action {
            PostMatchAction::PlayAgain => {
                if latch.as_ref().is_some_and(|latch| latch.play_again_pressed) {
                    continue;
                }
                if flow.as_ref().is_some_and(|flow| flow.allocation.is_some()) {
                    let solo = !party.as_ref().is_some_and(|party| party.in_party());
                    if solo && let Some(flow) = flow.as_mut() {
                        flow.request_requeue();
                    }
                    // LeaveMatch clears the allocation before reconnecting to the
                    // lobby. The next FindMatch therefore has a fresh request ID.
                    session_ui.write(SessionUiCommand::LeaveMatch);
                } else if session
                    .as_ref()
                    .is_some_and(|session| !session.join_confirmed())
                    || (game
                        .as_ref()
                        .is_some_and(|game| matches!(game.state, GameState::Running))
                        && roots.single().is_ok_and(|root| root.0.is_some()))
                    || career.as_ref().is_some_and(|career| {
                        let expects = latch
                            .as_ref()
                            .is_some_and(|latch| expects_result(latch, career));
                        expects
                            && !roots
                                .single()
                                .ok()
                                .and_then(|root| root.0.as_ref())
                                .is_some_and(|result| result.saved)
                    })
                {
                    // Direct servers reject rematch while Running, including a
                    // saved abandonment/interruption with a stale live snapshot.
                    // Retired workers and pending saves cannot rematch either;
                    // use complete LeaveMatch cleanup and a usable Home instead.
                    session_ui.write(SessionUiCommand::LeaveMatch);
                } else {
                    commands.write(NetworkCommand::RequestRematch);
                }
                pressed(&mut latch);
            }
            PostMatchAction::Details => {
                if let Some(career) = career.as_mut() {
                    career.open_last_result_modal();
                }
            }
            PostMatchAction::BackToMenu => {
                session_ui.write(SessionUiCommand::LeaveMatch);
            }
        }
    }
}

fn pressed(latch: &mut Option<ResMut<PostMatchLatch>>) {
    if let Some(latch) = latch.as_mut() {
        latch.play_again_pressed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::screen_button;

    fn saved_receipt() -> MatchResult {
        serde_json::from_value(serde_json::json!({
            "result_id":"previous", "server_epoch":7, "match_id":1,
            "started_at_ms":0,"ended_at_ms":1000,"duration_ms":1000,
            "map_profile":"verdant_default","ruleset":"public-casual-v1",
            "outcome":"completed","winner":"green","rated":false,
            "unrated_reason":"allocated_bots","participants":[],"saved":true
        }))
        .unwrap()
    }

    fn full_receipt() -> MatchResult {
        let mut result = saved_receipt();
        result.match_id = 2;
        result.participants = (0..10).map(|index| serde_json::from_value(serde_json::json!({
            "player_id": 11 + index, "profile_id": if index == 0 {Some("p-1")} else {None},
            "nickname": format!("Player {index}"), "team": if index < 5 {"green"} else {"blue"},
            "hero_class":"mage", "character":"cube", "avatar":null, "sprite_character":null,
            "stats":{"kills":index,"deaths":2,"assists":3,"earned_gold":5000 + index * 100,"final_level":9},
            "disconnected":false,"rating":null,"progression_xp_gained":150
        })).unwrap()).collect();
        result
    }

    #[test]
    fn both_teams_use_final_earned_income_and_do_not_invent_legacy_gold() {
        let mut result = full_receipt();
        let latch = PostMatchLatch::default();
        let rows = team_stats(&latch, Some(&result));
        assert_eq!(rows.len(), 10);
        assert_eq!(
            team_totals(&rows, shared::map::Team::Green),
            ((10, 10, 15), Some(26000))
        );
        assert_eq!(
            team_totals(&rows, shared::map::Team::Blue),
            ((35, 10, 15), Some(28500))
        );
        result.participants[2].stats.earned_gold = None;
        let rows = team_stats(&latch, Some(&result));
        assert_eq!(rows[2].gold, None);
        assert_eq!(team_totals(&rows, shared::map::Team::Green).1, None);
        assert_eq!(team_totals(&rows, shared::map::Team::Blue).1, Some(28500));
        assert!(team_stats(&latch, Some(&saved_receipt())).is_empty());
    }

    #[test]
    fn phone_full_teams_and_totals_fit_above_result_actions() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut app = screen_app();
        app.add_plugins((
            bevy::asset::AssetPlugin::default(),
            bevy::image::ImagePlugin::default(),
            bevy::text::TextPlugin,
            bevy::transform::TransformPlugin,
            bevy::input::InputPlugin,
            bevy::ui::UiPlugin,
            bevy::camera::visibility::VisibilityPlugin,
            bevy::picking::PickingPlugin,
            bevy::picking::InteractionPlugin,
        ))
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<TextureAtlasLayout>>();
        let mut mobile = MobileControls::default();
        mobile.enabled = true;
        mobile.viewport = Vec2::new(852.0, 393.0);
        mobile.safe = crate::mobile_controls::MobileSafeInsets {
            left: 32.0,
            right: 32.0,
            top: 12.0,
            bottom: 20.0,
        };
        app.insert_resource(mobile);
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set(852.0, 393.0);
        app.world_mut().spawn((window, bevy::window::PrimaryWindow));
        app.world_mut().spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(852, 393),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
        ));
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view.storage_enabled = true;
            career.view.profile = Some(shared::career::ProfileSummary::new(
                "p-1".into(),
                "Guest".into(),
            ));
            career.public_profile_id = Some("p-1".into());
            career.view.last_result = Some(full_receipt());
        }
        app.finish();
        app.cleanup();
        enter(&mut app);
        for _ in 0..5 {
            app.update();
        }
        let rect = |app: &App, entity| {
            Rect::from_center_size(
                app.world()
                    .get::<UiGlobalTransform>(entity)
                    .unwrap()
                    .translation,
                app.world().get::<ComputedNode>(entity).unwrap().size(),
            )
        };
        let teams = named(&mut app, "PostMatchTeams").unwrap();
        let status = named(&mut app, "PostMatchStatus").unwrap();
        let actions = named(&mut app, "PostMatchActions").unwrap();
        let table = rect(&app, teams);
        let controls = rect(&app, actions);
        assert!(
            table.size().y >= 126.0 && table.max.y < controls.min.y,
            "{table:?} / {controls:?}"
        );
        let status = rect(&app, status);
        assert!(
            status.max.y <= controls.min.y,
            "status {status:?} must stay above actions {controls:?}; table {table:?}"
        );
        for index in 11..21 {
            let row = named(&mut app, &format!("PostMatchPlayer{index}")).unwrap();
            let row = rect(&app, row);
            assert!(row.size().y >= 18.0 && table.contains(row.center()));
            assert!(row.min.x >= table.min.x && row.max.x <= table.max.x + 0.1);
        }
        for team in ["Green", "Blue"] {
            let total = named(&mut app, &format!("PostMatchTotal{team}")).unwrap();
            assert!(table.contains(rect(&app, total).center()));
        }
    }

    fn screen_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<AppScreen>()
            .init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_plugins(PostMatchScreenPlugin);
        {
            let mut game = app.world_mut().resource_mut::<GameStateSnapshot>();
            game.meta = shared::protocol::SnapshotMeta::new(7, 2, 20);
            game.your_id = 11;
            game.state = GameState::Victory {
                winner: shared::map::Team::Green,
            };
            game.scoreboard = Some(shared::live_score::LiveScoreboard {
                elapsed_secs: 720,
                kills: Vec::new(),
                players: vec![LiveScorePlayer {
                    avatar: None,
                    player_id: 11,
                    nickname: "Guest".into(),
                    team: shared::map::Team::Blue,
                    hero_class: shared::HeroClass::Mage,
                    kills: 4,
                    deaths: 3,
                    assists: 6,
                    earned_gold: 6210,
                    level: 9,
                    connected: true,
                }],
            });
        }
        app.world_mut().spawn((crate::player::Player, Team::Blue));
        app
    }

    fn enter(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<AppScreen>>()
            .set(AppScreen::PostMatch);
        for _ in 0..3 {
            app.update();
        }
    }

    fn text_of(app: &mut App, name: &str) -> Option<String> {
        let mut query = app.world_mut().query::<(&Name, &Text)>();
        query
            .iter(app.world())
            .find(|(node, _)| node.as_str() == name)
            .map(|(_, text)| text.0.clone())
    }

    fn named(app: &mut App, name: &str) -> Option<Entity> {
        let mut query = app.world_mut().query::<(Entity, &Name)>();
        query
            .iter(app.world())
            .find(|(_, node)| node.as_str() == name)
            .map(|(entity, _)| entity)
    }

    fn by_id(app: &mut App, id: &str) -> Entity {
        let mut query = app.world_mut().query::<(Entity, &TestId)>();
        query
            .iter(app.world())
            .find(|(_, test_id)| test_id.as_str() == id)
            .map(|(entity, _)| entity)
            .unwrap()
    }

    /// A guest on a rematch server: live numbers, no damage, no progress,
    /// nothing waits; Play again rematches at once; the badge counts.
    #[test]
    fn a_guest_sees_live_numbers_and_the_next_round_without_waiting() {
        let mut app = screen_app();
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .rematch_in_secs = Some(10);
        enter(&mut app);
        assert_eq!(
            text_of(&mut app, "PostMatchOutcome").as_deref(),
            Some("Defeat")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchSummaryText").as_deref(),
            Some("Green destroyed the enemy base")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchKdaValue").as_deref(),
            Some("4 / 3 / 6")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchGoldValue").as_deref(),
            Some("6210")
        );
        assert!(
            named(&mut app, "PostMatchDamage").is_none(),
            "no result will come"
        );
        assert!(named(&mut app, "PostMatchProgress").is_none());
        assert_eq!(
            text_of(&mut app, "PostMatchStatusText").as_deref(),
            Some("Local practice result")
        );
        let details = by_id(&mut app, "PostMatchDetails");
        assert_eq!(
            app.world().get::<Visibility>(details),
            Some(&Visibility::Hidden)
        );
        let play = by_id(&mut app, "PostMatchPlayAgain");
        assert!(!app.world().get::<Pressable>(play).unwrap().disabled);
        let timer = named(&mut app, "PostMatchRoundTimer").unwrap();
        assert_eq!(
            app.world().get::<Visibility>(timer),
            Some(&Visibility::Inherited)
        );
    }

    /// A career profile: finalizing skeletons and Details wait; Play again stays usable;
    /// the saved result fills the same boxes without replacing the buttons.
    #[test]
    fn the_career_result_fills_the_finalizing_screen_in_place() {
        let mut app = screen_app();
        {
            let mut career = app.world_mut().resource_mut::<CareerClient>();
            career.view.storage_enabled = true;
            career.view.profile = Some(shared::career::ProfileSummary {
                progression_xp: 2450,
                ..shared::career::ProfileSummary::new("p-1".into(), "Guest".into())
            });
            career.public_profile_id = Some("p-1".into());
        }
        enter(&mut app);
        assert!(app.world().resource::<CareerClient>().hold_result_modal);
        assert!(named(&mut app, "PostMatchDamage").is_some());
        assert!(
            text_of(&mut app, "PostMatchDamageValue").is_none(),
            "skeleton"
        );
        assert!(named(&mut app, "PostMatchProgress").is_some());
        assert!(
            text_of(&mut app, "PostMatchCareerLine").is_none(),
            "skeleton"
        );
        assert_eq!(
            text_of(&mut app, "PostMatchStatusText").as_deref(),
            Some("Saving match results…")
        );
        let play = by_id(&mut app, "PostMatchPlayAgain");
        let details = by_id(&mut app, "PostMatchDetails");
        assert!(!app.world().get::<Pressable>(play).unwrap().disabled);
        assert!(app.world().get::<Pressable>(details).unwrap().disabled);
        assert_eq!(
            app.world().get::<Visibility>(details),
            Some(&Visibility::Inherited)
        );

        let mut result = saved_receipt();
        result.match_id = 2;
        result.duration_ms = 12 * 60_000;
        result.unrated_reason = None;
        result.participants = vec![
            serde_json::from_value(serde_json::json!({
                "player_id": 11, "profile_id": "p-1", "nickname": "Guest", "team": "blue",
                "hero_class": "mage", "character": "cube", "avatar": null,
                "sprite_character": null,
                "stats": {"kills":4,"deaths":3,"assists":6,"damage_to_heroes":38762.4,
                    "damage_to_structures":0.0,"damage_to_creeps":0.0,"damage_taken":0.0,
                    "minion_last_hits":0,"jungle_last_hits":0,"structures_destroyed":0,
                    "final_level":9},
                "disconnected": false,
                "rating": {"before": 1200, "after": 1216, "delta": 16},
                "progression_xp_gained": 150
            }))
            .unwrap(),
        ];
        app.world_mut()
            .resource_mut::<CareerClient>()
            .view
            .last_result = Some(result);
        for _ in 0..2 {
            app.update();
        }
        assert_eq!(
            text_of(&mut app, "PostMatchSummaryText").as_deref(),
            Some("Green destroyed the enemy base · 12 min")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchDamageValue").as_deref(),
            Some("38762")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchCareerLine").as_deref(),
            Some("Your rating: 1216 (+16)  ·  +150 career XP")
        );
        assert_eq!(
            text_of(&mut app, "PostMatchStatusText").as_deref(),
            Some("Progress saved")
        );
        assert_eq!(by_id(&mut app, "PostMatchPlayAgain"), play, "not rebuilt");
        assert!(!app.world().get::<Pressable>(play).unwrap().disabled);
        assert!(!app.world().get::<Pressable>(details).unwrap().disabled);
        assert!(
            app.world().resource::<CareerClient>().modal == crate::career::CareerModal::Closed,
            "the result does not open the career page over this screen"
        );
    }

    /// No match behind it (the layout fixture): "Match complete", no
    /// summary, no stats, local result.
    #[test]
    fn the_layout_fixture_is_a_neutral_complete_screen() {
        let mut app = screen_app();
        {
            let mut game = app.world_mut().resource_mut::<GameStateSnapshot>();
            game.state = GameState::Running;
            game.scoreboard = None;
        }
        let players: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<crate::player::Player>>()
            .iter(app.world())
            .collect();
        for player in players {
            app.world_mut().despawn(player);
        }
        enter(&mut app);
        assert_eq!(
            text_of(&mut app, "PostMatchOutcome").as_deref(),
            Some("Match complete")
        );
        assert!(text_of(&mut app, "PostMatchSummaryText").is_none());
        assert!(named(&mut app, "PostMatchStats").is_none());
        assert_eq!(
            text_of(&mut app, "PostMatchStatusText").as_deref(),
            Some("Local practice result")
        );
    }

    #[test]
    fn play_again_is_available_without_waiting_for_persistence() {
        assert!(play_again_enabled(false));
        assert!(!play_again_enabled(true));
        assert_eq!(result_ui_scale(1280.0, 720.0), 1.0);
        assert_eq!(result_ui_scale(1920.0, 1080.0), 1.5);
        assert_eq!(result_ui_scale(1024.0, 640.0), 0.8);
    }

    #[test]
    fn prior_saved_receipt_cannot_claim_current_match_saved() {
        let mut career = CareerClient::default();
        let mut game = GameStateSnapshot::default();
        game.meta.server_epoch = 7;
        game.meta.match_id = 2;
        let result = saved_receipt();
        career.view.last_result = Some(result);
        assert!(current_result(&career, &game).is_none());
        let receipt = career.view.last_result.as_mut().unwrap();
        receipt.match_id = 2;
        receipt.saved = false;
        assert!(!current_result(&career, &game).unwrap().saved);
        career.view.last_result.as_mut().unwrap().saved = true;
        assert!(current_result(&career, &game).unwrap().saved);
        game.meta.server_epoch += 1;
        assert!(current_result(&career, &game).is_none());
    }

    #[test]
    fn worker_retirement_preserves_rendered_terminal_panel() {
        let mut app = App::new();
        app.init_resource::<CareerClient>()
            .init_resource::<GameStateSnapshot>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<PostMatchAction>()
            .add_systems(
                Update,
                (
                    refresh_post_match,
                    post_match_actions.after(UiSet::Dispatch),
                )
                    .chain(),
            );
        let mut flow = crate::match_service::MatchServiceClient::default();
        flow.allocation = Some(shared::match_service::MatchAllocation {
            allocation_id: "allocated".into(),
            endpoint: "127.0.0.1:4001".into(),
            preference: shared::match_service::MatchPreference::BotPractice,
            team: shared::map::Team::Green,
            human_count: 1,
            bot_count: 9,
            rated: false,
            join_deadline_ms: 0,
        });
        app.insert_resource(flow);
        let panel = app
            .world_mut()
            .spawn((
                PostMatchRoot(
                    Some(MatchResult {
                        outcome: MatchOutcome::Abandoned,
                        saved: false,
                        ..saved_receipt()
                    }),
                    true,
                ),
                Name::new("RetainedTerminal"),
            ))
            .id();
        app.update();
        assert!(
            app.world().get_entity(panel).is_ok(),
            "transport teardown must not erase the terminal panel"
        );
        // A reused UDP port may expose another arena's bootstrap epoch.
        app.world_mut()
            .resource_mut::<GameStateSnapshot>()
            .meta
            .server_epoch = 999;
        app.update();
        assert!(app.world().get_entity(panel).is_ok());
        app.world_mut().spawn((
            Button,
            Interaction::Pressed,
            crate::ui::UiAction(PostMatchAction::PlayAgain),
        ));
        app.update();
        assert_eq!(
            app.world().resource::<Messages<SessionUiCommand>>().len(),
            1,
            "even a pending abandoned receipt must permit returning to the lobby"
        );
    }

    #[test]
    fn saved_terminal_receipt_with_running_direct_server_returns_home_on_replay() {
        for outcome in [MatchOutcome::Abandoned, MatchOutcome::Interrupted] {
            let mut app = App::new();
            app.add_message::<NetworkCommand>()
                .add_message::<SessionUiCommand>()
                .add_ui_action::<PostMatchAction>()
                .init_resource::<PostMatchLatch>()
                .insert_resource(crate::net::ClientSession::admitted_for_test())
                .insert_resource(GameStateSnapshot {
                    state: GameState::Running,
                    meta: shared::protocol::SnapshotMeta::new(7, 1, 20),
                    ..default()
                })
                .add_systems(Update, post_match_actions.after(UiSet::Dispatch));
            app.world_mut().spawn(PostMatchRoot(
                Some(MatchResult {
                    outcome,
                    saved: true,
                    ..saved_receipt()
                }),
                true,
            ));
            app.world_mut().spawn((
                Button,
                Interaction::Pressed,
                crate::ui::UiAction(PostMatchAction::PlayAgain),
            ));
            app.update();
            let exits: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .collect();
            assert!(matches!(exits.as_slice(), [SessionUiCommand::LeaveMatch]));
            assert!(
                app.world()
                    .resource::<Messages<NetworkCommand>>()
                    .is_empty(),
                "a direct server rejects RequestRematch while its snapshot is Running"
            );
        }
    }

    #[test]
    fn the_headline_is_written_from_the_local_point_of_view() {
        assert_eq!(
            outcome_headline(Some(Team::Green), Some(Team::Green)),
            "Victory"
        );
        assert_eq!(
            outcome_headline(Some(Team::Green), Some(Team::Blue)),
            "Defeat"
        );
        assert_eq!(outcome_headline(Some(Team::Green), None), "Match complete");
        assert_eq!(outcome_headline(None, Some(Team::Blue)), "Match complete");
    }

    #[test]
    fn post_match_presses_dispatch_once_and_disabled_buttons_do_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<PostMatchAction>()
            .add_systems(Update, post_match_actions.after(UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |row| {
            screen_button(
                row,
                "Play again",
                ButtonKind::Primary,
                PostMatchAction::PlayAgain,
                "PostMatchPlayAgain",
            );
            screen_button(
                row,
                "Back to menu",
                ButtonKind::Secondary,
                PostMatchAction::BackToMenu,
                "PostMatchBackToMenu",
            );
        });
        app.update();
        harness::press(app.world_mut(), "PostMatchPlayAgain");
        app.update();
        assert_eq!(
            harness::drain_actions::<PostMatchAction>(app.world_mut()),
            [PostMatchAction::PlayAgain]
        );
        let rematches = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<NetworkCommand>>()
                .drain()
                .filter(|command| matches!(command, NetworkCommand::RequestRematch))
                .count()
        };
        assert_eq!(rematches(&mut app), 1);
        app.update();
        assert_eq!(rematches(&mut app), 0);
        harness::set_disabled(app.world_mut(), "PostMatchBackToMenu", true);
        harness::press(app.world_mut(), "PostMatchBackToMenu");
        app.update();
        assert!(
            app.world()
                .resource::<Messages<SessionUiCommand>>()
                .is_empty()
        );
    }
}
