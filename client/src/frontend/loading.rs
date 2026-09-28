//! One server-owned countdown and an asset barrier shared by the whole roster.
//!
//! The screen is the Verdant Crown loading shell (frame, header, footer)
//! with one of three bodies: the connecting block while there is no roster
//! (`omoba-ui/handoff/screens/loading-shell.md`: status ring, headline,
//! detail line, Retry), the countdown ring over the roster during the 3 s
//! after the last lock-in (`prematch-countdown.md`), and the roster itself
//! while the map loads (`loading-teams.md`, P1: its rows are unchanged).
//! Boxes are rebuilt only when the phase, the roster, the window or the
//! language change; the status texts, the rings and the counts are written
//! every frame.
//!
//! Text comes from the `loading` dictionary; the render key includes the
//! locale generation. Server-authored draft errors stay English.
// i18n-strict
use bevy::{
    asset::RecursiveDependencyLoadState,
    ecs::system::SystemParam,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
    window::PrimaryWindow,
};
use shared::prematch::PrematchPhase;

use super::{
    AppScreen,
    draft::{self, DraftClient, DraftScrollMemory, DraftSet},
};
use crate::{
    i18n::{Locale, tr, trf},
    mobile_controls::MobileControls,
    model_scale::ModelScaleSource,
    net::{
        GameState, GameStateSnapshot, LinkStatus, NetworkAvatar, NetworkCharacterChoice,
        NetworkPlayerId, NetworkSpriteCharacter, SessionUiCommand,
    },
    sprite::{PlayerSpriteVisual, PlayerVisualMode},
    team::{AvatarThumbnails, CharacterChoice},
    ui::{
        Activated, FocusEntry, TestId, UiActionAppExt,
        kit_assets::{Background, CoverImage, Frame, Icon, KitImage},
        theme::{self, ButtonKind, Form, TextStyle},
        tokens::{TextRole, border, color, radius, size, space},
        widgets::{
            ButtonSize, button_node,
            game::RingSize,
            icon_node, spawn_button,
            status::{RingMode, status_ring},
            surfaces,
        },
    },
    verdant3d::{VerdantEnvironment, VerdantFoliage, VerdantStructureVisual},
    world::AvatarAssetCache,
    world2d::World2dStatic,
};

/// Dictionary keys of the loading tips.
const TIPS: [&str; 5] = [
    "loading.tip.last_hit",
    "loading.tip.towers",
    "loading.tip.jungle",
    "loading.tip.minimap",
    "loading.tip.ultimate",
];
/// The dictionary key of the tip shown for `index` (show it with `tr`).
pub fn tip_for(index: usize) -> &'static str {
    TIPS[index % TIPS.len()]
}

pub struct LoadingScreenPlugin;
impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        preload_screen_art(app);
        app.add_ui_action::<LoadingAction>()
            .init_resource::<LoadingLatch>()
            .add_systems(OnEnter(AppScreen::Loading), enter_loading)
            .add_systems(
                Update,
                (loading_actions, assess_readiness)
                    .chain()
                    .in_set(DraftSet::Input)
                    .run_if(in_state(AppScreen::Loading)),
            )
            .add_systems(
                Update,
                (render_loading, sync_loading)
                    .chain()
                    .in_set(DraftSet::Draw)
                    .before(crate::ui::UiSet::Paint)
                    .run_if(in_state(AppScreen::Loading)),
            );
    }
}

/// The kit textures the loading and result screens draw on their first
/// frame (frames, slabs, the ring atlas, their icons): kept loaded so a
/// 3 s countdown never shows a frame without its ring or frame. The
/// full-screen backgrounds are not kept (they fade in).
fn preload_screen_art(app: &mut App) {
    use crate::ui::kit_assets::{KitPreload, KitSource, Sprite};
    let frames = [
        Frame::Ornament,
        Frame::Panel,
        Frame::ButtonPrimary,
        Frame::ButtonPrimaryHover,
        Frame::ButtonPrimaryPressed,
        Frame::ButtonPrimaryDisabled,
        Frame::ButtonSecondary,
        Frame::ButtonSecondaryHover,
        Frame::ButtonSecondaryPressed,
        Frame::ButtonSecondaryDisabled,
    ];
    let icons = [
        Icon::NavLock,
        Icon::NavInfo,
        Icon::NavAlertTriangle,
        Icon::NavWifiOff,
        Icon::NavRefreshCw,
        Icon::NavCrown,
        Icon::NavCheck,
        Icon::NavTimer,
        Icon::HudKill,
        Icon::HudAttack,
        Icon::HudGold,
    ];
    let Some(mut preload) = app.world_mut().get_resource_mut::<KitPreload>() else {
        return;
    };
    preload.add(frames.map(|frame| (KitSource::Frame(frame), false)));
    preload.add(icons.map(|icon| (KitSource::Icon(icon), false)));
    preload.add([(KitSource::Sprite(Sprite::TimerRingAtlas), true)]);
}

#[derive(Component)]
struct LoadingRoot;

/// Runs in `DraftSet::Input`, after `UiSet::Dispatch`.
fn loading_actions(
    mut activated: MessageReader<Activated<LoadingAction>>,
    mut state: ResMut<DraftClient>,
    mut session: MessageWriter<SessionUiCommand>,
) {
    let mut cancel = false;
    let mut retry = false;
    for pressed in activated.read() {
        match pressed.action {
            LoadingAction::Cancel => cancel = true,
            LoadingAction::Retry => retry = true,
        }
    }
    if cancel {
        state.reset();
        session.write(SessionUiCommand::LeaveMatch);
    } else if retry {
        // The same command as the in-match connection panel's Retry.
        session.write(SessionUiCommand::Retry);
    }
}

