//! Pre-join selection flow: hero class -> roster avatar -> team.
//!
//! The roster avatars (CC0 VRM models under `assets/avatars/`) are the primary
//! cosmetic path for the demo; the legacy SDK character stays in
//! [`TeamSelection::character`] as the fallback model when no roster avatar is
//! selected. Picking a team commits the join (class + avatar + team in one
//! packet).
// i18n-strict

use bevy::prelude::*;
use omoba_passport::avatars::avatar_roster;
use shared::HeroClass;
use std::collections::HashMap;

pub use crate::domain::Team;
use crate::frontend::AppScreen;
use crate::i18n::{Locale, Localized, data, locale_changed, tr};
use crate::net::{ClientConnectionState, ClientSession, NetworkCommand, SessionUiCommand};
use crate::sprite::{PlayerVisualMode, SpriteVisualAssets};
use crate::ui::kit_assets::Icon;
use crate::ui::theme::ButtonKind;
use crate::ui::theme::{self, TextStyle};
use crate::ui::tokens::{Metric, TextRole};
use crate::ui::widgets::ButtonStyle;
use crate::ui::{Activated, TestId, UiAction, UiActionAppExt, UiSet};
pub use shared::wire::CharacterChoice;

const TEAM_BUTTON_SIZE: f32 = 64.0;
const TEAM_BUTTON_GAP: f32 = 28.0;
const CLASS_BUTTON_WIDTH: f32 = 190.0;
const CLASS_BUTTON_HEIGHT: f32 = 60.0;
const CLASS_BUTTON_GAP: f32 = 12.0;
const AVATAR_BUTTON_WIDTH: f32 = 76.0;
const AVATAR_BUTTON_HEIGHT: f32 = 92.0;
const AVATAR_THUMBNAIL_SIZE: f32 = 56.0;
const AVATAR_GRID_GAP: f32 = 8.0;
const AVATAR_GRID_COLUMNS: usize = 8;
const SPRITE_GRID_MAX_COLUMNS: usize = 5;
const SPRITE_GRID_WIDTH_PERCENT: f32 = 92.0;
// Opaque: the match world must not be visible while a hero is being picked.
const TEAM_OVERLAY_COLOR: Color = crate::ui::theme::BACKDROP;
const SELECT_BUTTON_COLOR: Color = crate::ui::theme::TILE;
const SELECT_BUTTON_SELECTED_COLOR: Color = crate::ui::theme::TILE_SELECTED;

/// Every press on hero select. Buttons carry `UiAction<HeroSelectAction>`
/// and a `ButtonStyle` (tiles, `Link` for Ekza, `Team` for the lock-in); the
/// lock-in itself is decided by [`lock_in`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum HeroSelectAction {
    Back,
    Class(HeroClass),
    Avatar(String),
    Sprite(String),
    LockIn(Team),
    ConnectWallet,
    ConnectAccount,
    RefreshStudio,
}

#[derive(Resource)]
pub struct TeamSelection {
    pub team: Option<Team>,
    /// Legacy SDK character; used as the model when no roster avatar is picked.
    pub character: CharacterChoice,
    /// Selected hero class (gameplay kit).
    pub hero_class: HeroClass,
    /// Selected roster avatar slug (cosmetic; primary demo path).
    pub avatar: Option<String>,
    /// Selected 2D sprite character, preserved independently of the 3D avatar.
    pub sprite_character: String,
}

impl Default for TeamSelection {
    fn default() -> Self {
        Self {
            team: None,
            character: CharacterChoice::default(),
            hero_class: HeroClass::default(),
            // Preselect the first shipped avatar so a plain "click a team"
            // flow already exercises the roster path.
            avatar: crate::passport::selectable_avatars()
                .first()
                .map(|avatar| avatar.slug.clone()),
            sprite_character: shared::DEFAULT_SPRITE_CHARACTER_ID.to_owned(),
        }
    }
}

/// Thumbnail image handles for the avatar grid, keyed by slug.
#[derive(Resource, Default)]
pub struct AvatarThumbnails(pub HashMap<String, Handle<Image>>);

pub struct TeamSelectPlugin;

impl Plugin for TeamSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamSelection>()
            .init_resource::<AvatarThumbnails>()
            .add_ui_action::<HeroSelectAction>()
            // Thumbnails are loaded up front so the collection and the profile
            // card can draw avatars without opening the picker first.
            .add_systems(
                Startup,
                preload_avatar_thumbnails
                    .after(crate::persistence::load_persistent_client_settings)
                    .after(crate::sprite::load_sprite_visual_assets),
            )
            // The picker is a screen now: it exists only while the player is
            // choosing a hero, never behind or on top of a live match.
            .add_systems(OnEnter(AppScreen::HeroSelect), setup_team_select_ui)
            .add_systems(
                Update,
                (
                    // Before the network send: the join written by a lock-in is
                    // committed in the same frame, so the search screen never
                    // opens on an uncommitted join.
                    team_select_ui_system
                        .after(UiSet::Dispatch)
                        .before(crate::net::ClientNetPipeline::SendCommands),
                    sync_hero_panel,
                    sync_hero_select_status,
                    apply_phone_copy,
                )
                    .run_if(in_state(AppScreen::HeroSelect)),
            )
            .add_systems(Update, (attach_avatar_thumbnails, restore_picker_scroll))
            .add_systems(
                Update,
                wallet_connect_ui_system
                    .before(team_select_ui_system)
                    .run_if(in_state(AppScreen::HeroSelect)),
            )
            .add_systems(Update, (autojoin_from_env, sync_practice_picker));
        app.add_systems(
            PostUpdate,
            layout_spacious_picker
                .after(crate::mobile_ui::MobileUiLayout)
                .before(bevy::ui::UiSystems::Layout)
                .run_if(in_state(AppScreen::HeroSelect)),
        );
        app.add_systems(
            PostUpdate,
            sync_picker_scrollbar
                .after(bevy::ui::UiSystems::Layout)
                .run_if(in_state(AppScreen::HeroSelect)),
        );
    }
}

#[derive(Component)]
pub struct TeamSelectRoot;

/// Leaves the picker without committing a join.
#[derive(Component)]
pub struct HeroSelectBackButton;

/// Connection status inside the picker header.
#[derive(Component)]
struct HeroSelectStatus;

/// Admission failures have their own stable place above the primary action.
#[derive(Component)]
struct HeroSelectNotice;

#[derive(Component)]
struct JoinActionLabel;

/// A hero-select label with a shorter phone wording: [`apply_phone_copy`]
/// points its `Localized` key at `phone` on a phone and at `desktop`
/// elsewhere (the relabel system then rewrites the text).
#[derive(Component)]
struct PhoneCopy {
    desktop: &'static str,
    phone: &'static str,
}

impl PhoneCopy {
    /// The label (filled with the desktop wording) and its phone variant.
    fn label(desktop: &'static str, phone: &'static str) -> (Text, Localized, Self) {
        let (text, localized) = Localized::new(desktop).into_text();
        (text, localized, Self { desktop, phone })
    }
}

fn apply_phone_copy(
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut labels: Query<(Ref<PhoneCopy>, &mut Localized)>,
) {
    let phone = mobile.as_ref().is_some_and(|mobile| mobile.enabled);
    let platform_changed = mobile.as_ref().is_some_and(|mobile| mobile.is_changed());
    for (copy, mut localized) in &mut labels {
        if !platform_changed && !copy.is_added() {
            continue;
        }
        let key = if phone { copy.phone } else { copy.desktop };
        if localized.key != key {
            localized.key = key;
        }
    }
}

/// Dictionary keys of a class's name and tagline, for `Localized` labels.
fn class_keys(class: HeroClass) -> (&'static str, &'static str) {
    match class {
        HeroClass::Warrior => ("hero.warrior.name", "hero.warrior.tagline"),
        HeroClass::Mage => ("hero.mage.name", "hero.mage.tagline"),
        HeroClass::Ranger => ("hero.ranger.name", "hero.ranger.tagline"),
        HeroClass::Cleric => ("hero.cleric.name", "hero.cleric.tagline"),
        HeroClass::Warden => ("hero.warden.name", "hero.warden.tagline"),
        HeroClass::Dawnweaver => ("hero.dawnweaver.name", "hero.dawnweaver.tagline"),
        HeroClass::Wildspark => ("hero.wildspark.name", "hero.wildspark.tagline"),
        HeroClass::Cinderforge => ("hero.cinderforge.name", "hero.cinderforge.tagline"),
        HeroClass::Edgeweaver => ("hero.edgeweaver.name", "hero.edgeweaver.tagline"),
        HeroClass::Stormfist => ("hero.stormfist.name", "hero.stormfist.tagline"),
        HeroClass::Veilstalker => ("hero.veilstalker.name", "hero.veilstalker.tagline"),
        HeroClass::Emberveil => ("hero.emberveil.name", "hero.emberveil.tagline"),
        HeroClass::Orbitwright => ("hero.orbitwright.name", "hero.orbitwright.tagline"),
        HeroClass::Riftshot => ("hero.riftshot.name", "hero.riftshot.tagline"),
        HeroClass::Chainkeeper => ("hero.chainkeeper.name", "hero.chainkeeper.tagline"),
        HeroClass::Frostguard => ("hero.frostguard.name", "hero.frostguard.tagline"),
    }
}

/// `Warrior · Melee bruiser…` in the active language.
fn class_line(class: HeroClass) -> String {
    format!("{} · {}", data::hero_name(class), data::hero_tagline(class))
}

#[derive(Component)]
struct TeamSelectButton;

#[derive(Component)]
struct ClassSelectButton;

#[derive(Component)]
struct AvatarSelectButton;

/// Placeholder node inside an avatar button that receives the thumbnail image
/// once [`AvatarThumbnails`] has a handle for the slug.
#[derive(Component)]
struct AvatarThumbnailSlot {
    slug: String,
}

#[derive(Component)]
struct ModelAvatarGrid;

#[derive(Component)]
struct PickerScrollbar;
#[derive(Component)]
struct PickerScrollThumb;

#[derive(Component)]
struct WalletStatusText;

#[derive(Component)]
struct WalletConnectButton;

#[derive(Component)]
struct AccountStatusText;

#[derive(Component)]
struct AccountConnectButton;

const WALLET_BUTTON_COLOR: Color = crate::ui::theme::LINK;

#[derive(Component)]
struct SpriteAvatarGrid;

#[derive(Component)]
struct SpriteSelectButton;

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpriteGridLayout {
    columns: usize,
    rows: usize,
    max_width: f32,
}

fn sprite_grid_layout(character_count: usize) -> SpriteGridLayout {
    let columns = character_count.clamp(1, SPRITE_GRID_MAX_COLUMNS);
    SpriteGridLayout {
        columns,
        rows: character_count.div_ceil(columns),
        max_width: columns as f32 * AVATAR_BUTTON_WIDTH
            + columns.saturating_sub(1) as f32 * AVATAR_GRID_GAP,
    }
}

fn update_sprite_selection(selection: &mut TeamSelection, requested: &str) -> bool {
    let normalized = shared::normalize_sprite_character_id(Some(requested));
    if crate::sprite_roster::sprite_character_definition(normalized)
        .is_some_and(|entry| entry.render_fallback.is_some())
    {
        return false;
    }
    if selection.sprite_character == normalized {
        false
    } else {
        selection.sprite_character = normalized.to_owned();
        true
    }
}

fn setup_team_select_ui(
    mut selection: ResMut<TeamSelection>,
    visual_mode: Res<PlayerVisualMode>,
    sprite_assets: Res<SpriteVisualAssets>,
    asset_server: Res<AssetServer>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    mut preview: ResMut<crate::frontend::preview::AvatarPreview>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    // Entering the picker always means "not committed yet", even when the
    // player backed out of a queue with a team already chosen.
    selection.team = None;
    load_avatar_thumbnails(&asset_server, *visual_mode, &mut thumbnails);
    if let Some(slug) = selection.avatar.as_deref() {
        preview.show_portrait(slug);
    }
    let image = preview.image.clone();
    spawn_team_select_ui(
        &mut commands,
        &selection,
        *visual_mode,
        &sprite_assets,
        image,
        is_compact(&windows),
    );
}

/// A short window gets the compact picker layout.
fn is_compact(windows: &Query<&Window, With<bevy::window::PrimaryWindow>>) -> bool {
    windows
        .single()
        .is_ok_and(|window| window.resolution.height() < COMPACT_HEIGHT)
}

/// Avatar thumbnails are used outside the picker (profile card, collection),
/// so they are requested once at startup.
fn preload_avatar_thumbnails(
    visual_mode: Res<PlayerVisualMode>,
    asset_server: Res<AssetServer>,
    mut thumbnails: ResMut<AvatarThumbnails>,
) {
    load_avatar_thumbnails(&asset_server, *visual_mode, &mut thumbnails);
}

fn load_avatar_thumbnails(
    asset_server: &AssetServer,
    visual_mode: PlayerVisualMode,
    thumbnails: &mut AvatarThumbnails,
) {
    if visual_mode != PlayerVisualMode::Models3d {
        return;
    }
    for avatar in avatar_roster()
        .iter()
        .cloned()
        .chain(omoba_passport::store::catalogue_definitions())
    {
        if let Some(thumbnail) = crate::passport::thumbnail_asset_path(&avatar) {
            thumbnails
                .0
                .insert(avatar.slug.clone(), asset_server.load(thumbnail));
        }
    }
}

/// Wallet pairing from the menu: start it, mirror its progress into the status
/// line, and rebuild the picker when purchased avatars become selectable
/// (wallet approved, or the store catalogue arrived after the menu was built).
fn wallet_connect_ui_system(
    mut commands: Commands,
    selection: Res<TeamSelection>,
    visual_mode: Res<PlayerVisualMode>,
    sprite_assets: Res<SpriteVisualAssets>,
    asset_server: Res<AssetServer>,
    mut thumbnails: ResMut<AvatarThumbnails>,
    mut status: Query<&mut Text, (With<WalletStatusText>, Without<AccountStatusText>)>,
    mut account_status: Query<&mut Text, (With<AccountStatusText>, Without<WalletStatusText>)>,
    preview: Res<crate::frontend::preview::AvatarPreview>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut listed_revision: Local<Option<u64>>,
    mut listed_account: Local<Option<bool>>,
    grid_scroll: Query<&ScrollPosition, With<ModelAvatarGrid>>,
    locale: Option<Res<crate::i18n::Locale>>,
    mut listed_locale: Local<Option<u32>>,
) {
    if selection.team.is_some() || overlay_query.is_empty() {
        return;
    }
    let just_connected = crate::passport::poll_wallet();
    let line = crate::passport::wallet_status_line();
    for mut text in &mut status {
        if text.0 != line {
            text.0.clone_from(&line);
        }
    }
    let account_just_connected = crate::passport::poll_account();
    let account_line = crate::passport::account_status_line();
    for mut text in &mut account_status {
        if text.0 != account_line {
            text.0.clone_from(&account_line);
        }
    }
    let catalogue = crate::passport::avatar_catalogue();
    let connected = crate::passport::account_connected();
    let stale = picker_catalogue_changed(
        *listed_revision,
        catalogue.revision,
        *listed_account,
        connected,
    );
    *listed_account = Some(connected);
    *listed_revision = Some(catalogue.revision);
    // The Studio hint and the avatar source suffixes are plain text built here.
    let generation = locale.as_ref().map(|locale| locale.generation());
    let relabel = listed_locale.is_some() && *listed_locale != generation;
    *listed_locale = generation;
    if just_connected || account_just_connected || stale || relabel {
        if let Ok(scroll) = grid_scroll.single() {
            commands.insert_resource(PickerScrollRestore(scroll.y));
        }
        for overlay in &overlay_query {
            commands
                .entity(overlay)
                .despawn_related::<Children>()
                .despawn();
        }
        load_avatar_thumbnails(&asset_server, *visual_mode, &mut thumbnails);
        spawn_team_select_ui(
            &mut commands,
            &selection,
            *visual_mode,
            &sprite_assets,
            preview.image.clone(),
            is_compact(&windows),
        );
    }
}

fn picker_catalogue_changed(
    previous_revision: Option<u64>,
    revision: u64,
    previous_account: Option<bool>,
    connected: bool,
) -> bool {
    previous_revision.is_some_and(|previous| previous != revision)
        || previous_account.is_some_and(|previous| previous != connected)
}

#[derive(Resource)]
struct PickerScrollRestore(f32);
fn restore_picker_scroll(
    mut commands: Commands,
    pending: Option<Res<PickerScrollRestore>>,
    mut grids: Query<&mut ScrollPosition, With<ModelAvatarGrid>>,
) {
    let Some(pending) = pending else {
        return;
    };
    if let Ok(mut scroll) = grids.single_mut() {
        scroll.y = pending.0;
        commands.remove_resource::<PickerScrollRestore>();
    }
}

/// Window height under which the picker switches to its compact layout.
const COMPACT_HEIGHT: f32 = 700.0;