#[derive(SystemParam)]
struct LoadingAssets<'w, 's> {
    server: Res<'w, AssetServer>,
    spawner: Res<'w, SceneSpawner>,
    cache: Res<'w, AvatarAssetCache>,
    mode: Res<'w, PlayerVisualMode>,
    meshes: Res<'w, Assets<Mesh>>,
    materials: Res<'w, Assets<StandardMaterial>>,
    procedural: Query<'w, 's, (&'static Mesh3d, &'static MeshMaterial3d<StandardMaterial>)>,
    players: Query<
        'w,
        's,
        (
            Entity,
            &'static NetworkPlayerId,
            &'static NetworkAvatar,
            &'static NetworkCharacterChoice,
            Option<&'static NetworkSpriteCharacter>,
            Option<&'static ModelScaleSource>,
        ),
    >,
    scenes: Query<'w, 's, (&'static SceneRoot, Option<&'static SceneInstance>)>,
    children: Query<'w, 's, &'static Children>,
    map: Query<
        'w,
        's,
        Entity,
        Or<(
            With<VerdantEnvironment>,
            With<VerdantFoliage>,
            With<VerdantStructureVisual>,
        )>,
    >,
    environment: Query<'w, 's, Entity, With<VerdantEnvironment>>,
    foliage: Query<'w, 's, Entity, With<VerdantFoliage>>,
    map_sprites: Query<'w, 's, &'static Sprite, With<World2dStatic>>,
    player_sprites: Query<'w, 's, (&'static PlayerSpriteVisual, &'static Sprite)>,
}
impl LoadingAssets<'_, '_> {
    fn scene_ready(&self, entity: Entity) -> bool {
        self.scenes.get(entity).is_ok_and(|(root, instance)| {
            instance.is_some_and(|instance| self.spawner.instance_is_ready(**instance))
                && matches!(
                    self.server.recursive_dependency_load_state(root.0.id()),
                    RecursiveDependencyLoadState::Loaded
                )
        })
    }
    fn hero_scene_ready(&self, entity: Entity) -> bool {
        self.scene_ready(entity)
            || self
                .children
                .get(entity)
                .is_ok_and(|children| children.iter().any(|child| self.scene_ready(child)))
    }
    fn image_ready(&self, image: &Handle<Image>) -> bool {
        matches!(
            self.server.recursive_dependency_load_state(image.id()),
            RecursiveDependencyLoadState::Loaded
        )
    }
    fn cube_ready(&self, entity: Entity) -> bool {
        let ready = |entity| {
            procedural_assets_ready(
                CharacterChoice::Cube,
                None,
                self.procedural.get(entity).ok(),
                &self.meshes,
                &self.materials,
            )
        };
        ready(entity)
            || self
                .children
                .get(entity)
                .is_ok_and(|children| children.iter().any(ready))
    }
}

fn procedural_assets_ready(
    character: CharacterChoice,
    avatar: Option<&str>,
    rendered: Option<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    meshes: &Assets<Mesh>,
    materials: &Assets<StandardMaterial>,
) -> bool {
    character == CharacterChoice::Cube
        && avatar.is_none()
        && rendered.is_some_and(|(mesh, material)| {
            meshes.contains(&mesh.0) && materials.contains(&material.0)
        })
}

fn assess_readiness(
    game: Res<GameStateSnapshot>,
    mut state: ResMut<DraftClient>,
    assets: LoadingAssets,
) {
    let map_ready = || {
        if *assets.mode == PlayerVisualMode::Models3d {
            !assets.environment.is_empty()
                && !assets.foliage.is_empty()
                && assets.map.iter().all(|entity| assets.scene_ready(entity))
        } else {
            !assets.map_sprites.is_empty()
                && assets
                    .map_sprites
                    .iter()
                    .all(|sprite| assets.image_ready(&sprite.image))
        }
    };
    let Some(draft) = &game.prematch else {
        // No roster (loading-shell.md): the map, then the player's own
        // Studio avatar, is all this screen can report.
        state.local_assets = own_assets(map_ready(), &game, &assets).into();
        return;
    };
    if !matches!(
        draft.phase,
        PrematchPhase::Countdown | PrematchPhase::Loading
    ) {
        return;
    }
    if !map_ready() {
        state.local_assets = tr("loading.assets.battlefield").into();
        return;
    }
    for selected in &draft.players {
        let Some((entity, _, avatar, character, sprite_character, source)) = assets
            .players
            .iter()
            .find(|(_, id, _, _, _, _)| id.0 == selected.player_id)
        else {
            state.local_assets = tr("loading.assets.team_heroes").into();
            return;
        };
        if avatar.0 != selected.avatar || character.0 != selected.character {
            state.local_assets = tr("loading.assets.applying").into();
            return;
        }
        if *assets.mode == PlayerVisualMode::Sprite2d {
            if sprite_character.is_none_or(|sprite| sprite.0 != selected.sprite_character)
                || !assets.player_sprites.iter().any(|(visual, sprite)| {
                    visual.owner() == entity && assets.image_ready(&sprite.image)
                })
            {
                state.local_assets = tr("loading.assets.sprites").into();
                return;
            }
            continue;
        }
        // Cube is an intentional built-in mesh, not an unready model fallback.
        // This exemption can never apply to a selected SDK/roster avatar.
        if selected.character == CharacterChoice::Cube && selected.avatar.is_none() {
            if assets.cube_ready(entity) {
                continue;
            }
            state.local_assets = tr("loading.assets.built_in").into();
            return;
        }
        if let Some(slug) = &selected.avatar {
            if omoba_passport::avatars::avatar_definition(slug).is_none() {
                omoba_passport::store::request_refresh();
                state.local_assets = tr("loading.assets.refreshing").into();
                return;
            }
            if omoba_passport::store::knows(slug) {
                match omoba_passport::store::model_state(slug) {
                    omoba_passport::store::ModelState::Pending => {
                        state.local_assets = tr("loading.assets.downloading").into();
                        return;
                    }
                    omoba_passport::store::ModelState::Unavailable => {
                        state.local_assets = tr("loading.assets.unavailable").into();
                        return;
                    }
                    omoba_passport::store::ModelState::Ready => {}
                }
            }
            // NetworkAvatar can already name the choice while a fallback is
            // still instantiated. Require the final cached asset as well.
            let expected = assets
                .cache
                .requested()
                .find(|(cached, _)| *cached == slug)
                .map(|(_, handle)| handle);
            if !matching_avatar_asset(expected, source.map(|source| &source.gltf)) {
                state.local_assets = tr("loading.assets.avatar_models").into();
                return;
            }
        }
        if !assets.hero_scene_ready(entity)
            || source.is_some_and(|source| {
                !matches!(
                    assets
                        .server
                        .recursive_dependency_load_state(source.gltf.id()),
                    RecursiveDependencyLoadState::Loaded
                )
            })
        {
            state.local_assets = tr("loading.assets.animations").into();
            return;
        }
    }
    state.local_assets = tr("loading.assets.ready").into();
    if draft.phase == PrematchPhase::Loading && !draft.players.is_empty() {
        state.request_loaded();
    }
}

/// The asset line without a roster: the battlefield, then the own Studio
/// avatar's download state, then ready.
fn own_assets(map_ready: bool, game: &GameStateSnapshot, assets: &LoadingAssets) -> &'static str {
    if !map_ready {
        return tr("loading.assets.battlefield");
    }
    let own = assets
        .players
        .iter()
        .find(|(_, id, ..)| id.0 == game.your_id)
        .and_then(|(_, _, avatar, ..)| avatar.0.as_deref());
    match own.filter(|slug| omoba_passport::store::knows(slug)) {
        Some(slug) => match omoba_passport::store::model_state(slug) {
            omoba_passport::store::ModelState::Pending => tr("loading.assets.downloading"),
            omoba_passport::store::ModelState::Unavailable => tr("loading.assets.unavailable"),
            omoba_passport::store::ModelState::Ready => tr("loading.assets.ready"),
        },
        None => tr("loading.assets.ready"),
    }
}

fn matching_avatar_asset<T: Asset>(
    expected: Option<&Handle<T>>,
    actual: Option<&Handle<T>>,
) -> bool {
    expected
        .zip(actual)
        .is_some_and(|(expected, actual)| expected == actual)
}

/// What the loading screen latched on entry, and the countdown it follows.
#[derive(Resource, Default, Debug)]
pub(super) struct LoadingLatch {
    /// `Time::elapsed_secs` on entry (the slow line after 30 s).
    entered_at: f32,
    /// The tip, fixed for the whole wait: a teardown resets the match id.
    tip: usize,
    /// The first `countdown_ms` of the current `GameState::Starting` run
    /// (servers without a draft; the client does not know the constant).
    starting_total: Option<u32>,
    /// The last prematch countdown seen and when (`remaining_ms`, seconds):
    /// the arc drains per frame between snapshots.
    countdown: Option<(u32, f32)>,
}

/// Clears the previous wait: the tip, the clock and the asset line.
fn enter_loading(
    mut latch: ResMut<LoadingLatch>,
    mut state: ResMut<DraftClient>,
    game: Res<GameStateSnapshot>,
    time: Option<Res<Time>>,
) {
    *latch = LoadingLatch {
        entered_at: time.map_or(0.0, |time| time.elapsed_secs()),
        tip: game.meta.match_id as usize,
        ..default()
    };
    state.local_assets.clear();
}

/// Seconds without a roster or a start before the slow line
/// (`LOADING_TIMEOUT_MS`, the same length as the asset barrier).
const SLOW_AFTER_SECS: f32 = shared::prematch::LOADING_TIMEOUT_MS as f32 / 1000.0;

/// The headline of the connecting body.
#[derive(Clone, Debug, PartialEq)]
struct Headline {
    text: &'static str,
    ink: Color,
    /// `nav/wifi-off` before it (reconnecting).
    wifi: bool,
}

/// The detail line under it.
#[derive(Clone, Debug, PartialEq)]
struct Detail {
    text: String,
    ink: Color,
    /// `nav/alert-triangle` before it (a failure).
    alert: bool,
}

/// Everything the connecting body shows (`loading-shell.md` status
/// priority, first match wins).
#[derive(Clone, Debug, PartialEq)]
struct ShellStatus {
    ring: RingMode,
    headline: Headline,
    detail: Option<Detail>,
    retry: bool,
    /// `loading.ready_count` while a server without a draft is forming.
    stage: Option<String>,
}

fn shell_status(
    link: LinkStatus,
    state: &GameState,
    rematch_in_secs: Option<u64>,
    starting_total: Option<u32>,
    slow: bool,
) -> ShellStatus {
    let connecting = Headline {
        text: tr("loading.connecting"),
        ink: color::TEXT_GOLD,
        wifi: false,
    };
    let failed = Headline {
        text: tr("loading.shell.failed"),
        ink: color::TEXT_DANGER,
        wifi: false,
    };
    let failure = |text: String| ShellStatus {
        ring: RingMode::Error,
        headline: failed.clone(),
        detail: Some(Detail {
            text,
            ink: color::TEXT_DANGER,
            alert: true,
        }),
        retry: link.can_retry(),
        stage: None,
    };
    let waiting = |detail: Option<String>| ShellStatus {
        ring: RingMode::Indeterminate,
        headline: connecting.clone(),
        detail: detail.map(|text| Detail {
            text,
            ink: color::TEXT_SECONDARY,
            alert: false,
        }),
        retry: false,
        stage: None,
    };
    let mut status = match link {
        LinkStatus::Rejected(_) | LinkStatus::Unconfirmed | LinkStatus::Disconnected => {
            return failure(link.detail().unwrap_or_default());
        }
        LinkStatus::Reconnecting { .. } => {
            return ShellStatus {
                ring: RingMode::Indeterminate,
                headline: Headline {
                    ink: color::STATE_WARNING,
                    wifi: true,
                    ..connecting
                },
                detail: link.detail().map(|text| Detail {
                    text,
                    ink: color::STATE_WARNING,
                    alert: false,
                }),
                retry: false,
                stage: None,
            };
        }
        LinkStatus::Connecting => waiting(None),
        LinkStatus::Joining { .. } => waiting(link.detail()),
        LinkStatus::Connected => match state {
            GameState::Starting { countdown_ms } => {
                let total = starting_total.unwrap_or(*countdown_ms).max(1);
                ShellStatus {
                    ring: RingMode::Countdown {
                        progress: *countdown_ms as f32 / total as f32,
                        number: countdown_ms.div_ceil(1000).max(1),
                    },
                    ..waiting(None)
                }
            }
            GameState::Forming { ready, needed } => ShellStatus {
                stage: Some(trf(
                    "loading.ready_count",
                    &[("ready", ready), ("total", needed)],
                )),
                ..waiting(None)
            },
            GameState::Victory { .. } => waiting(Some(rematch_in_secs.map_or_else(
                || tr("state.next_round.preparing").to_owned(),
                |seconds| trf("state.next_round.countdown", &[("seconds", &seconds)]),
            ))),
            GameState::Lobby | GameState::Running => waiting(None),
        },
    };
    if slow && status.detail.is_none() {
        status.detail = Some(Detail {
            text: tr("loading.shell.slow").to_owned(),
            ink: color::TEXT_SECONDARY,
            alert: false,
        });
    }
    status
}