pub fn spawn_team_select_ui(
    commands: &mut Commands,
    selection: &TeamSelection,
    visual_mode: PlayerVisualMode,
    sprite_assets: &SpriteVisualAssets,
    preview_image: Handle<Image>,
    compact: bool,
) {
    let sprite_grid = sprite_grid_layout(crate::sprite_roster::sprite_character_roster().len());
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(if compact { 5.0 } else { 10.0 }),
                // Room on the right for the live hero panel (3D mode only).
                // Proportional, so a small window keeps both columns readable.
                padding: UiRect::axes(Val::Px(18.0), Val::Px(18.0)).with_right(
                    if visual_mode == PlayerVisualMode::Models3d {
                        Val::Percent(HERO_PANEL_PERCENT + 2.0)
                    } else {
                        Val::Px(18.0)
                    },
                ),
                ..default()
            },
            BackgroundColor(TEAM_OVERLAY_COLOR),
            ZIndex(crate::ui::theme::SCREEN_Z),
            // Leaving hero select always takes the picker with it, whether the
            // player locked in, went back, or the session pulled the screen.
            bevy::state::state_scoped::DespawnOnExit(AppScreen::HeroSelect),
            TeamSelectRoot,
            Name::new("TeamSelectOverlay"),
        ))
        .with_children(|parent| {
            crate::ui::living_background::spawn(
                parent,
                crate::ui::living_background::LivingScene::Arena,
                crate::ui::living_background::LivingBands {
                    header: Some(120.0),
                    footer: None,
                },
                crate::ui::theme::Form::of(compact),
            );
            parent.spawn((
                crate::ui::widgets::surfaces::ornament_frame(),
                Name::new("HeroSelectOrnament"),
            ));
            parent
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::FlexStart,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(16.0),
                        ..default()
                    },
                    Name::new("HeroSelectHeader"),
                ))
                .with_children(|header| {
                    header
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(16.0), Val::Px(7.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(SELECT_BUTTON_COLOR),
                            ButtonStyle::new(ButtonKind::Secondary),
                            UiAction(HeroSelectAction::Back),
                            HeroSelectBackButton,
                            TestId::new("HeroSelectBack"),
                        ))
                        .with_children(|button| {
                            button.spawn(crate::ui::widgets::icon_node(
                                Icon::NavChevronLeft,
                                20.0,
                                theme::GOLD,
                            ));
                            button.spawn((
                                Localized::new("team.back").into_text(),
                                theme::role_text(TextRole::Button),
                                TextColor(theme::IVORY),
                                Name::new("HeroSelectBackLabel"),
                            ));
                        });
                    header.spawn((
                        Localized::new("team.title").into_text(),
                        TextFont {
                            font_size: 26.0,
                            ..default()
                        },
                        TextStyle::new(TextRole::Title),
                        TextColor(theme::GOLD),
                        Name::new("HeroSelectTitle"),
                    ));
                    header.spawn((
                        Text::new(String::new()),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextStyle::keep_case(TextRole::Caption),
                        TextColor(crate::ui::theme::MUTED),
                        Node {
                            margin: UiRect::left(Val::Auto),
                            max_width: Val::Percent(45.0),
                            ..default()
                        },
                        HeroSelectStatus,
                        Name::new("HeroSelectStatus"),
                    ));
                });
            spawn_section_title(
                parent,
                PhoneCopy::label("team.section.class", "team.section.class_phone"),
                "ClassSelectTitle",
            );

            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        max_width: Val::Percent(100.0),
                        column_gap: Val::Px(CLASS_BUTTON_GAP),
                        row_gap: Val::Px(CLASS_BUTTON_GAP),
                        ..default()
                    },
                    Name::new("ClassButtonsRow"),
                    crate::ui::ScrollArea::wheel(48.0)
                        .touch_drag(8.0)
                        .keyed(0x434c4153),
                ))
                .with_children(|row| {
                    for class in HeroClass::ALL {
                        spawn_class_button(row, class, selection.hero_class == class);
                    }
                });

            spawn_section_title(
                parent,
                PhoneCopy::label("team.section.avatar", "team.section.avatar_phone"),
                "AvatarSelectTitle",
            );
            parent.spawn((
                PhoneCopy::label("team.scroll_hint", "team.scroll_hint_phone"),
                TextFont {
                    font_size: 12.5,
                    ..default()
                },
                TextColor(crate::ui::theme::MUTED),
                Name::new("RendererStatus"),
            ));

            parent
                .spawn((
                    Node {
                        display: if visual_mode == PlayerVisualMode::Models3d {
                            Display::Flex
                        } else {
                            Display::None
                        },
                        width: Val::Percent(92.0),
                        max_width: Val::Px(
                            AVATAR_GRID_COLUMNS as f32 * (AVATAR_BUTTON_WIDTH + AVATAR_GRID_GAP),
                        ),
                        // Whole rows only: a half-cut row reads as a broken
                        // layout rather than as something to scroll. A short
                        // window gets one row so the lock-in stays visible.
                        max_height: Val::Px(
                            if compact { 1.0 } else { 2.0 }
                                * (AVATAR_BUTTON_HEIGHT + AVATAR_GRID_GAP)
                                + AVATAR_GRID_GAP,
                        ),
                        overflow: Overflow::scroll_y(),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        column_gap: Val::Px(AVATAR_GRID_GAP),
                        row_gap: Val::Px(AVATAR_GRID_GAP),
                        ..default()
                    },
                    if visual_mode == PlayerVisualMode::Models3d {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    },
                    roster_scroll(),
                    ModelAvatarGrid,
                    BackgroundColor(crate::ui::theme::perceptual(
                        crate::ui::tokens::color::SURFACE_1,
                    )),
                    Name::new("AvatarGrid"),
                ))
                .with_children(|grid| {
                    if visual_mode != PlayerVisualMode::Models3d {
                        return;
                    }
                    let catalogue = crate::passport::avatar_catalogue();
                    spawn_avatar_group_label(grid, "team.group.default", "DefaultAvatarsLabel");
                    for entry in catalogue
                        .entries
                        .iter()
                        .filter(|e| e.source == crate::passport::AvatarCatalogueSource::Default)
                    {
                        let avatar = &entry.avatar;
                        spawn_avatar_button(
                            grid,
                            &avatar.slug,
                            &avatar.display_name,
                            selection.avatar.as_deref() == Some(avatar.slug.as_str()),
                        );
                    }
                    spawn_avatar_group_label(grid, "team.group.studio", "StudioAvatarsLabel");
                    spawn_avatar_group_hint(
                        grid,
                        crate::i18n::data::catalogue_status(&catalogue.status),
                    );
                    grid.spawn((
                        Button,
                        Node {
                            min_width: Val::Px(120.0),
                            min_height: Val::Px(44.0),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(WALLET_BUTTON_COLOR),
                        ButtonStyle::new(ButtonKind::Link),
                        UiAction(HeroSelectAction::RefreshStudio),
                        TestId::new("PickerRefreshStudio"),
                    ))
                    .with_children(|button| {
                        button.spawn((
                            Localized::new("team.refresh_studio").into_text(),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                    });
                    for entry in catalogue
                        .entries
                        .iter()
                        .filter(|e| e.source != crate::passport::AvatarCatalogueSource::Default)
                    {
                        let avatar = &entry.avatar;
                        spawn_avatar_button(
                            grid,
                            &avatar.slug,
                            &avatar.display_name,
                            selection.avatar.as_deref() == Some(avatar.slug.as_str()),
                        );
                    }
                    spawn_ekza_row(grid);
                });

            parent
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(3.0),
                        border_radius: BorderRadius::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(theme::EDGE),
                    PickerScrollbar,
                    Name::new("PickerScrollbar"),
                ))
                .with_children(|track| {
                    track.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Percent(100.0),
                            height: Val::Percent(40.0),
                            border_radius: BorderRadius::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(theme::GOLD),
                        PickerScrollThumb,
                    ));
                });

            parent
                .spawn((
                    Node {
                        display: if visual_mode == PlayerVisualMode::Sprite2d {
                            Display::Flex
                        } else {
                            Display::None
                        },
                        max_height: Val::Vh(32.0),
                        overflow: Overflow::scroll_y(),
                        flex_shrink: 0.0,
                        width: Val::Percent(SPRITE_GRID_WIDTH_PERCENT),
                        max_width: Val::Px(sprite_grid.max_width),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        column_gap: Val::Px(AVATAR_GRID_GAP),
                        row_gap: Val::Px(AVATAR_GRID_GAP),
                        ..default()
                    },
                    if visual_mode == PlayerVisualMode::Sprite2d {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    },
                    roster_scroll(),
                    SpriteAvatarGrid,
                    Name::new("SpriteCharacterGrid"),
                ))
                .with_children(|grid| {
                    for (index, character) in crate::sprite_roster::sprite_character_roster()
                        .iter()
                        .enumerate()
                        .filter(|_| visual_mode == PlayerVisualMode::Sprite2d)
                    {
                        spawn_sprite_button(
                            grid,
                            character,
                            sprite_assets.portrait(index),
                            selection.sprite_character == character.id,
                        );
                    }
                });

            spawn_section_title(
                parent,
                PhoneCopy::label(
                    if crate::sandbox::requested() {
                        "team.section.combat_test"
                    } else {
                        "team.section.match"
                    },
                    "team.section.match_phone",
                ),
                "TeamSelectTitle",
            );

            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(TEAM_BUTTON_GAP),
                        ..default()
                    },
                    Name::new("TeamButtonsRow"),
                ))
                .with_children(|row| {
                    spawn_team_button(row, Team::Green, "FindMatchButton");
                });

            parent.spawn((
                PhoneCopy::label("team.hint", "team.hint_phone"),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(0.78, 0.80, 0.86, 1.0)),
                TextStyle::new(TextRole::Caption),
                Name::new("TeamSelectHint"),
            ));
            parent.spawn((
                Text::new(""),
                theme::role_text(TextRole::Caption),
                TextColor(theme::GOLD),
                HeroSelectNotice,
                Name::new("HeroSelectNotice"),
            ));
            if visual_mode == PlayerVisualMode::Models3d {
                spawn_hero_panel(parent, selection, preview_image.clone());
            }
        })
        .id();
    if visual_mode == PlayerVisualMode::Sprite2d {
        let (image, layout) = sprite_assets.ui_frame();
        commands.entity(root).insert(ImageNode::from_atlas_image(
            image,
            TextureAtlas { layout, index: 0 },
        ));
    }
}

/// Share of the window the live hero panel takes on the right of the picker.
const HERO_PANEL_PERCENT: f32 = 27.0;
/// Upper bound for that panel on a wide window.
const HERO_PANEL_MAX_WIDTH: f32 = 320.0;

/// Marks the live hero panel's avatar name.
#[derive(Component)]
struct HeroPanelAvatarName;

/// Marks the live hero panel's class name.
#[derive(Component)]
struct HeroPanelClassName;

/// One ability row in the live hero panel, by slot index.
#[derive(Component)]
struct HeroPanelAbility(usize);