/// Parts of the screen the per-frame sync writes.
#[derive(Component, Clone, Copy)]
struct LoadingParts {
    ring: Option<Entity>,
    headline: Option<Entity>,
    headline_icon: Option<Entity>,
    detail: Option<Entity>,
    detail_icon: Option<Entity>,
    retry: Option<Entity>,
    stage: Entity,
    assets: Entity,
    assets_icon: Entity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadingAction {
    Cancel,
    Retry,
}

/// The phone header countdown ring sits `safe left + 432` (after the title
/// box shortened to 400).
const PHONE_RING_X: f32 = 432.0;

fn render_loading(
    mut commands: Commands,
    game: Res<GameStateSnapshot>,
    windows: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<LoadingRoot>>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    assets: Res<AssetServer>,
    scroll: Res<DraftScrollMemory>,
    locale: Option<Res<Locale>>,
    mobile: Option<Res<MobileControls>>,
    latch: Option<Res<LoadingLatch>>,
    mut last: Local<String>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let form = Form::from_mobile(mobile.as_deref());
    let phase = game.prematch.as_ref().map(|p| p.phase);
    let roster_key = game.prematch.as_ref().map(|p| (&p.players, &p.error));
    let key = format!(
        "{phase:?}:{}:{}:{}:{}:{form:?}",
        serde_json::to_string(&roster_key).unwrap_or_default(),
        window.width(),
        window.height(),
        locale.as_ref().map_or(0, |locale| locale.generation())
    );
    if *last == key && !roots.is_empty() {
        return;
    }
    *last = key;
    for root in &roots {
        commands.entity(root).despawn();
    }
    for entry in crate::passport::avatar_catalogue().entries {
        if let Some(path) = crate::passport::thumbnail_asset_path(&entry.avatar) {
            thumbnails
                .0
                .insert(entry.avatar.slug.clone(), assets.load(path));
        }
    }
    let safe = mobile.as_deref().map(|mobile| mobile.safe);
    let tip = tip_for(
        latch
            .as_ref()
            .map_or(game.meta.match_id as usize, |latch| latch.tip),
    );
    let countdown = phase == Some(PrematchPhase::Countdown);
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
            BackgroundColor(color::SURFACE_0),
            ZIndex(theme::SCREEN_Z),
            DespawnOnExit(AppScreen::Loading),
            LoadingRoot,
            Name::new("LoadingScreen"),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    ..default()
                },
                KitImage::background(Background::MenuArena),
                CoverImage { anchor_y: 0.5 },
                Pickable::IGNORE,
            ));
            let shell = Shell::of(form, safe);
            if form == Form::Desktop {
                root.spawn(surfaces::ornament_frame());
            }
            spawn_header(root, &shell, phase);
            let mut ring = None;
            if countdown {
                ring = Some(spawn_countdown_ring(root, &shell));
            }
            let body = match &game.prematch {
                Some(prematch) => {
                    spawn_roster(root, &shell, prematch, game.your_id, &thumbnails, &scroll);
                    ShellParts::default()
                }
                None => spawn_connecting_body(root, &shell),
            };
            let footer = spawn_footer(root, &shell, tip, game.prematch.is_none());
            parts = Some(LoadingParts {
                ring: ring.or(body.ring),
                headline: body.headline,
                headline_icon: body.headline_icon,
                detail: body.detail,
                detail_icon: body.detail_icon,
                retry: body.retry,
                stage: footer.0,
                assets: footer.1,
                assets_icon: footer.2,
            });
        })
        .insert(parts.expect("loading layout spawned"));
}

/// Where the shell's header, body and footer go on this layout family.
struct Shell {
    form: Form,
    /// Left/right content inset (desktop: inside the frame; phone: safe
    /// area + screen margin).
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl Shell {
    fn of(form: Form, safe: Option<crate::mobile_controls::MobileSafeInsets>) -> Self {
        match form {
            Form::Desktop => Self {
                form,
                left: 40.0,
                right: 40.0,
                top: 28.0,
                bottom: 48.0,
            },
            Form::Phone => {
                let margin = space::SCREEN_MARGIN.phone;
                let (left, right, top, bottom) = safe.map_or((32.0, 32.0, 0.0, 20.0), |safe| {
                    (safe.left, safe.right, safe.top, safe.bottom)
                });
                Self {
                    form,
                    left: left + margin,
                    right: right + margin,
                    top: top + space::S12,
                    bottom: bottom + space::S16,
                }
            }
        }
    }

    fn desktop(&self) -> bool {
        self.form == Form::Desktop
    }
}

fn absolute(left: Val, right: Val, top: Val, bottom: Val) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left,
        right,
        top,
        bottom,
        ..default()
    }
}

/// Title (banner strip on desktop), subtitle and Cancel: the header of every
/// loading body (loading-teams.md, imported by loading-shell.md and
/// prematch-countdown.md).
fn spawn_header(root: &mut ChildSpawnerCommands, shell: &Shell, phase: Option<PrematchPhase>) {
    let countdown = phase == Some(PrematchPhase::Countdown);
    let title = if countdown {
        tr("loading.title.team_ready")
    } else {
        tr("loading.title.entering")
    };
    let subtitle = if countdown {
        tr("loading.subtitle.countdown")
    } else {
        tr("loading.subtitle.loading")
    };
    if shell.desktop() {
        root.spawn(Node {
            top: Val::Px(shell.top),
            ..absolute(Val::Px(0.0), Val::Px(0.0), Val::Auto, Val::Auto)
        })
        .with_children(|row| {
            row.spawn(Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(space::S4),
                ..default()
            })
            .with_children(|column| {
                column
                    .spawn((
                        Node {
                            width: Val::Px(560.0),
                            height: Val::Px(44.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        KitImage::frame(Frame::Panel),
                    ))
                    .with_child((
                        Text::new(title),
                        theme::role_text(TextRole::Title),
                        TextColor(color::TEXT_GOLD),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                        Name::new("LoadingPhaseTitle"),
                    ));
                column.spawn((
                    Text::new(subtitle),
                    theme::role_text(TextRole::Caption),
                    TextColor(color::TEXT_SECONDARY),
                    TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    Name::new("LoadingPhaseSubtitle"),
                ));
            });
        });
        root.spawn(Node {
            top: Val::Px(shell.top),
            ..absolute(Val::Auto, Val::Px(shell.right), Val::Auto, Val::Auto)
        })
        .with_children(|slot| spawn_cancel(slot, shell.form));
    } else {
        root.spawn(Node {
            width: Val::Px(if countdown { 400.0 } else { 460.0 }),
            flex_direction: FlexDirection::Column,
            ..absolute(
                Val::Px(shell.left),
                Val::Auto,
                Val::Px(shell.top),
                Val::Auto,
            )
        })
        .with_children(|column| {
            column.spawn((
                Text::new(title),
                theme::role_text(TextRole::Title),
                TextColor(color::TEXT_GOLD),
                TextLayout::new(Justify::Left, LineBreak::NoWrap),
                Name::new("LoadingPhaseTitle"),
            ));
            column.spawn((
                Text::new(subtitle),
                theme::role_text(TextRole::Caption),
                TextColor(color::TEXT_SECONDARY),
                TextLayout::new(Justify::Left, LineBreak::NoWrap),
                Name::new("LoadingPhaseSubtitle"),
            ));
        });
        root.spawn(absolute(
            Val::Auto,
            Val::Px(shell.right),
            Val::Px(shell.top),
            Val::Auto,
        ))
        .with_children(|slot| spawn_cancel(slot, shell.form));
    }
}

/// Cancel: secondary, always enabled; never focused by the screen appearing
/// (a stray confirm must not cancel a shared countdown), the first
/// direction focuses it.
fn spawn_cancel(parent: &mut ChildSpawnerCommands, form: Form) {
    let button = spawn_button(
        parent,
        Node {
            width: Val::Px(size::BUTTON_MIN_WIDTH),
            ..button_node(ButtonSize::Regular, ButtonKind::Secondary, form)
        },
        tr("common.cancel"),
        TextStyle::new(TextRole::Button),
        ButtonKind::Secondary,
        None,
        LoadingAction::Cancel,
        TestId::new("LoadingCancel"),
        (),
    );
    parent
        .commands()
        .entity(button)
        .insert(FocusEntry::Deferred);
}

/// The prematch countdown ring: desktop 96 in the right gutter on the
/// crest line, phone 44 in the header gap.
fn spawn_countdown_ring(root: &mut ChildSpawnerCommands, shell: &Shell) -> Entity {
    let (node, ring_size) = if shell.desktop() {
        (
            Node {
                // x 1126 of 1280 (58 from the right), centred on y 356.
                margin: UiRect::top(Val::Px(-(size::TIMER_RING_MD / 2.0) - 4.0)),
                ..absolute(Val::Auto, Val::Px(58.0), Val::Percent(50.0), Val::Auto)
            },
            RingSize::Medium,
        )
    } else {
        (
            absolute(
                Val::Px(shell.left + PHONE_RING_X - space::SCREEN_MARGIN.phone),
                Val::Auto,
                Val::Px(shell.top),
                Val::Auto,
            ),
            RingSize::Small,
        )
    };
    let mut ring = Entity::PLACEHOLDER;
    root.spawn((node, Name::new("LoadingCountdownRing")))
        .with_children(|slot| {
            slot.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(space::S4),
                ..default()
            })
            .with_children(|column| {
                ring = status_ring(
                    column,
                    RingMode::Countdown {
                        progress: 1.0,
                        number: 3,
                    },
                    ring_size,
                );
                if ring_size == RingSize::Medium {
                    // Choices are locked (the draft's lock-count caption).
                    column.spawn(icon_node(Icon::NavLock, size::ICON_SM, color::TEXT_MUTED));
                }
            });
        });
    ring
}

/// The 5v5 roster body (loading-teams.md is P1: the rows stay as they were,
/// inside the new shell; the countdown ring keeps the right gutter free).
fn spawn_roster(
    root: &mut ChildSpawnerCommands,
    shell: &Shell,
    prematch: &shared::prematch::PrematchSnapshot,
    your_id: u64,
    thumbnails: &AvatarThumbnails,
    scroll: &DraftScrollMemory,
) {
    let compact = !shell.desktop();
    // Desktop: x 40–1092, y 104 to the footer (the right gutter holds the
    // countdown ring). Phone: 50 below the header line (y 62 at the
    // reference safe top 0), 64 above the bottom edge (the footer line).
    let (left, right, top, bottom) = if shell.desktop() {
        (shell.left, 188.0, 104.0, 112.0)
    } else {
        (shell.left, shell.right, shell.top + 50.0, 64.0)
    };
    root.spawn((
        Node {
            column_gap: Val::Px(12.0),
            ..absolute(Val::Px(left), Val::Px(right), Val::Px(top), Val::Px(bottom))
        },
        Name::new("LoadingRoster"),
    ))
    .with_children(|teams| {
        let own_team = prematch
            .players
            .iter()
            .find(|p| p.player_id == your_id)
            .map(|p| p.team);
        for (index, team) in [shared::map::Team::Green, shared::map::Team::Blue]
            .into_iter()
            .enumerate()
        {
            teams
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_width: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|column| {
                    column.spawn((
                        Text::new(if Some(team) == own_team {
                            tr("loading.your_team")
                        } else {
                            tr("loading.opponents")
                        }),
                        theme::role_text(TextRole::Eyebrow),
                        TextColor(color::TEXT_GOLD),
                    ));
                    let id = index as u8 + 2;
                    column
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                min_height: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                overflow: Overflow::scroll_y(),
                                row_gap: Val::Px(5.0),
                                ..default()
                            },
                            ScrollPosition(Vec2::new(0.0, *scroll.0.get(&id).unwrap_or(&0.0))),
                            draft::draft_pane(id),
                            Name::new(format!("LoadingTeam-{index}")),
                        ))
                        .with_children(|rows| {
                            for player in prematch.players.iter().filter(|p| p.team == team) {
                                draft::roster_row(rows, player, your_id, thumbnails, true, compact);
                            }
                        });
                });
        }
    });
}

/// Entities of the connecting body the sync writes.
#[derive(Default)]
struct ShellParts {
    ring: Option<Entity>,
    headline: Option<Entity>,
    headline_icon: Option<Entity>,
    detail: Option<Entity>,
    detail_icon: Option<Entity>,
    retry: Option<Entity>,
}