/// Shows what the player is about to lock in: the avatar in 3D, the class and
/// the kit that class brings. Everything here is read-only; selection stays
/// with the grids on the left.
fn spawn_hero_panel(
    parent: &mut ChildSpawnerCommands,
    selection: &TeamSelection,
    preview_image: Handle<Image>,
) {
    let class = selection.hero_class;
    let avatar_name = crate::passport::avatar_display_name(selection.avatar.as_deref());
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                bottom: Val::Px(12.0),
                width: Val::Percent(HERO_PANEL_PERCENT),
                max_width: Val::Px(HERO_PANEL_MAX_WIDTH),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(14.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(crate::ui::theme::PANEL_OPAQUE),
            BorderColor::all(crate::ui::theme::PANEL_EDGE),
            Name::new("HeroSelectPanel"),
        ))
        .with_children(|panel| {
            panel.spawn((
                ImageNode::new(preview_image),
                Node {
                    width: Val::Percent(92.0),
                    // The preview texture is portrait; keep its shape at any size.
                    aspect_ratio: Some(0.742),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                Name::new("HeroSelectPreview"),
                crate::frontend::preview::InteractivePreview,
            ));
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(4.0),
                        padding: UiRect::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(theme::PANEL_OPAQUE),
                    Name::new("HeroSelectStageCaption"),
                ))
                .with_children(|caption| {
                    caption.spawn((
                        Text::new(avatar_name.to_owned()),
                        TextFont::default(),
                        TextStyle::keep_case(TextRole::Heading),
                        TextColor(theme::IVORY),
                        TextLayout::new_with_justify(Justify::Center),
                        HeroPanelAvatarName,
                        Name::new("HeroSelectAvatarName"),
                    ));
                    caption.spawn((
                        Text::new(class_line(class)),
                        theme::role_text(TextRole::Caption),
                        TextColor(theme::GOLD),
                        TextLayout::new_with_justify(Justify::Center),
                        HeroPanelClassName,
                        Name::new("HeroSelectClassName"),
                    ));
                });
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::top(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(theme::EDGE),
                    Name::new("HeroSelectAbilityList"),
                ))
                .with_children(|kit| {
                    for (index, ability) in class.abilities().iter().enumerate() {
                        kit.spawn((
                            Text::new(format!(
                                "{}  {}",
                                ability_key(index),
                                data::ability_name(ability)
                            )),
                            theme::role_text(TextRole::Caption),
                            TextColor(theme::MUTED),
                            HeroPanelAbility(index),
                            Name::new(format!("HeroSelectAbility-{index}")),
                        ));
                    }
                });
        });
}

/// Hotkey label for an ability slot, matching the in-match hotbar.
fn ability_key(index: usize) -> &'static str {
    ["Q", "W", "E", "R"].get(index).copied().unwrap_or("?")
}

/// Keeps the live hero panel in step with the grids, and points the shared 3D
/// preview at the avatar the player currently has selected.
fn sync_hero_select_status(
    session: Res<crate::net::ClientSession>,
    notice: Option<Res<crate::frontend::JoinNotice>>,
    mut status: Query<
        (&mut Text, &mut TextColor),
        (With<HeroSelectStatus>, Without<HeroSelectNotice>),
    >,
    mut notices: Query<&mut Text, (With<HeroSelectNotice>, Without<HeroSelectStatus>)>,
) {
    let (line, color) = {
        if session.is_offline() {
            (tr("home.offline_hint").to_owned(), crate::ui::theme::GOLD)
        } else {
            let (line, _) = crate::frontend::home::connection_line(&session);
            (line, crate::ui::theme::MUTED)
        }
    };
    let notice_line = notice
        .as_ref()
        .and_then(|notice| notice.0.as_deref())
        .unwrap_or("");
    for mut text in &mut notices {
        if text.0 != notice_line {
            text.0 = notice_line.to_owned();
        }
    }
    for (mut text, mut text_color) in &mut status {
        if text.0 != line {
            text.0.clone_from(&line);
        }
        if text_color.0 != color {
            text_color.0 = color;
        }
    }
}

fn sync_hero_panel(
    selection: Res<TeamSelection>,
    locale: Option<Res<Locale>>,
    mut preview: ResMut<crate::frontend::preview::AvatarPreview>,
    mut names: Query<&mut Text, (With<HeroPanelAvatarName>, Without<HeroPanelClassName>)>,
    mut classes: Query<&mut Text, (With<HeroPanelClassName>, Without<HeroPanelAvatarName>)>,
    mut abilities: Query<
        (&HeroPanelAbility, &mut Text),
        (Without<HeroPanelAvatarName>, Without<HeroPanelClassName>),
    >,
) {
    if !selection.is_changed() && !locale_changed(&locale) {
        return;
    }
    if selection.is_changed()
        && let Some(slug) = selection.avatar.as_deref()
    {
        preview.show_portrait(slug);
    }
    let avatar_name = crate::passport::avatar_display_name(selection.avatar.as_deref());
    for mut text in &mut names {
        if text.0 != avatar_name {
            text.0 = avatar_name.to_owned();
        }
    }
    let class = selection.hero_class;
    let class_line = class_line(class);
    for mut text in &mut classes {
        if text.0 != class_line {
            text.0.clone_from(&class_line);
        }
    }
    let kit = class.abilities();
    for (slot, mut text) in &mut abilities {
        let Some(ability) = kit.get(slot.0) else {
            continue;
        };
        let line = format!("{}  {}", ability_key(slot.0), data::ability_name(ability));
        if text.0 != line {
            text.0 = line;
        }
    }
}

/// Wallet and account connection in one strip under the grids. Both are
/// optional: the shipped roster plays without either.
fn spawn_ekza_row(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                max_width: Val::Percent(100.0),
                column_gap: Val::Px(12.0),
                row_gap: Val::Px(6.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(crate::ui::theme::PANEL_OPAQUE),
            BorderColor::all(crate::ui::theme::PANEL_EDGE),
            Name::new("EkzaConnectRow"),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(crate::passport::wallet_status_line()),
                TextFont {
                    font_size: 12.5,
                    ..default()
                },
                TextStyle::keep_case(TextRole::Caption),
                TextColor(crate::ui::theme::MUTED),
                WalletStatusText,
                Name::new("PassportStatus"),
            ));
            if !crate::passport::is_connected() {
                spawn_connect_button(row, "team.connect_wallet", ConnectTarget::Wallet);
            }
            row.spawn((
                Text::new(crate::passport::account_status_line()),
                TextFont {
                    font_size: 12.5,
                    ..default()
                },
                TextStyle::keep_case(TextRole::Caption),
                TextColor(crate::ui::theme::MUTED),
                AccountStatusText,
                Name::new("EkzaAccountStatus"),
            ));
            spawn_connect_button(
                row,
                if crate::passport::account_connected() {
                    "team.sign_out"
                } else {
                    "team.connect_account"
                },
                ConnectTarget::Account,
            );
        });
}

/// Which Ekza connection a strip button starts.
enum ConnectTarget {
    Wallet,
    Account,
}

fn spawn_connect_button(
    row: &mut ChildSpawnerCommands,
    label: &'static str,
    target: ConnectTarget,
) {
    let mut button = row.spawn((
        Button,
        Node {
            padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(WALLET_BUTTON_COLOR),
        ButtonStyle::new(ButtonKind::Link),
    ));
    match target {
        ConnectTarget::Wallet => {
            button.insert((
                WalletConnectButton,
                UiAction(HeroSelectAction::ConnectWallet),
                TestId::new("WalletConnectButton"),
            ));
        }
        ConnectTarget::Account => {
            button.insert((
                AccountConnectButton,
                UiAction(HeroSelectAction::ConnectAccount),
                TestId::new("AccountConnectButton"),
            ));
        }
    }
    button.with_children(|button| {
        button.spawn((
            Localized::new(label).into_text(),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextStyle::new(TextRole::Button).sized(Metric::new(14.0, 12.0)),
            TextColor::WHITE,
        ));
    });
}

/// The hero roster grids: 48 px per wheel notch, 180 px per PageUp/PageDown.
fn roster_scroll() -> crate::ui::ScrollArea {
    crate::ui::ScrollArea::wheel(48.0)
        .page_keys(180.0)
        .touch_drag(8.0)
        .keyed(0x4845524f)
}

fn sync_picker_scrollbar(
    grid: Query<(&ComputedNode, &ScrollPosition), With<ModelAvatarGrid>>,
    mut tracks: Query<&mut Visibility, With<PickerScrollbar>>,
    mut thumbs: Query<&mut Node, With<PickerScrollThumb>>,
) {
    let Ok((computed, scroll)) = grid.single() else {
        return;
    };
    let viewport = computed.size().y * computed.inverse_scale_factor();
    let content = computed.content_size().y * computed.inverse_scale_factor();
    let overflow = (content - viewport).max(0.0);
    for mut visibility in &mut tracks {
        *visibility = if overflow > 1.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    let ratio = (viewport / content.max(1.0)).clamp(0.08, 1.0);
    for mut node in &mut thumbs {
        node.height = Val::Percent(ratio * 100.0);
        node.top =
            Val::Percent((scroll.y / overflow.max(1.0)).clamp(0.0, 1.0) * (1.0 - ratio) * 100.0);
    }
}

fn spawn_sprite_button(
    grid: &mut ChildSpawnerCommands,
    character: &crate::sprite_roster::SpriteCharacterDefinition,
    portrait: (Handle<Image>, Handle<TextureAtlasLayout>, usize),
    selected: bool,
) {
    let draft = character.render_fallback.is_some();
    let mut tile = grid.spawn((
        Node {
            width: Val::Px(AVATAR_BUTTON_WIDTH),
            height: Val::Px(AVATAR_BUTTON_HEIGHT + if draft { 24.0 } else { 0.0 }),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(if selected {
            SELECT_BUTTON_SELECTED_COLOR
        } else {
            SELECT_BUTTON_COLOR
        }),
        TestId::new(format!("SpriteButton-{}", character.id)),
    ));
    if !draft {
        tile.insert((
            Button,
            ButtonStyle {
                kind: ButtonKind::Tile,
                selected,
            },
            UiAction(HeroSelectAction::Sprite(character.id.clone())),
            SpriteSelectButton,
        ));
    }
    tile.with_children(|button| {
        let mut entity = button.spawn((
            Node {
                width: Val::Px(AVATAR_THUMBNAIL_SIZE),
                height: Val::Px(AVATAR_THUMBNAIL_SIZE),
                ..default()
            },
            BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.9)),
        ));
        entity.insert(ImageNode::from_atlas_image(
            portrait.0,
            TextureAtlas {
                layout: portrait.1,
                index: portrait.2,
            },
        ));
        let style = (
            TextFont {
                font_size: 9.5,
                ..default()
            },
            TextColor(Color::srgba(0.86, 0.88, 0.92, 1.0)),
        );
        if draft {
            let fallback =
                crate::sprite_roster::sprite_character_render_definition(Some(&character.id))
                    .map_or(tr("team.sprite.default"), |entry| {
                        entry.display_name.as_str()
                    });
            button.spawn((
                Localized::with_args(
                    "team.sprite.pending",
                    [("name", &character.display_name), ("fallback", &fallback)],
                )
                .into_text(),
                style,
            ));
        } else {
            button.spawn((Text::new(character.display_name.clone()), style));
        }
    });
}

fn spawn_section_title(parent: &mut ChildSpawnerCommands, title: impl Bundle, name: &str) {
    parent.spawn((
        title,
        theme::role_text(TextRole::Eyebrow),
        TextColor(theme::GOLD),
        Name::new(name.to_owned()),
    ));
}

/// Full-width row inside the wrapping avatar grid, so each group starts on its
/// own line while the grid keeps a single scroll area.
fn spawn_avatar_group_label(grid: &mut ChildSpawnerCommands, title: &'static str, name: &str) {
    grid.spawn((
        Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::FlexStart,
            padding: UiRect::top(Val::Px(8.0)),
            ..default()
        },
        Name::new(name.to_owned()),
    ))
    .with_children(|row| {
        row.spawn((
            Localized::new(title).into_text(),
            theme::role_text(TextRole::Eyebrow),
            TextColor(theme::GOLD),
        ));
    });
}