/// The connecting body (loading-shell.md): ring, headline, detail and
/// Retry in one centred block between the header and the footer.
fn spawn_connecting_body(root: &mut ChildSpawnerCommands, shell: &Shell) -> ShellParts {
    let desktop = shell.desktop();
    let mut parts = ShellParts::default();
    // Desktop: centred between the header (ends y 92) and the footer (starts
    // 104 above the bottom). Phone: from 52 below the header line (y 64 at
    // the reference safe top 0).
    let (top, bottom, width) = if desktop {
        (92.0, 104.0, 560.0)
    } else {
        (shell.top + 52.0, 0.0, 500.0)
    };
    let mut body = root.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: if desktop {
                JustifyContent::Center
            } else {
                JustifyContent::FlexStart
            },
            ..absolute(Val::Px(0.0), Val::Px(0.0), Val::Px(top), Val::Px(bottom))
        },
        Name::new("LoadingBody"),
    ));
    body.with_children(|block| {
        parts.ring = Some(status_ring(
            block,
            RingMode::Indeterminate,
            RingSize::Medium,
        ));
        let line = |gap: f32, height: f32| Node {
            width: Val::Px(width),
            max_width: Val::Percent(100.0),
            min_height: Val::Px(height),
            margin: UiRect::top(Val::Px(gap)),
            column_gap: Val::Px(space::S8),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        };
        block
            .spawn((
                line(
                    if desktop { space::S16 } else { space::S12 },
                    if desktop { 28.0 } else { 24.0 },
                ),
                Name::new("LoadingHeadline"),
            ))
            .with_children(|row| {
                parts.headline_icon = Some(
                    row.spawn((
                        icon_node(Icon::NavWifiOff, size::ICON_MD, color::STATE_WARNING),
                        Visibility::Hidden,
                    ))
                    .id(),
                );
                parts.headline = Some(
                    row.spawn((
                        Text::new(tr("loading.connecting")),
                        theme::role_text(TextRole::Heading),
                        TextColor(color::TEXT_GOLD),
                        TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    ))
                    .id(),
                );
            });
        block
            .spawn((
                Node {
                    align_items: AlignItems::FlexStart,
                    ..line(
                        if desktop { space::S16 } else { 10.0 },
                        if desktop { 36.0 } else { 34.0 },
                    )
                },
                Name::new("LoadingDetail"),
            ))
            .with_children(|row| {
                parts.detail_icon = Some(
                    row.spawn((
                        icon_node(Icon::NavAlertTriangle, size::ICON_SM, color::TEXT_DANGER),
                        Visibility::Hidden,
                    ))
                    .id(),
                );
                parts.detail = Some(
                    row.spawn((
                        Text::new(""),
                        theme::role_text(TextRole::Caption),
                        TextColor(color::TEXT_SECONDARY),
                        TextLayout::new(Justify::Center, LineBreak::WordBoundary),
                        Node {
                            max_width: Val::Px(width - size::ICON_SM - space::S8),
                            ..default()
                        },
                    ))
                    .id(),
                );
            });
        block
            .spawn(Node {
                margin: UiRect::top(Val::Px(if desktop { space::S16 } else { 6.0 })),
                ..default()
            })
            .with_children(|slot| {
                let retry = spawn_button(
                    slot,
                    Node {
                        width: Val::Px(size::BUTTON_MIN_WIDTH),
                        ..button_node(ButtonSize::Regular, ButtonKind::Secondary, shell.form)
                    },
                    tr("net.retry"),
                    TextStyle::new(TextRole::Button),
                    ButtonKind::Secondary,
                    Some(Icon::NavRefreshCw),
                    LoadingAction::Retry,
                    TestId::new("LoadingRetry"),
                    (),
                );
                // Hidden, not removed: the block never jumps.
                slot.commands()
                    .entity(retry)
                    .insert((Visibility::Hidden, FocusEntry::Preferred));
                parts.retry = Some(retry);
            });
    });
    parts
}