fn spawn_avatar_group_hint(grid: &mut ChildSpawnerCommands, hint: impl crate::i18n::UiLabel) {
    grid.spawn((
        Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::FlexStart,
            ..default()
        },
        Name::new("PurchasedAvatarsHint"),
    ))
    .with_children(|row| {
        row.spawn((
            hint.into_text(),
            theme::role_text(TextRole::Caption),
            TextColor(theme::MUTED),
        ));
    });
}

fn spawn_class_button(row: &mut ChildSpawnerCommands, class: HeroClass, selected: bool) {
    row.spawn((
        Button,
        Node {
            width: Val::Px(CLASS_BUTTON_WIDTH),
            height: Val::Px(CLASS_BUTTON_HEIGHT),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(if selected {
            SELECT_BUTTON_SELECTED_COLOR
        } else {
            SELECT_BUTTON_COLOR
        }),
        ButtonStyle {
            kind: ButtonKind::Tile,
            selected,
        },
        UiAction(HeroSelectAction::Class(class)),
        ClassSelectButton,
        TestId::new(format!("ClassButton-{}", class.id())),
    ))
    .with_children(|button| {
        let (name, _) = class_keys(class);
        button.spawn(crate::ui::widgets::icon_node(
            crate::ui::widgets::game::class_icon(class),
            24.0,
            theme::GOLD,
        ));
        button.spawn((
            Localized::new(name).into_text(),
            theme::role_text(TextRole::Button),
            TextColor(theme::IVORY),
        ));
    });
}

fn spawn_avatar_button(
    grid: &mut ChildSpawnerCommands,
    slug: &str,
    display_name: &str,
    selected: bool,
) {
    grid.spawn((
        Button,
        Node {
            width: Val::Px(AVATAR_BUTTON_WIDTH),
            height: Val::Px(AVATAR_BUTTON_HEIGHT),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(if selected {
            SELECT_BUTTON_SELECTED_COLOR
        } else {
            SELECT_BUTTON_COLOR
        }),
        ButtonStyle {
            kind: ButtonKind::Tile,
            selected,
        },
        UiAction(HeroSelectAction::Avatar(slug.to_owned())),
        AvatarSelectButton,
        TestId::new(format!("AvatarButton-{slug}")),
    ))
    .with_children(|button| {
        button
            .spawn((
                Node {
                    width: Val::Px(AVATAR_THUMBNAIL_SIZE),
                    height: Val::Px(AVATAR_THUMBNAIL_SIZE),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(crate::ui::theme::PANEL),
                AvatarThumbnailSlot {
                    slug: slug.to_owned(),
                },
                Name::new("PickerAvatarThumbnail"),
            ))
            .with_children(|portrait| {
                if omoba_passport::avatars::avatar_definition(slug)
                    .and_then(crate::passport::thumbnail_asset_path)
                    .is_none()
                {
                    portrait.spawn(crate::ui::widgets::icon_node(
                        Icon::ClassWarrior,
                        32.0,
                        theme::GOLD,
                    ));
                }
            });
        button.spawn((
            Text::new(display_name),
            TextFont::default(),
            TextStyle::keep_case(TextRole::Caption).sized(Metric::new(13.0, 11.0)),
            TextColor(theme::IVORY),
            TextLayout::new_with_justify(Justify::Center),
            Node {
                max_width: Val::Percent(100.0),
                ..default()
            },
        ));
    });
}

/// Attaches the thumbnail image to avatar buttons once (avatars without a
/// shipped thumbnail keep the dark placeholder tile + name).
fn attach_avatar_thumbnails(
    mut commands: Commands,
    thumbnails: Res<AvatarThumbnails>,
    slots: Query<(Entity, &AvatarThumbnailSlot), Without<ImageNode>>,
) {
    for (entity, slot) in &slots {
        if let Some(handle) = thumbnails.0.get(&slot.slug) {
            // try_insert: the overlay (and these slots) can be despawned by a
            // join in the same frame; inserting into a dead entity must not panic.
            commands
                .entity(entity)
                .try_insert(ImageNode::new(handle.clone()));
        }
    }
}

fn spawn_team_button(row: &mut ChildSpawnerCommands, team: Team, name: &str) {
    row.spawn((
        Button,
        Node {
            width: Val::Px(230.0),
            height: Val::Px(TEAM_BUTTON_SIZE),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(crate::ui::theme::button_idle_color(
            ButtonKind::Primary,
            false,
        )),
        BorderColor::all(crate::ui::theme::EDGE),
        ButtonStyle::new(ButtonKind::Primary),
        UiAction(HeroSelectAction::LockIn(team)),
        TeamSelectButton,
        TestId::new(name.to_owned()),
    ))
    .with_children(|button| {
        button.spawn((
            JoinActionLabel,
            Text::new(tr(if crate::sandbox::requested() {
                "team.join.combat_test"
            } else {
                "team.join.find_match"
            })),
            theme::role_text(TextRole::ButtonLg),
            TextColor(theme::IVORY),
        ));
    });
}

/// Applies hero-select presses. Runs after `UiSet::Dispatch` and before the
/// network send, so a lock-in's join leaves in the same frame.
fn team_select_ui_system(
    mut commands: Commands,
    client_session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    mut activated: MessageReader<Activated<HeroSelectAction>>,
    mut styles: Query<(&UiAction<HeroSelectAction>, &mut ButtonStyle)>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut screen: Option<ResMut<NextState<AppScreen>>>,
    mut notice: Option<ResMut<crate::frontend::JoinNotice>>,
    mut command_writer: MessageWriter<NetworkCommand>,
    mut session_ui_writer: MessageWriter<SessionUiCommand>,
) {
    if selection.team.is_some() {
        // Locked in: a press queued before the lock must not fire later.
        activated.clear();
        return;
    }
    let mut selection_changed = false;
    for Activated { action, .. } in activated.read() {
        match action {
            HeroSelectAction::Back => {
                if let Some(screen) = screen.as_deref_mut() {
                    if client_session.is_offline() {
                        session_ui_writer.write(SessionUiCommand::LeaveMatch);
                    }
                    screen.set(AppScreen::Home);
                    return;
                }
            }
            HeroSelectAction::Class(class) => {
                if client_session.is_offline() && class.is_standard() {
                    if let Some(notice) = notice.as_deref_mut() {
                        notice.0 = Some(tr("error.join.offline_kit_unsupported").into());
                    }
                    continue;
                }
                selection.hero_class = *class;
                selection_changed = true;
            }
            HeroSelectAction::Avatar(slug) => {
                if client_session.is_offline()
                    && !omoba_passport::avatars::avatar_roster()
                        .iter()
                        .any(|a| a.slug == *slug && a.passport.is_none())
                {
                    if let Some(notice) = notice.as_deref_mut() {
                        notice.0 = Some(tr("team.notice.offline_avatar").into());
                    }
                    continue;
                }
                selection.avatar = Some(slug.clone());
                selection_changed = true;
            }
            HeroSelectAction::Sprite(id) => {
                selection_changed |= update_sprite_selection(&mut selection, id);
            }
            HeroSelectAction::ConnectWallet => crate::passport::connect(),
            HeroSelectAction::ConnectAccount => crate::passport::connect_account(),
            HeroSelectAction::RefreshStudio => crate::passport::refresh_avatar_catalogue(),
            HeroSelectAction::LockIn(team) => {
                let (join, next_screen) = match lock_in(
                    *team,
                    &selection,
                    client_session.join_in_flight(),
                    client_session.state(),
                    crate::sandbox::requested(),
                    client_session.is_offline(),
                ) {
                    LockIn::Ignore => continue,
                    LockIn::Reconnect => {
                        info!(
                            "[omoba:cli] event=join_deferred reason=disconnected \
                             msg=\"Reconnecting to server; try finding a match once connected.\""
                        );
                        session_ui_writer.write(SessionUiCommand::Retry);
                        continue;
                    }
                    LockIn::Join { command, screen } => (command, screen),
                };
                selection.team = Some(*team);
                // A new attempt: the previous failure is no longer the story.
                if let Some(notice) = notice.as_deref_mut() {
                    notice.0 = None;
                }
                info!(
                    "[omoba:cli] event=join_request team={:?} class={} avatar={:?} character={:?}",
                    team,
                    selection.hero_class.id(),
                    selection.avatar,
                    selection.character
                );
                command_writer.write(join);
                if let Ok(overlay) = overlay_query.single() {
                    commands
                        .entity(overlay)
                        .despawn_related::<Children>()
                        .despawn();
                }
                // The hero is locked: matchmaking owns the screen from here.
                if let Some(screen) = screen.as_deref_mut() {
                    screen.set(next_screen);
                }
                return;
            }
        }
    }
    if selection_changed {
        for (action, mut style) in &mut styles {
            let selected = match &action.0 {
                HeroSelectAction::Class(class) => *class == selection.hero_class,
                HeroSelectAction::Avatar(slug) => selection.avatar.as_deref() == Some(slug),
                HeroSelectAction::Sprite(id) => selection.sprite_character == *id,
                _ => continue,
            };
            if style.selected != selected {
                style.selected = selected;
            }
        }
    }
}

/// What pressing a team's lock-in button on hero select does.
#[derive(Debug)]
enum LockIn {
    /// A join is already on its way: the press is ignored.
    Ignore,
    /// The transport is dead: a join written now would be lost, so the
    /// select screen stays and the connection is retried instead.
    Reconnect,
    /// Send `command` and hand the screen to `screen`.
    Join {
        command: NetworkCommand,
        screen: AppScreen,
    },
}

/// The lock-in decision, without the ECS: the sandbox and offline practice
/// join the arena directly, everyone else joins the prematch draft; the
/// sandbox goes straight into the match, everyone else to the search screen.
fn lock_in(
    team: Team,
    selection: &TeamSelection,
    join_in_flight: bool,
    connection: ClientConnectionState,
    sandbox: bool,
    offline: bool,
) -> LockIn {
    if join_in_flight {
        return LockIn::Ignore;
    }
    if connection == ClientConnectionState::Disconnected {
        return LockIn::Reconnect;
    }
    let sprite_character = Some(selection.sprite_character.clone());
    let command = if sandbox || offline {
        NetworkCommand::Join {
            team,
            character: selection.character,
            hero_class: selection.hero_class,
            avatar: selection.avatar.clone(),
            sprite_character,
        }
    } else {
        NetworkCommand::JoinPrematch {
            character: selection.character,
            hero_class: selection.hero_class,
            avatar: selection.avatar.clone(),
            sprite_character,
        }
    };
    LockIn::Join {
        command,
        screen: if sandbox {
            AppScreen::InMatch
        } else {
            AppScreen::Searching
        },
    }
}

/// Debug/automation hook: `OMOBA_AUTOJOIN="<class>:<avatar-slug|->:<team>[:<sprite-id>]"`
/// (e.g. `mage:agnes:green`) joins immediately without UI interaction. Used by
/// the headless evidence runs; ignored when unset.
fn autojoin_from_env(
    mut done: Local<bool>,
    mut commands: Commands,
    client_session: Res<ClientSession>,
    mut selection: ResMut<TeamSelection>,
    overlay_query: Query<Entity, With<TeamSelectRoot>>,
    mut command_writer: MessageWriter<NetworkCommand>,
) {
    if *done {
        return;
    }
    let Ok(raw) = std::env::var("OMOBA_AUTOJOIN") else {
        *done = true;
        return;
    };
    if selection.team.is_some() || client_session.join_in_flight() {
        *done = true;
        return;
    }
    *done = true;

    let mut parts = raw.split(':');
    let class = HeroClass::from_id(parts.next().unwrap_or("")).unwrap_or_default();
    let avatar = omoba_passport::avatars::normalize_avatar_slug(parts.next()).map(str::to_owned);
    let team = match parts.next() {
        Some("blue") => Team::Blue,
        _ => Team::Green,
    };
    let sprite_character = shared::normalize_sprite_character_id(parts.next()).to_owned();

    selection.hero_class = class;
    selection.avatar = avatar.clone();
    selection.team = Some(team);
    selection.sprite_character.clone_from(&sprite_character);
    info!(
        "[omoba:cli] event=autojoin team={:?} class={} avatar={:?}",
        team,
        class.id(),
        avatar
    );
    command_writer.write(NetworkCommand::Join {
        team,
        character: selection.character,
        hero_class: class,
        avatar,
        sprite_character: Some(sprite_character),
    });
    for overlay in &overlay_query {
        commands
            .entity(overlay)
            .despawn_related::<Children>()
            .despawn();
    }
}

fn sync_practice_picker(
    session: Res<ClientSession>,
    mut labels: Query<&mut Text, With<JoinActionLabel>>,
) {
    let label = tr(if session.is_offline() {
        "team.join.practice"
    } else if crate::sandbox::requested() {
        "team.join.combat_test"
    } else {
        "team.join.find_match"
    });
    for mut text in &mut labels {
        if text.0 != label {
            text.0 = label.into();
        }
    }
}

/// Tablet touch input keeps the spacious picker, including its live hero.
/// Online account controls are not useful in the bundled offline arena.
fn layout_spacious_picker(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    scale: Res<UiScale>,
    session: Res<ClientSession>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    mut nodes: Query<(
        Option<&Name>,
        Option<&TestId>,
        Option<&ClassSelectButton>,
        Option<&UiAction<HeroSelectAction>>,
        &mut Node,
    )>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let w = window.width() / scale.0.max(0.1);
    let h = window.height() / scale.0.max(0.1);
    let phone = window.height() < 600.0;
    let safe = mobile
        .as_ref()
        .filter(|m| m.enabled)
        .map(|m| [m.safe.left, m.safe.right, m.safe.top, m.safe.bottom])
        .unwrap_or([0.0; 4]);
    let layout = PickerLayout::new(w, h, phone, safe.map(|v| v / scale.0.max(0.1)));
    let PickerLayout {
        inset,
        top,
        bottom,
        left_w,
        stage_x,
        stage_w,
        class_y,
        grid_x,
        grid_y,
        grid_w,
        grid_h,
        stage_y,
        stage_h,
        action_y,
        ..
    } = layout;
    let absolute = |node: &mut Node, x, y, width, height| {
        node.position_type = PositionType::Absolute;
        node.left = Val::Px(x);
        node.top = Val::Px(y);
        node.right = Val::Auto;
        node.bottom = Val::Auto;
        node.width = Val::Px(width);
        node.height = Val::Px(height);
        node.max_width = Val::Auto;
        node.max_height = Val::Auto;
        node.min_width = Val::Px(0.0);
        node.min_height = Val::Px(0.0);
    };
    for (name, id, class, action, mut node) in &mut nodes {
        let key = crate::ui::test_id::node_key(name, id).unwrap_or_default();
        if session.is_offline() && (matches!(key, "EkzaConnectRow" | "StudioAvatarsLabel" | "PurchasedAvatarsHint" | "PickerRefreshStudio")
            || action.is_some_and(|a| matches!(&a.0, HeroSelectAction::Avatar(slug) if omoba_passport::avatars::avatar_definition(slug).is_none()))) {
            node.display = Display::None; continue;
        }
        if class.is_some() {
            node.width = Val::Px(if phone { left_w } else { (left_w - 24.0) / 4.0 });
            node.height = Val::Px(if phone {
                ((bottom - class_y - 24.0) / 5.0).clamp(40.0, 48.0)
            } else {
                64.0
            });
            node.flex_direction = if phone {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };
            node.column_gap = Val::Px(8.0);
            node.flex_shrink = 0.0;
        }
        if action.is_some_and(|a| matches!(a.0, HeroSelectAction::Avatar(_))) {
            node.width = Val::Px(layout.tile_width());
            node.height = Val::Px(layout.tile_width() + if phone { 28.0 } else { 32.0 });
            node.padding = UiRect::all(Val::Px(5.0));
            node.justify_content = JustifyContent::FlexStart;
        }
        match key {
            "PickerAvatarThumbnail" => {
                node.width = Val::Px(layout.tile_width() - 10.0);
                node.height = Val::Px(layout.tile_width() - 10.0);
                node.flex_shrink = 0.0;
            }
            "TeamSelectOverlay" => {
                node.padding = UiRect::ZERO;
            }
            "HeroSelectHeader" => {
                absolute(&mut node, inset, top, w - inset - layout.right, 48.0);
                node.flex_direction = FlexDirection::Row;
                node.column_gap = Val::Px(if phone { 12.0 } else { 20.0 });
            }
            "HeroSelectTitle" => {
                node.display = Display::Flex;
                node.max_width = Val::Percent(60.0);
                node.margin.left = Val::Px(if phone { 0.0 } else { 16.0 });
            }
            "HeroSelectStatus" => {
                node.display = if phone { Display::None } else { Display::Flex };
                node.max_width = Val::Px(stage_w);
            }
            "HeroSelectBack" => {
                node.width = Val::Px(if phone { 44.0 } else { 128.0 });
                node.height = Val::Px(44.0);
                node.flex_shrink = 0.0;
                node.padding = UiRect::ZERO;
                node.column_gap = Val::Px(8.0);
            }
            "HeroSelectBackLabel" => {
                node.display = if phone { Display::None } else { Display::Flex }
            }
            "HeroSelectOrnament" => {
                node.display = if phone { Display::None } else { Display::Flex }
            }
            "ClassSelectTitle" => absolute(&mut node, inset, class_y - 24.0, left_w, 20.0),
            "ClassButtonsRow" => {
                absolute(
                    &mut node,
                    inset,
                    class_y,
                    left_w,
                    if phone {
                        bottom - class_y
                    } else {
                        layout.class_height()
                    },
                );
                node.flex_direction = if phone {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                node.justify_content = JustifyContent::FlexStart;
                node.column_gap = Val::Px(8.0);
                node.row_gap = Val::Px(6.0);
                node.flex_wrap = if phone {
                    FlexWrap::NoWrap
                } else {
                    FlexWrap::Wrap
                };
                node.overflow = Overflow::scroll_y();
            }
            "AvatarSelectTitle" => absolute(&mut node, grid_x, grid_y - 24.0, grid_w, 20.0),
            "RendererStatus" => node.display = Display::None,
            "PickerScrollbar" => absolute(
                &mut node,
                grid_x + grid_w - 4.0,
                grid_y + 8.0,
                3.0,
                grid_h - 16.0,
            ),
            "AvatarGrid" => {
                absolute(&mut node, grid_x, grid_y, grid_w, grid_h);
                node.padding = UiRect::all(Val::Px(if phone { 4.0 } else { 12.0 }));
                node.row_gap = Val::Px(8.0);
                node.column_gap = Val::Px(8.0);
                node.align_content = AlignContent::FlexStart;
                node.justify_content = JustifyContent::FlexStart;
                node.border_radius = BorderRadius::all(Val::Px(12.0));
            }
            "HeroSelectPanel" => {
                node.display = Display::Flex;
                absolute(&mut node, stage_x, stage_y, stage_w, stage_h);
                node.padding = UiRect::ZERO;
                node.row_gap = Val::Px(0.0);
                node.overflow = Overflow::clip();
            }
            "HeroSelectPreview" => {
                node.width = Val::Auto;
                node.height = Val::Px(if phone {
                    (stage_h - 62.0).max(100.0)
                } else {
                    (stage_h - 204.0).max(156.0)
                });
                node.flex_shrink = 0.0;
            }
            "HeroSelectStageCaption" => {
                node.padding = UiRect::all(Val::Px(if phone { 6.0 } else { 12.0 }));
                node.position_type = if phone {
                    PositionType::Absolute
                } else {
                    PositionType::Relative
                };
                node.bottom = if phone { Val::Px(0.0) } else { Val::Auto };
                node.max_height = Val::Px(if phone { 72.0 } else { 92.0 });
                node.flex_shrink = 0.0;
            }
            "HeroSelectAvatarName" | "HeroSelectClassName" => node.max_width = Val::Percent(100.0),
            "HeroSelectAbilityList" => {
                node.display = if phone { Display::None } else { Display::Flex }
            }
            "HeroSelectNotice" => absolute(
                &mut node,
                stage_x,
                action_y - if phone { 46.0 } else { 52.0 },
                stage_w,
                40.0,
            ),
            "TeamSelectTitle" => node.display = Display::None,
            "TeamButtonsRow" => {
                absolute(&mut node, stage_x, action_y, stage_w, 56.0);
                node.justify_content = JustifyContent::Center;
            }
            "FindMatchButton" => {
                node.width = Val::Percent(100.0);
                node.height = Val::Px(56.0);
            }
            "TeamSelectHint" => {
                node.display = if phone { Display::None } else { Display::Flex };
                absolute(&mut node, inset, top + 52.0, left_w, 22.0);
            }
            "EkzaConnectRow" => {
                node.display = if phone { Display::None } else { Display::Flex };
                node.width = Val::Percent(100.0);
                node.max_width = Val::Percent(100.0);
                node.justify_content = JustifyContent::FlexStart;
            }
            _ => {}
        }
    }
}

/// One set of geometry keeps the roster, live preview and action disjoint on
/// wide screens and on the compact, three-column phone picker.
#[derive(Clone, Copy, Debug)]
struct PickerLayout {
    phone: bool,
    inset: f32,
    right: f32,
    top: f32,
    bottom: f32,
    left_w: f32,
    class_y: f32,
    grid_x: f32,
    grid_y: f32,
    grid_w: f32,
    grid_h: f32,
    stage_x: f32,
    stage_y: f32,
    stage_w: f32,
    stage_h: f32,
    action_y: f32,
}

impl PickerLayout {
    fn new(w: f32, h: f32, phone: bool, safe: [f32; 4]) -> Self {
        let inset = if phone {
            safe[0] + 12.0
        } else {
            safe[0].max(48.0)
        };
        let right = if phone {
            safe[1] + 12.0
        } else {
            safe[1].max(48.0)
        };
        let top = if phone {
            safe[2] + 8.0
        } else {
            safe[2].max(40.0)
        };
        let bottom = h - if phone {
            safe[3] + 12.0
        } else {
            safe[3].max(48.0)
        };
        let available = w - inset - right;
        let stage_w = if phone {
            available * 0.35
        } else {
            (available * 0.29).min(352.0)
        };
        let stage_x = w - right - stage_w;
        let left_w = if phone {
            available * 0.22
        } else {
            stage_x - inset - 24.0
        };
        let class_y = top + if phone { 72.0 } else { 104.0 };
        let grid_x = if phone { inset + left_w + 12.0 } else { inset };
        let class_rows = HeroClass::ALL.len().div_ceil(4).min(2) as f32;
        let grid_y = if phone {
            class_y
        } else {
            class_y + class_rows * 64.0 + (class_rows - 1.0) * 6.0 + 40.0
        };
        let grid_w = if phone {
            stage_x - grid_x - 12.0
        } else {
            left_w
        };
        let stage_y = class_y - 24.0;
        let action_y = bottom - 56.0;
        let stage_h = action_y - stage_y - if phone { 54.0 } else { 64.0 };
        Self {
            phone,
            inset,
            right,
            top,
            bottom,
            left_w,
            class_y,
            grid_x,
            grid_y,
            grid_w,
            grid_h: bottom - grid_y,
            stage_x,
            stage_y,
            stage_w,
            stage_h,
            action_y,
        }
    }

    fn tile_width(self) -> f32 {
        let columns = if self.phone { 3.0 } else { 8.0 };
        let padding = if self.phone { 8.0 } else { 24.0 };
        ((self.grid_w - padding - (columns - 1.0) * 8.0) / columns).floor()
    }

    fn class_height(self) -> f32 {
        self.grid_y - self.class_y - 40.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::SPRITE_CHARACTER_IDS;

    #[test]
    fn hero_picker_keeps_preview_notice_and_action_inside_each_viewport() {
        for (w, h, phone, safe) in [
            (1280.0, 720.0, false, [0.0; 4]),
            (1180.0, 820.0, false, [32.0, 32.0, 12.0, 20.0]),
            (844.0, 390.0, true, [32.0, 32.0, 12.0, 20.0]),
        ] {
            let p = PickerLayout::new(w, h, phone, safe);
            assert!(p.grid_x + p.grid_w + 10.0 <= p.stage_x);
            assert!(p.stage_h >= 140.0, "{p:?}");
            assert!(p.stage_y + p.stage_h <= p.action_y - 46.0);
            assert!(p.action_y + 56.0 <= h - safe[3]);
            assert!(p.stage_x + p.stage_w <= w - safe[1]);
            assert!(p.grid_y + p.grid_h <= h - safe[3]);
            assert!(p.tile_width() >= 68.0, "{p:?}");
            if phone {
                assert!(p.inset + p.left_w + 10.0 <= p.grid_x);
            }
        }
    }

    #[test]
    fn picker_admission_notice_and_connection_queries_are_disjoint() {
        let mut world = World::new();
        let mut system = IntoSystem::into_system(sync_hero_select_status);
        system.initialize(&mut world);
    }

    #[test]
    fn picker_roster_supports_swipe_without_turning_taps_into_scroll() {
        let scroll = roster_scroll();
        assert!(scroll.wheel.is_some());
        assert!(scroll.drag.is_some());
        assert_eq!(scroll.key, Some(0x4845524f));
    }

    /// O25: the lock-in decision for every connection/mode combination.
    #[test]
    fn lock_in_ignores_in_flight_reconnects_dead_and_routes_by_mode() {
        let selection = TeamSelection {
            hero_class: HeroClass::Mage,
            avatar: Some("agnes".into()),
            sprite_character: "ronin".into(),
            ..default()
        };
        let connected = ClientConnectionState::Connected;
        for (sandbox, offline) in [(false, false), (true, false), (false, true)] {
            assert!(matches!(
                lock_in(Team::Blue, &selection, true, connected, sandbox, offline),
                LockIn::Ignore
            ));
            assert!(
                matches!(
                    lock_in(
                        Team::Blue,
                        &selection,
                        true,
                        ClientConnectionState::Disconnected,
                        sandbox,
                        offline
                    ),
                    LockIn::Ignore
                ),
                "an in-flight join wins over a reconnect"
            );
            assert!(matches!(
                lock_in(
                    Team::Blue,
                    &selection,
                    false,
                    ClientConnectionState::Disconnected,
                    sandbox,
                    offline
                ),
                LockIn::Reconnect
            ));
        }

        let LockIn::Join { command, screen } =
            lock_in(Team::Blue, &selection, false, connected, false, false)
        else {
            panic!("online lock-in joins");
        };
        assert_eq!(screen, AppScreen::Searching);
        assert!(matches!(
            command,
            NetworkCommand::JoinPrematch { hero_class: HeroClass::Mage, avatar: Some(ref a), sprite_character: Some(ref s), .. }
                if a == "agnes" && s == "ronin"
        ));

        for (sandbox, expected) in [(true, AppScreen::InMatch), (false, AppScreen::Searching)] {
            let LockIn::Join { command, screen } =
                lock_in(Team::Blue, &selection, false, connected, sandbox, !sandbox)
            else {
                panic!("sandbox and offline lock-ins join");
            };
            assert_eq!(screen, expected);
            assert!(matches!(
                command,
                NetworkCommand::Join {
                    team: Team::Blue,
                    hero_class: HeroClass::Mage,
                    ..
                }
            ));
        }
    }

    #[test]
    fn picker_refreshes_account_action_even_with_an_unchanged_empty_library() {
        assert!(!picker_catalogue_changed(Some(3), 3, Some(false), false));
        assert!(picker_catalogue_changed(Some(3), 3, Some(false), true));
        assert!(picker_catalogue_changed(Some(3), 3, Some(true), false));
        assert!(picker_catalogue_changed(Some(3), 4, Some(true), true));
    }

    /// Bevy checks query conflicts when a system is initialized, not when it compiles.
    /// The picker system holds two `&mut Text` queries (wallet and account
    /// status); they must stay disjoint or the game panics on start.
    #[test]
    fn picker_system_queries_are_disjoint() {
        let mut world = World::new();
        let mut system = IntoSystem::into_system(wallet_connect_ui_system);
        system.initialize(&mut world);
    }

    #[test]
    fn ten_sprite_buttons_use_a_bounded_two_row_grid() {
        let layout = sprite_grid_layout(SPRITE_CHARACTER_IDS.len());
        assert_eq!(
            layout,
            SpriteGridLayout {
                columns: 5,
                rows: 2,
                max_width: 412.0,
            }
        );
        assert!(layout.max_width < 480.0);
    }

    #[test]
    fn smaller_rosters_do_not_reserve_empty_columns() {
        assert_eq!(sprite_grid_layout(3).columns, 3);
        assert_eq!(sprite_grid_layout(3).rows, 1);
        assert_eq!(sprite_grid_layout(0).rows, 0);
    }

    #[test]
    fn shipped_sprite_buttons_select_their_ids_and_drafts_do_not_replace_selection() {
        let mut selection = TeamSelection::default();
        for id in &SPRITE_CHARACTER_IDS[..9] {
            let changed = update_sprite_selection(&mut selection, id);
            assert_eq!(selection.sprite_character, *id);
            assert_eq!(changed, *id != shared::DEFAULT_SPRITE_CHARACTER_ID);
        }
        assert!(!update_sprite_selection(
            &mut selection,
            "orchard-comet-centaur"
        ));
        assert_eq!(selection.sprite_character, "aurora-magnet-ram");
        assert!(update_sprite_selection(&mut selection, "unknown"));
        assert_eq!(
            selection.sprite_character,
            shared::DEFAULT_SPRITE_CHARACTER_ID
        );
    }

    #[test]
    fn model_join_ui_excludes_inactive_layout_and_scrolls_all_shipped_choices() {
        let mut app = App::new();
        app.add_message::<bevy::input::mouse::MouseWheel>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_systems(Startup, |mut commands: Commands| {
                spawn_team_select_ui(
                    &mut commands,
                    &TeamSelection::default(),
                    PlayerVisualMode::Models3d,
                    &SpriteVisualAssets::default(),
                    Handle::default(),
                    false,
                )
            })
            .add_systems(Update, crate::ui::scroll::scroll_areas);
        app.world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow));
        app.update();
        let mut model = app
            .world_mut()
            .query_filtered::<(Entity, &Node), With<ModelAvatarGrid>>();
        let (grid, node) = model.single(app.world()).unwrap();
        assert_eq!(node.display, Display::Flex);
        // Two whole avatar rows on a normal window; a short window gets one.
        assert_eq!(
            node.max_height,
            Val::Px(2.0 * (AVATAR_BUTTON_HEIGHT + AVATAR_GRID_GAP) + AVATAR_GRID_GAP)
        );
        assert_eq!(node.overflow.y, OverflowAxis::Scroll);
        let mut inactive = app
            .world_mut()
            .query_filtered::<&Node, With<SpriteAvatarGrid>>();
        assert_eq!(inactive.single(app.world()).unwrap().display, Display::None);
        let mut choices = app.world_mut().query::<&AvatarSelectButton>();
        assert_eq!(choices.iter(app.world()).count(), avatar_roster().len());
        // Defaults precede the persistent Studio section, even without a connection.
        let mut names = app.world_mut().query::<crate::ui::test_id::NodeKey>();
        let names: Vec<String> = names
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect();
        for expected in [
            "DefaultAvatarsLabel",
            "StudioAvatarsLabel",
            "PurchasedAvatarsHint",
            // Pairing starts from the menu, not from a launch flag.
            "WalletConnectButton",
        ] {
            assert!(names.iter().any(|name| name == expected), "{expected}");
        }
        assert_eq!(
            crate::passport::wallet_view(),
            crate::passport::WalletView::Disconnected
        );
        assert!(!crate::passport::poll_wallet());
        let mut sprite_choices = app.world_mut().query::<&SpriteSelectButton>();
        assert_eq!(sprite_choices.iter(app.world()).count(), 0);
        let mut teams = app.world_mut().query::<&TeamSelectButton>();
        assert_eq!(teams.iter(app.world()).count(), 1);
        // A 720p / 768p roster viewport is bounded while the complete content scrolls.
        for height in [720.0, 768.0] {
            app.world_mut().entity_mut(grid).insert((
                ComputedNode {
                    size: Vec2::new(672.0, height * 0.32),
                    content_size: Vec2::new(672.0, 700.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
                // No visibility propagation in this app.
                InheritedVisibility::VISIBLE,
            ));
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::PageDown);
            app.update();
            assert!(app.world().entity(grid).get::<ScrollPosition>().unwrap().y > 0.0);
            assert!(
                app.world().entity(grid).get::<ScrollPosition>().unwrap().y
                    <= 700.0 - height * 0.32
            );
        }
        let mut status = app.world_mut().query::<(&Name, Option<&Button>)>();
        assert!(
            status
                .iter(app.world())
                .any(|(name, button)| name.as_str() == "RendererStatus" && button.is_none())
        );
    }

    fn hero_select_app() -> App {
        use crate::ui::test_id::harness;
        let mut app = harness::kit_app();
        let mut session = ClientSession::default();
        session.set_state_for_test(ClientConnectionState::Connected);
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppScreen>()
            .insert_resource(session)
            .init_resource::<TeamSelection>()
            .init_resource::<crate::frontend::JoinNotice>()
            .add_message::<NetworkCommand>()
            .add_message::<SessionUiCommand>()
            .add_ui_action::<HeroSelectAction>()
            .add_systems(Startup, |mut commands: Commands| {
                spawn_team_select_ui(
                    &mut commands,
                    &TeamSelection::default(),
                    PlayerVisualMode::Models3d,
                    &SpriteVisualAssets::default(),
                    Handle::default(),
                    false,
                )
            })
            .add_systems(Update, team_select_ui_system.after(UiSet::Dispatch));
        app.update();
        app
    }

    #[test]
    fn class_presses_select_once_and_repaint_the_tiles() {
        use crate::ui::test_id::harness;
        let mut app = hero_select_app();
        let mage = format!("ClassButton-{}", HeroClass::Mage.id());
        harness::press(app.world_mut(), &mage);
        app.update();
        assert_eq!(
            harness::drain_actions::<HeroSelectAction>(app.world_mut()),
            [HeroSelectAction::Class(HeroClass::Mage)]
        );
        assert_eq!(
            app.world().resource::<TeamSelection>().hero_class,
            HeroClass::Mage
        );
        app.update();
        assert!(harness::drain_actions::<HeroSelectAction>(app.world_mut()).is_empty());
        let tile = harness::find(app.world_mut(), &mage).unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(tile).unwrap().0,
            SELECT_BUTTON_SELECTED_COLOR
        );
        let warrior = harness::find(
            app.world_mut(),
            &format!("ClassButton-{}", HeroClass::Warrior.id()),
        )
        .unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(warrior).unwrap().0,
            SELECT_BUTTON_COLOR
        );
        // A disabled tile does nothing.
        let ranger = format!("ClassButton-{}", HeroClass::Ranger.id());
        harness::set_disabled(app.world_mut(), &ranger, true);
        harness::press(app.world_mut(), &ranger);
        app.update();
        assert_eq!(
            app.world().resource::<TeamSelection>().hero_class,
            HeroClass::Mage
        );
    }

    #[test]
    fn the_lock_in_press_goes_through_the_lock_in_decision_once() {
        use crate::ui::test_id::harness;
        let mut app = hero_select_app();
        let button = harness::find(app.world_mut(), "FindMatchButton").unwrap();
        assert_eq!(
            app.world().get::<BackgroundColor>(button).unwrap().0,
            crate::ui::theme::PRIMARY
        );
        harness::set_disabled(app.world_mut(), "FindMatchButton", true);
        harness::press(app.world_mut(), "FindMatchButton");
        app.update();
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );
        harness::set_disabled(app.world_mut(), "FindMatchButton", false);
        harness::press(app.world_mut(), "FindMatchButton");
        app.update();
        let joins: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetworkCommand>>()
            .drain()
            .collect();
        // Not the sandbox, not offline: the prematch draft, then Searching.
        assert!(matches!(joins[..], [NetworkCommand::JoinPrematch { .. }]));
        assert_eq!(
            app.world().resource::<TeamSelection>().team,
            Some(Team::Green)
        );
        assert!(matches!(
            *app.world().resource::<NextState<AppScreen>>(),
            NextState::Pending(AppScreen::Searching)
        ));
        app.update();
        assert!(
            app.world()
                .resource::<Messages<NetworkCommand>>()
                .is_empty()
        );
    }

    #[test]
    fn phone_copy_follows_the_platform_and_the_language() {
        use crate::i18n::{Locale, LocaleId, relabel_localized};
        let mut controls = crate::mobile_controls::MobileControls::default();
        controls.enabled = true;
        let mut app = App::new();
        app.insert_resource(Locale::detached(LocaleId::ENGLISH))
            .insert_resource(controls)
            .add_systems(Update, apply_phone_copy)
            .add_systems(PostUpdate, relabel_localized);
        let label = app
            .world_mut()
            .spawn(PhoneCopy::label(
                "team.section.class",
                "team.section.class_phone",
            ))
            .id();
        let text = |app: &App| app.world().get::<Text>(label).unwrap().0.clone();
        assert_eq!(text(&app), "01  CHOOSE YOUR CLASS");
        app.update();
        assert_eq!(text(&app), "01  CLASS");
        app.insert_resource(Locale::detached(LocaleId::parse("zh-Hans").unwrap()));
        app.update();
        assert_eq!(text(&app), "01  职业");
        app.world_mut()
            .resource_mut::<crate::mobile_controls::MobileControls>()
            .enabled = false;
        app.update();
        assert_eq!(text(&app), "01  选择职业");
    }
}