/// The footer (loading-teams.md): stage or ready count and the tip on the
/// first line, the asset line under them. Returns the count, the asset line
/// and its alert icon.
fn spawn_footer(
    root: &mut ChildSpawnerCommands,
    shell: &Shell,
    tip: &'static str,
    connecting: bool,
) -> (Entity, Entity, Entity) {
    let mut stage = Entity::PLACEHOLDER;
    let mut assets = Entity::PLACEHOLDER;
    let mut assets_icon = Entity::PLACEHOLDER;
    let count = |text: &str| {
        (
            Text::new(text),
            theme::role_text(TextRole::Label),
            TextColor(color::TEXT_GOLD),
            TextLayout::new(Justify::Left, LineBreak::NoWrap),
            Name::new("LoadingReadyCount"),
        )
    };
    let asset_line = |parent: &mut ChildSpawnerCommands,
                      justify: JustifyContent,
                      icon: &mut Entity,
                      text: &mut Entity| {
        parent
            .spawn(Node {
                column_gap: Val::Px(space::S8),
                align_items: AlignItems::Center,
                justify_content: justify,
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                overflow: Overflow::clip(),
                ..default()
            })
            .with_children(|line| {
                // Shown only for an unavailable Studio hero (then it takes
                // its room; otherwise it takes none).
                *icon = line
                    .spawn(icon_node(
                        Icon::NavAlertTriangle,
                        size::ICON_SM,
                        color::TEXT_DANGER,
                    ))
                    .insert(Node {
                        width: Val::Px(size::ICON_SM),
                        height: Val::Px(size::ICON_SM),
                        flex_shrink: 0.0,
                        display: Display::None,
                        ..default()
                    })
                    .id();
                *text = line
                    .spawn((
                        Text::new(tr("loading.assets.preparing")),
                        theme::role_text(TextRole::Caption),
                        TextColor(color::TEXT_SECONDARY),
                        TextLayout::new(Justify::Left, LineBreak::NoWrap),
                        Name::new("LoadingAssetStatus"),
                    ))
                    .id();
            });
    };
    let tip_line = |parent: &mut ChildSpawnerCommands, justify: JustifyContent| {
        parent
            .spawn((
                Node {
                    column_gap: Val::Px(space::S8),
                    align_items: AlignItems::Center,
                    justify_content: justify,
                    ..default()
                },
                Name::new("LoadingTip"),
            ))
            .with_children(|line| {
                line.spawn(icon_node(Icon::NavInfo, size::ICON_SM, color::TEXT_GOLD));
                line.spawn((
                    Text::new(tr(tip)),
                    theme::role_text(TextRole::Caption),
                    TextColor(color::TEXT_MUTED),
                    TextLayout::new(Justify::Right, LineBreak::NoWrap),
                ));
            });
    };
    if shell.desktop() {
        root.spawn((
            Node {
                width: Val::Px(904.0),
                max_width: Val::Percent(100.0),
                height: Val::Px(56.0),
                margin: UiRect::left(Val::Px(-452.0)),
                padding: UiRect::axes(Val::Px(space::S16), Val::Px(space::S8)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(space::S4),
                border: UiRect::all(Val::Px(border::HAIRLINE)),
                border_radius: BorderRadius::all(Val::Px(radius::MD)),
                ..absolute(
                    Val::Percent(50.0),
                    Val::Auto,
                    Val::Auto,
                    Val::Px(shell.bottom),
                )
            },
            BackgroundColor(theme::perceptual(color::SURFACE_GLASS_STRONG)),
            BorderColor::all(theme::perceptual(color::BORDER_HAIRLINE)),
            Name::new("LoadingFooter"),
        ))
        .with_children(|footer| {
            footer
                .spawn(Node {
                    height: Val::Px(20.0),
                    column_gap: Val::Px(space::S16),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    stage = row
                        .spawn((
                            count(""),
                            Node {
                                width: Val::Px(436.0),
                                flex_shrink: 0.0,
                                ..default()
                            },
                        ))
                        .id();
                    row.spawn(Node {
                        flex_grow: 1.0,
                        justify_content: JustifyContent::FlexEnd,
                        ..default()
                    })
                    .with_children(|slot| tip_line(slot, JustifyContent::FlexEnd));
                });
            footer
                .spawn(Node {
                    height: Val::Px(18.0),
                    ..default()
                })
                .with_children(|row| {
                    asset_line(
                        row,
                        JustifyContent::FlexStart,
                        &mut assets_icon,
                        &mut assets,
                    );
                });
        });
    } else {
        if connecting {
            // Kept on phone here (loading-teams phone drops it for the cards).
            root.spawn(Node {
                justify_content: JustifyContent::Center,
                ..absolute(
                    Val::Px(0.0),
                    Val::Px(0.0),
                    Val::Auto,
                    Val::Px(shell.bottom + 33.0),
                )
            })
            .with_children(|slot| tip_line(slot, JustifyContent::Center));
        }
        root.spawn((
            Node {
                height: Val::Px(20.0),
                column_gap: Val::Px(space::S16),
                align_items: AlignItems::Center,
                ..absolute(
                    Val::Px(shell.left + 3.0),
                    Val::Px(shell.right + 3.0),
                    Val::Auto,
                    Val::Px(shell.bottom),
                )
            },
            Name::new("LoadingFooter"),
        ))
        .with_children(|row| {
            stage = row
                .spawn((
                    count(""),
                    Node {
                        width: Val::Px(340.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ))
                .id();
            asset_line(row, JustifyContent::FlexEnd, &mut assets_icon, &mut assets);
        });
    }
    (stage, assets, assets_icon)
}

/// Per-frame text and state that never move a box: the status block, the
/// countdown arc, the ready or stage count and the asset line.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_loading(
    game: Res<GameStateSnapshot>,
    state: Res<DraftClient>,
    session: Option<Res<crate::net::ClientSession>>,
    time: Option<Res<Time>>,
    mut latch: ResMut<LoadingLatch>,
    roots: Query<&LoadingParts>,
    mut rings: Query<&mut RingMode>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
    mut visibility: Query<&mut Visibility>,
    mut nodes: Query<&mut Node>,
) {
    let Ok(parts) = roots.single() else {
        return;
    };
    let now = time.as_ref().map_or(0.0, |time| time.elapsed_secs());
    let mut set_text = |entity: Entity, text: &str, ink: Color| {
        if let Ok((mut current, mut color)) = texts.get_mut(entity) {
            if current.0 != text {
                current.0 = text.to_owned();
            }
            if color.0 != ink {
                color.0 = ink;
            }
        }
    };
    let mut show = |entity: Option<Entity>, shown: bool| {
        if let Some(mut current) = entity.and_then(|entity| visibility.get_mut(entity).ok()) {
            let next = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *current != next {
                *current = next;
            }
        }
    };
    let mut ring_mode = None;
    match &game.prematch {
        Some(prematch) => {
            // Countdown: the arc drains per frame between snapshots.
            if prematch.phase == PrematchPhase::Countdown {
                if latch
                    .countdown
                    .is_none_or(|(ms, _)| ms != prematch.remaining_ms)
                {
                    latch.countdown = Some((prematch.remaining_ms, now));
                }
                let (ms, seen) = latch.countdown.unwrap_or((prematch.remaining_ms, now));
                let left = (ms as f32 - (now - seen) * 1000.0).max(0.0);
                ring_mode = Some(RingMode::Countdown {
                    progress: left / shared::prematch::COUNTDOWN_MS as f32,
                    number: prematch.remaining_ms.div_ceil(1000).max(1),
                });
            }
            let ready = prematch.players.iter().filter(|p| p.loaded).count();
            let total = prematch.players.len();
            let count = if prematch.phase == PrematchPhase::Loading {
                trf(
                    "loading.ready_count_timed",
                    &[
                        ("ready", &ready),
                        ("total", &total),
                        ("seconds", &prematch.remaining_ms.div_ceil(1000)),
                    ],
                )
            } else {
                trf(
                    "loading.ready_count",
                    &[("ready", &ready), ("total", &total)],
                )
            };
            set_text(parts.stage, &count, color::TEXT_GOLD);
        }
        None => {
            match game.state {
                GameState::Starting { countdown_ms } => {
                    if latch.starting_total.is_none() {
                        latch.starting_total = Some(countdown_ms);
                    }
                }
                _ => latch.starting_total = None,
            }
            let link = session
                .as_deref()
                .map_or(LinkStatus::Connecting, crate::net::link_status);
            let status = shell_status(
                link,
                &game.state,
                game.rematch_in_secs,
                latch.starting_total,
                now - latch.entered_at > SLOW_AFTER_SECS,
            );
            ring_mode = Some(status.ring.clone());
            if let Some(headline) = parts.headline {
                set_text(headline, status.headline.text, status.headline.ink);
            }
            show(parts.headline_icon, status.headline.wifi);
            if let Some(detail) = parts.detail {
                let (text, ink) = status
                    .detail
                    .as_ref()
                    .map_or((String::new(), color::TEXT_SECONDARY), |detail| {
                        (detail.text.clone(), detail.ink)
                    });
                set_text(detail, &text, ink);
            }
            show(
                parts.detail_icon,
                status.detail.as_ref().is_some_and(|detail| detail.alert),
            );
            show(parts.retry, status.retry);
            set_text(
                parts.stage,
                status.stage.as_deref().unwrap_or_default(),
                color::TEXT_GOLD,
            );
        }
    }
    if let (Some(ring), Some(mode)) = (parts.ring, ring_mode)
        && let Ok(mut current) = rings.get_mut(ring)
        && *current != mode
    {
        *current = mode;
    }
    // The asset line: the step now, `unavailable` in danger with its icon,
    // `ready` in accent.
    let unavailable = state.local_assets == tr("loading.assets.unavailable");
    let (text, ink) = if state.local_assets.is_empty() {
        (tr("loading.assets.preparing"), color::TEXT_SECONDARY)
    } else if unavailable {
        (state.local_assets.as_str(), color::TEXT_DANGER)
    } else if state.local_assets == tr("loading.assets.ready") {
        (state.local_assets.as_str(), color::TEXT_ACCENT)
    } else {
        (state.local_assets.as_str(), color::TEXT_SECONDARY)
    };
    set_text(parts.assets, text, ink);
    if let Ok(mut node) = nodes.get_mut(parts.assets_icon) {
        let display = if unavailable {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_connecting_body_follows_the_status_priority() {
        let idle = GameState::Lobby;
        // 1–2, 4: failures show the track only, the reason and Retry.
        for link in [
            LinkStatus::Rejected(shared::protocol::JoinRejection::ProtocolMismatch),
            LinkStatus::Unconfirmed,
            LinkStatus::Disconnected,
        ] {
            let status = shell_status(link, &idle, None, None, false);
            assert_eq!(status.ring, RingMode::Error);
            assert_eq!(status.headline.text, "Connection problem");
            let detail = status.detail.unwrap();
            assert!(detail.alert && detail.ink == color::TEXT_DANGER);
            assert!(status.retry);
        }
        let disconnected = shell_status(LinkStatus::Disconnected, &idle, None, None, false);
        assert!(
            !disconnected
                .detail
                .unwrap()
                .text
                .contains("choose your team"),
            "R7.4: teams are assigned now"
        );
        // 3: reconnecting turns, warns, never offers Retry.
        let reconnecting = shell_status(
            LinkStatus::Reconnecting { attempt: 2 },
            &idle,
            None,
            None,
            true,
        );
        assert_eq!(reconnecting.ring, RingMode::Indeterminate);
        assert!(reconnecting.headline.wifi);
        assert_eq!(reconnecting.headline.ink, color::STATE_WARNING);
        assert!(reconnecting.detail.unwrap().text.contains("attempt 2"));
        assert!(!reconnecting.retry);
        // 5–6: connecting / joining.
        assert_eq!(
            shell_status(LinkStatus::Connecting, &idle, None, None, false).detail,
            None
        );
        let joining = shell_status(
            LinkStatus::Joining {
                attempt: 2,
                max: 15,
            },
            &idle,
            None,
            None,
            false,
        );
        assert!(joining.detail.unwrap().text.contains("attempt 2/15"));
        // 7: a server without a draft counts down, forms, or waits for a round.
        let starting = shell_status(
            LinkStatus::Connected,
            &GameState::Starting { countdown_ms: 1500 },
            None,
            Some(3000),
            false,
        );
        assert_eq!(
            starting.ring,
            RingMode::Countdown {
                progress: 0.5,
                number: 2
            }
        );
        let forming = shell_status(
            LinkStatus::Connected,
            &GameState::Forming {
                ready: 7,
                needed: 10,
            },
            None,
            None,
            false,
        );
        assert_eq!(forming.stage.as_deref(), Some("7 / 10 players ready"));
        let victory = shell_status(
            LinkStatus::Connected,
            &GameState::Victory {
                winner: shared::map::Team::Blue,
            },
            Some(6),
            None,
            false,
        );
        assert_eq!(victory.detail.unwrap().text, "Next round in 6s");
        // 8: slow, only where nothing else is said.
        let slow = shell_status(LinkStatus::Connected, &GameState::Running, None, None, true);
        assert_eq!(
            slow.detail.unwrap().text,
            "This is taking longer than usual. Cancel to return to the menu."
        );
    }

    #[test]
    fn tips_wrap_around() {
        assert_eq!(tip_for(0), TIPS[0]);
        assert_eq!(tip_for(TIPS.len()), TIPS[0]);
    }
    #[test]
    fn final_avatar_handle_is_required_even_when_a_fallback_scene_is_ready() {
        let mut assets = Assets::<Image>::default();
        let expected = assets.add(Image::default());
        let fallback = assets.add(Image::default());
        assert!(!matching_avatar_asset::<Image>(None, None));
        assert!(!matching_avatar_asset(Some(&expected), None));
        assert!(!matching_avatar_asset(Some(&expected), Some(&fallback)));
        assert!(matching_avatar_asset(Some(&expected), Some(&expected)));
    }

    #[test]
    fn intentional_cube_requires_actual_mesh_and_material_and_never_exempts_avatar_fallbacks() {
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mesh = Mesh3d(meshes.add(Cuboid::default()));
        let material = MeshMaterial3d(materials.add(StandardMaterial::default()));
        assert!(procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            Some("sdk-avatar"),
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Ipfs,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            None,
            &meshes,
            &materials
        ));
        materials.remove(material.0.id());
        assert!(!procedural_assets_ready(
            CharacterChoice::Cube,
            None,
            Some((&mesh, &material)),
            &meshes,
            &materials
        ));
    }

    #[test]
    fn cancel_leaves_once_and_a_disabled_cancel_does_nothing() {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        app.init_resource::<DraftClient>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<LoadingAction>()
            .add_systems(Update, loading_actions.after(crate::ui::UiSet::Dispatch));
        harness::spawn_ui(app.world_mut(), |header| {
            draft::action_button(
                header,
                "Cancel",
                LoadingAction::Cancel,
                "LoadingCancel",
                86.0,
                false,
                false,
            );
        });
        app.update();
        let leaves = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Messages<SessionUiCommand>>()
                .drain()
                .count()
        };
        harness::press(app.world_mut(), "LoadingCancel");
        app.update();
        assert_eq!(leaves(&mut app), 1);
        app.update();
        assert_eq!(leaves(&mut app), 0);
        harness::set_disabled(app.world_mut(), "LoadingCancel", true);
        harness::press(app.world_mut(), "LoadingCancel");
        app.update();
        assert_eq!(leaves(&mut app), 0);
    }
}
