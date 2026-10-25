//! The market's legal documents in the game (LR-06, LR-07): the first-launch
//! acceptance card, Settings → About and the gate on online traffic.
//!
//! - **Table.** The document version, the links and the age sign come from
//!   the market table (`shared::market::Market::legal`, mirrored by
//!   `scripts/markets.py`) through [`bound_region`], this module's one read of
//!   the market (LR-68 R0 swaps its body for the bound region). A link the
//!   table does not have is not shown. A market without a document version
//!   (global) shows no card and gates nothing; one without any entry shows no
//!   About page either.
//! - **Acceptance.** The card is a modal above every screen and below the
//!   boot splash. It names the documents, opens them in the browser, shows
//!   the age sign and lets the player switch the language. Continue stays
//!   disabled until the player ticks the acceptance switch. The choice is
//!   stored in the region folder (the settings folder,
//!   `OMOBA_CLIENT_CONFIG_DIR` included) as [`ACCEPTANCE_FILE`]:
//!   `{region, doc_version, accepted_at}` (Unix seconds, UTC). A record of
//!   another region or document version asks again. Nothing is sent to a
//!   server yet.
//! - **Gate.** [`bound_and_accepted`] is the one run condition for region
//!   traffic: the lobby transport (`net`: start, session lifecycle, menu
//!   retry), the release catalog read (`update`) and the career actions
//!   (`career`: VK sign-in, device links, website pairing) carry it. In the
//!   frame the player accepts, [`LegalAccepted`] starts what the boot skipped.
//!   Automation gets no exception: an RU harness writes the record into its
//!   own `OMOBA_CLIENT_CONFIG_DIR` first.
//!
//! The card is built so the region choice of LR-68 can join it: region and
//! legal become one Confirm.
// i18n-strict
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use shared::market::Legal;

use crate::i18n::{Locale, Localized, trf};
use crate::ui::kit_assets::Icon;
use crate::ui::theme::{self, ButtonKind, TextStyle};
use crate::ui::tokens::{TextRole, color};
use crate::ui::widgets::{self, ButtonSize};
use crate::ui::{
    Activated, ModalAppExt, ModalId, ModalRoot, Pressable, ScrollArea, UiActionAppExt, UiSet,
};

/// The acceptance record in the region folder.
pub(crate) const ACCEPTANCE_FILE: &str = "legal_acceptance.json";
/// A record larger than this is not ours.
const MAX_RECORD_BYTES: u64 = 4096;
/// Wheel step (UI px per line) of the card body on desktop.
const CARD_WHEEL_STEP: f32 = 28.0;

/// The region this game plays in and its legal table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    /// The key an acceptance is stored under (`ru`).
    pub id: &'static str,
    pub legal: Legal,
}

/// This game's region: the module's one read of the market (LR-68 rules,
/// `docs/plans/launch-readiness.md` section 7). Until LR-68 binds a region at
/// run time it is the build's market.
pub(crate) fn bound_region() -> Region {
    let market = shared::market::MARKET;
    Region {
        id: market.key(),
        legal: market.legal(),
    }
}

/// The age sign of this game's region (`12+`), for the boot splash.
pub(crate) fn age_rating() -> Option<&'static str> {
    bound_region().legal.age_rating
}

/// Whether Settings has an About page: the region has a document, a link or
/// the age sign (the global market has none, so its Settings are unchanged).
pub(crate) fn about_shown() -> bool {
    bound_region().legal.any()
}

/// What the region folder records about the accepted documents.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Acceptance {
    pub region: String,
    pub doc_version: String,
    /// Unix seconds, UTC.
    pub accepted_at: u64,
}

/// Written the frame the player accepts: starts the traffic the boot held back.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LegalAccepted;

/// The legal state of this run.
#[derive(Resource)]
pub(crate) struct LegalConsent {
    region: Region,
    file: Option<PathBuf>,
    /// The record of the current region and document version, if any.
    accepted: Option<Acceptance>,
    /// The acceptance switch on the card.
    checked: bool,
    /// A link the browser did not open; its address is shown instead.
    failed_link: Option<String>,
    /// The last link opened (tests read it).
    pub(crate) opened: Option<String>,
    open: fn(&str) -> Result<(), String>,
    now: fn() -> u64,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl LegalConsent {
    /// This game's region and its record in the settings folder.
    pub(crate) fn load() -> Self {
        Self::at(
            bound_region(),
            crate::platform::settings_directory().map(|directory| directory.join(ACCEPTANCE_FILE)),
        )
    }

    /// `region` with its record at `file`.
    pub(crate) fn at(region: Region, file: Option<PathBuf>) -> Self {
        let accepted = region.legal.version.and_then(|version| {
            file.as_deref()
                .and_then(read_record)
                .filter(|record| record.region == region.id && record.doc_version == version)
        });
        Self {
            region,
            file,
            accepted,
            checked: false,
            failed_link: None,
            opened: None,
            open: crate::platform::open_external_url,
            now: unix_now,
        }
    }

    /// The region's documents are accepted, or it has none to accept.
    pub(crate) fn accepted(&self) -> bool {
        self.region.legal.version.is_none() || self.accepted.is_some()
    }

    /// The card is up until the documents are accepted.
    fn card_open(&self) -> bool {
        !self.accepted()
    }

    /// Records the acceptance of the current version (the switch must be on)
    /// and saves it. A record that cannot be written still counts for this
    /// run; the card comes back at the next start.
    fn accept(&mut self) -> bool {
        let Some(version) = self.region.legal.version else {
            return false;
        };
        if !self.checked || self.accepted.is_some() {
            return false;
        }
        let record = Acceptance {
            region: self.region.id.to_owned(),
            doc_version: version.to_owned(),
            accepted_at: (self.now)(),
        };
        match self.file.as_deref() {
            Some(path) => {
                if let Err(error) = write_record(path, &record) {
                    warn!("Legal acceptance not saved at {path:?}: {error}");
                }
            }
            None => warn!("No settings folder: the legal acceptance holds for this run only"),
        }
        info!(
            "Legal documents {version} accepted for the {} region",
            record.region
        );
        self.accepted = Some(record);
        true
    }

    /// Opens `link`; a link the browser refuses is shown as text.
    fn open_link(&mut self, link: LegalLink) {
        let Some(url) = link.url(&self.region.legal) else {
            return;
        };
        match (self.open)(&url) {
            Ok(()) => self.failed_link = None,
            Err(error) => {
                warn!("Legal link {url} did not open: {error}");
                self.failed_link = Some(url.clone());
            }
        }
        self.opened = Some(url);
    }

    /// `region` with its record at `file`, opening links nowhere.
    #[cfg(test)]
    pub(crate) fn for_test(region: Region, file: Option<PathBuf>) -> Self {
        Self {
            open: |_| Ok(()),
            now: || 1_761_350_400,
            ..Self::at(region, file)
        }
    }
}

fn read_record(path: &Path) -> Option<Acceptance> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_RECORD_BYTES {
        return None;
    }
    serde_json::from_reader(file).ok()
}

fn write_record(path: &Path, record: &Acceptance) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(record).map_err(std::io::Error::other)?;
    crate::persistence::write_atomically(path, &data)
}

/// The one run condition for region traffic (LR-68 rules): the game is bound
/// to its region and the player accepted its documents. Without the plugin
/// (unit tests of other modules) the table decides, closed for a region that
/// asks.
pub(crate) fn bound_and_accepted(consent: Option<Res<LegalConsent>>) -> bool {
    match consent {
        Some(consent) => consent.accepted(),
        None => bound_region().legal.version.is_none(),
    }
}

/// A document or channel the game links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegalLink {
    Terms,
    Privacy,
    Source,
    CommunityVk,
    CommunityTelegram,
}

impl LegalLink {
    /// The links in the order the card and About list them.
    const ALL: [Self; 5] = [
        Self::Terms,
        Self::Privacy,
        Self::Source,
        Self::CommunityVk,
        Self::CommunityTelegram,
    ];

    /// The address in `legal`, if the region has one; the source of this
    /// game's own release.
    fn url(self, legal: &Legal) -> Option<String> {
        match self {
            Self::Terms => legal.terms.map(str::to_owned),
            Self::Privacy => legal.privacy.map(str::to_owned),
            Self::Source => legal.source_for(env!("CARGO_PKG_VERSION")),
            Self::CommunityVk => legal.community_vk.map(str::to_owned),
            Self::CommunityTelegram => legal.community_telegram.map(str::to_owned),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Terms => "legal.link.terms",
            Self::Privacy => "legal.link.privacy",
            Self::Source => "legal.link.source",
            Self::CommunityVk => "legal.link.community_vk",
            Self::CommunityTelegram => "legal.link.community_telegram",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Terms | Self::Privacy | Self::Source => Icon::NavLink,
            Self::CommunityVk | Self::CommunityTelegram => Icon::NavUsers,
        }
    }

    /// The button's `TestId` on the card or the About page.
    fn id(self, about: bool) -> &'static str {
        match (self, about) {
            (Self::Terms, false) => "LegalTerms",
            (Self::Privacy, false) => "LegalPrivacy",
            (Self::Source, false) => "LegalSource",
            (Self::CommunityVk, false) => "LegalCommunityVk",
            (Self::CommunityTelegram, false) => "LegalCommunityTelegram",
            (Self::Terms, true) => "AboutTerms",
            (Self::Privacy, true) => "AboutPrivacy",
            (Self::Source, true) => "AboutSource",
            (Self::CommunityVk, true) => "AboutCommunityVk",
            (Self::CommunityTelegram, true) => "AboutCommunityTelegram",
        }
    }
}

/// Everything a legal control can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegalAction {
    ToggleAccept,
    Continue,
    CycleLanguage,
    Open(LegalLink),
}

pub(crate) struct LegalPlugin;

impl Plugin for LegalPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<LegalConsent>() {
            app.insert_resource(LegalConsent::load());
        }
        app.add_message::<LegalAccepted>()
            .add_ui_action::<LegalAction>()
            .register_modal::<LegalConsent>(ModalId::Legal, LegalConsent::card_open)
            .add_systems(Startup, spawn_card)
            .add_systems(
                Update,
                (legal_actions, sync_legal_ui)
                    .chain()
                    .after(UiSet::Dispatch)
                    .before(UiSet::Paint),
            );
    }
}

#[derive(Component)]
struct LegalCardRoot;

#[derive(Component)]
struct AcceptSwitch;

#[derive(Component)]
struct ContinueButton;

#[derive(Component)]
struct CardLanguageValue;

/// The "could not open" line of the card and of the About page.
#[derive(Component)]
struct LinkStatus;

fn full_layer() -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn caption(text: impl crate::i18n::UiLabel) -> impl Bundle {
    (
        text.into_text(),
        theme::role_text(TextRole::Caption),
        TextColor(theme::MUTED),
        Node {
            flex_shrink: 0.0,
            ..default()
        },
    )
}

/// The age sign as a framed badge (`12+`).
fn spawn_age_badge(
    parent: &mut ChildSpawnerCommands,
    age: &str,
    (name, text_name): (&'static str, &'static str),
) {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor::all(color::GOLD_700),
            Name::new(name),
        ))
        .with_children(|badge| {
            badge.spawn((
                Text::new(age),
                theme::styled_text(TextStyle::keep_case(TextRole::Label)),
                TextColor(theme::GOLD),
                Name::new(text_name),
            ));
        });
}

fn link_button(parent: &mut ChildSpawnerCommands, link: LegalLink, about: bool) {
    let kind = if about {
        ButtonKind::Secondary
    } else {
        ButtonKind::Link
    };
    let mut node = widgets::button_node(ButtonSize::Regular, kind, theme::Form::Desktop);
    if about {
        node.width = Val::Percent(100.0);
    }
    node.justify_content = if about {
        JustifyContent::FlexStart
    } else {
        JustifyContent::Center
    };
    widgets::spawn_button(
        parent,
        node,
        Localized::new(link.key()),
        TextStyle::new(TextRole::Button),
        kind,
        Some(link.icon()),
        LegalAction::Open(link),
        link.id(about).into(),
        (),
    );
}

/// The first-launch card: documents, age sign, language, the acceptance
/// switch and Continue.
fn spawn_card(
    mut commands: Commands,
    consent: Res<LegalConsent>,
    platform: Option<Res<crate::ui::UiPlatform>>,
) {
    if !consent.card_open() {
        return;
    }
    let legal = consent.region.legal;
    let form = theme::Form::of(platform.as_deref().is_some_and(|p| p.is_mobile()));
    commands
        .spawn((
            full_layer(),
            BackgroundColor(color::BG_BASE),
            GlobalZIndex(ModalId::Legal.layer()),
            ModalRoot(ModalId::Legal),
            bevy::ui::FocusPolicy::Block,
            LegalCardRoot,
            Name::new("LegalCard"),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(92.0),
                    max_width: Val::Px(560.0),
                    max_height: Val::Percent(92.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(14.0),
                    padding: UiRect::all(Val::Px(24.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(theme::PANEL.with_alpha(1.0)),
                BorderColor::all(color::GOLD_700),
                Name::new("LegalCardPanel"),
            ))
            .with_children(|panel| {
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(12.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        Name::new("LegalCardHeader"),
                    ))
                    .with_children(|header| {
                        header.spawn((
                            Localized::new("legal.card.title").into_text(),
                            theme::role_text(TextRole::Title),
                            TextColor(theme::GOLD),
                            Name::new("LegalCardTitle"),
                        ));
                        if let Some(age) = legal.age_rating {
                            spawn_age_badge(header, age, ("LegalCardAge", "LegalCardAgeText"));
                        }
                    });
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Stretch,
                            row_gap: Val::Px(10.0),
                            flex_grow: 1.0,
                            flex_shrink: 1.0,
                            min_height: Val::Px(0.0),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                        ScrollArea::menu(CARD_WHEEL_STEP),
                        Name::new("LegalCardBody"),
                    ))
                    .with_children(|body| {
                        body.spawn((
                            Localized::new("legal.card.body").into_text(),
                            theme::role_text(TextRole::Body),
                            TextColor(theme::IVORY),
                            Node {
                                flex_shrink: 0.0,
                                ..default()
                            },
                            Name::new("LegalCardBodyText"),
                        ));
                        body.spawn((
                            Node {
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(8.0),
                                row_gap: Val::Px(4.0),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            Name::new("LegalCardLinks"),
                        ))
                        .with_children(|links| {
                            for link in [LegalLink::Terms, LegalLink::Privacy] {
                                if link.url(&legal).is_some() {
                                    link_button(links, link, false);
                                }
                            }
                        });
                        if let Some(version) = legal.version {
                            body.spawn((
                                caption(Localized::with_args(
                                    "legal.card.version",
                                    [("version", &version)],
                                )),
                                Name::new("LegalCardVersion"),
                            ));
                        }
                        body.spawn((caption(""), LinkStatus, Name::new("LegalCardLinkStatus")));
                        widgets::toggle_row(
                            body,
                            Localized::new("legal.card.language"),
                            crate::i18n::active().native_name(),
                            CardLanguageValue,
                            LegalAction::CycleLanguage,
                            "LegalLanguage",
                        );
                        let switch = widgets::controls::toggle(
                            body,
                            Localized::new("legal.card.accept"),
                            false,
                            LegalAction::ToggleAccept,
                            "LegalAccept",
                        );
                        body.commands().entity(switch).insert(AcceptSwitch);
                    });
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::Center,
                            flex_shrink: 0.0,
                            ..default()
                        },
                        Name::new("LegalCardFooter"),
                    ))
                    .with_children(|footer| {
                        let mut node =
                            widgets::button_node(ButtonSize::Large, ButtonKind::Primary, form);
                        node.width = Val::Percent(100.0);
                        let button = widgets::spawn_button(
                            footer,
                            node,
                            Localized::new("legal.card.continue"),
                            TextStyle::new(TextRole::ButtonLg),
                            ButtonKind::Primary,
                            Some(Icon::NavCheck),
                            LegalAction::Continue,
                            "LegalContinue".into(),
                            (),
                        );
                        footer.commands().entity(button).insert((
                            ContinueButton,
                            Pressable {
                                disabled: true,
                                ..default()
                            },
                        ));
                    });
            });
        });
}

/// Settings → About: the build, the age sign, the documents, the source of
/// this release and the region's channels. Spawned only when
/// [`about_shown`].
pub(crate) fn spawn_about(parent: &mut ChildSpawnerCommands) {
    spawn_about_for(parent, bound_region().legal);
}

fn spawn_about_for(parent: &mut ChildSpawnerCommands, legal: Legal) {
    parent.spawn((
        caption(Localized::with_args(
            "legal.about.build",
            [("build", &crate::build_info::label())],
        )),
        Name::new("AboutBuild"),
    ));
    if let Some(age) = legal.age_rating {
        parent
            .spawn((
                Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                Name::new("AboutAgeRow"),
            ))
            .with_children(|row| {
                row.spawn((
                    Localized::new("legal.about.age").into_text(),
                    theme::role_text(TextRole::Label),
                    TextColor(color::TEXT_SECONDARY),
                ));
                spawn_age_badge(row, age, ("AboutAge", "AboutAgeText"));
            });
    }
    for link in LegalLink::ALL {
        if link.url(&legal).is_some() {
            link_button(parent, link, true);
        }
    }
    if let Some(email) = legal.support_email {
        parent.spawn((
            caption(Localized::with_args(
                "legal.about.support",
                [("email", &email)],
            )),
            Name::new("AboutSupport"),
        ));
    }
    parent.spawn((caption(""), LinkStatus, Name::new("AboutLinkStatus")));
}

fn legal_actions(
    mut activated: MessageReader<Activated<LegalAction>>,
    mut consent: ResMut<LegalConsent>,
    mut locale: Option<ResMut<Locale>>,
    mut accepted: MessageWriter<LegalAccepted>,
) {
    for Activated { action, .. } in activated.read() {
        match *action {
            LegalAction::ToggleAccept => {
                if consent.card_open() {
                    consent.checked = !consent.checked;
                }
            }
            LegalAction::Continue => {
                if consent.accept() {
                    accepted.write(LegalAccepted);
                }
            }
            LegalAction::CycleLanguage => {
                if let Some(locale) = locale.as_deref_mut() {
                    let next = locale.id().next();
                    info!("Language: {} ({})", next.native_name(), next.code());
                    locale.set(next);
                }
            }
            LegalAction::Open(link) => consent.open_link(link),
        }
    }
}

#[allow(clippy::type_complexity)]
fn sync_legal_ui(
    mut commands: Commands,
    consent: Res<LegalConsent>,
    locale: Option<Res<Locale>>,
    roots: Query<Entity, With<LegalCardRoot>>,
    mut switches: Query<&mut widgets::ButtonStyle, With<AcceptSwitch>>,
    mut buttons: Query<&mut Pressable, With<ContinueButton>>,
    mut statuses: Query<&mut Text, (With<LinkStatus>, Without<CardLanguageValue>)>,
    mut languages: Query<&mut Text, (With<CardLanguageValue>, Without<LinkStatus>)>,
) {
    if !consent.card_open() {
        for root in &roots {
            commands.entity(root).despawn();
        }
    }
    let relabel = locale.as_ref().is_some_and(|locale| locale.is_changed());
    if consent.is_changed() || relabel {
        for mut style in &mut switches {
            widgets::ButtonStyle::set_selected(&mut style, consent.checked);
        }
        for mut pressable in &mut buttons {
            if pressable.disabled == consent.checked {
                pressable.disabled = !consent.checked;
            }
        }
        let status = consent
            .failed_link
            .as_ref()
            .map(|url| trf("legal.link.failed", &[("url", url)]))
            .unwrap_or_default();
        for mut text in &mut statuses {
            if text.0 != status {
                text.0.clone_from(&status);
            }
        }
    }
    if relabel && let Some(locale) = locale {
        let name = locale.id().native_name();
        for mut value in &mut languages {
            if value.0 != name {
                value.0 = name.to_owned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_id::harness;
    use shared::market::Market;

    fn ru() -> Region {
        Region {
            id: "ru",
            legal: Market::Ru.legal(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "omoba-legal-{name}-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn accept_now(consent: &mut LegalConsent) -> bool {
        consent.checked = true;
        consent.accept()
    }

    #[test]
    fn a_region_without_documents_asks_nothing_and_gates_nothing() {
        let global = Region {
            id: "global",
            legal: Market::Global.legal(),
        };
        let consent = LegalConsent::for_test(global, None);
        assert!(consent.accepted());
        assert!(!consent.card_open());
        // The compiled table decides without the plugin.
        let mut app = App::new();
        let without_plugin = app
            .world_mut()
            .run_system_cached(bound_and_accepted)
            .unwrap();
        assert_eq!(without_plugin, bound_region().legal.version.is_none());
    }

    /// LR-06 acceptance: the accepted version persists in the region folder,
    /// and a version bump (or another region's record) asks again.
    #[test]
    fn the_acceptance_persists_and_a_new_version_asks_again() {
        let directory = scratch("persist");
        let file = directory.join(ACCEPTANCE_FILE);
        let mut consent = LegalConsent::for_test(ru(), Some(file.clone()));
        assert!(!consent.accepted() && consent.card_open());
        assert!(!consent.accept(), "the switch must be on");
        assert!(accept_now(&mut consent));
        assert!(consent.accepted());
        let record: Acceptance = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            record,
            Acceptance {
                region: "ru".into(),
                doc_version: "0-draft".into(),
                accepted_at: 1_761_350_400,
            }
        );
        // The next start reads it.
        assert!(LegalConsent::for_test(ru(), Some(file.clone())).accepted());
        // A new document version asks again; accepting replaces the record.
        let mut bumped = ru();
        bumped.legal.version = Some("1");
        let mut consent = LegalConsent::for_test(bumped, Some(file.clone()));
        assert!(!consent.accepted());
        assert!(accept_now(&mut consent));
        let record: Acceptance = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(record.doc_version, "1");
        assert!(!LegalConsent::for_test(ru(), Some(file.clone())).accepted());
        // Another region's record, a broken or an oversized file is no acceptance.
        let other = Region { id: "cn", ..ru() };
        assert!(!LegalConsent::for_test(other, Some(file.clone())).accepted());
        std::fs::write(&file, b"{not json").unwrap();
        assert!(!LegalConsent::for_test(ru(), Some(file.clone())).accepted());
        let mut big = serde_json::to_vec(&Acceptance {
            region: "ru".into(),
            doc_version: "0-draft".into(),
            accepted_at: 1,
        })
        .unwrap();
        big.resize(MAX_RECORD_BYTES as usize + 1, b' ');
        std::fs::write(&file, big).unwrap();
        assert!(!LegalConsent::for_test(ru(), Some(file)).accepted());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn an_unwritable_folder_still_accepts_for_this_run() {
        let directory = scratch("unwritable");
        std::fs::create_dir_all(&directory).unwrap();
        // A file where the folder should be: the record cannot be written.
        let blocker = directory.join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let mut consent =
            LegalConsent::for_test(ru(), Some(blocker.join("inner").join(ACCEPTANCE_FILE)));
        assert!(accept_now(&mut consent));
        assert!(consent.accepted());
        let mut no_folder = LegalConsent::for_test(ru(), None);
        assert!(accept_now(&mut no_folder));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn links_come_from_the_table_and_a_missing_one_is_not_offered() {
        let legal = Market::Ru.legal();
        assert_eq!(
            LegalLink::Terms.url(&legal).as_deref(),
            Some("https://omoba.ru/terms")
        );
        assert_eq!(
            LegalLink::Privacy.url(&legal).as_deref(),
            Some("https://omoba.ru/privacy")
        );
        assert_eq!(
            LegalLink::Source.url(&legal),
            Some(format!(
                "https://github.com/o-moba/omoba-bevy/tree/v{}-ru",
                env!("CARGO_PKG_VERSION")
            ))
        );
        assert_eq!(LegalLink::CommunityVk.url(&legal), None);
        assert_eq!(LegalLink::CommunityTelegram.url(&legal), None);
        for link in LegalLink::ALL {
            assert_eq!(link.url(&Legal::NONE), None, "{link:?}");
            assert_ne!(link.id(false), link.id(true));
        }
        let mut consent = LegalConsent::for_test(ru(), None);
        consent.open = |_| Err("no browser".into());
        consent.open_link(LegalLink::Privacy);
        assert_eq!(
            consent.failed_link.as_deref(),
            Some("https://omoba.ru/privacy")
        );
        consent.open = |_| Ok(());
        consent.open_link(LegalLink::Terms);
        assert_eq!(consent.failed_link, None);
        assert_eq!(consent.opened.as_deref(), Some("https://omoba.ru/terms"));
    }

    fn card_app(consent: LegalConsent) -> App {
        let mut app = harness::kit_app();
        app.add_plugins(crate::i18n::I18nPlugin::default())
            .insert_resource(consent)
            .insert_resource(crate::ui::UiPlatform(crate::platform::UiProfile::Desktop))
            .add_plugins(LegalPlugin);
        app
    }

    fn text_of(app: &mut App, name: &str) -> Option<String> {
        app.world_mut()
            .query::<(&Name, &Text)>()
            .iter(app.world())
            .find(|(found, _)| found.as_str() == name)
            .map(|(_, text)| text.0.clone())
    }

    fn continue_disabled(app: &mut App) -> bool {
        let button = harness::find(app.world_mut(), "LegalContinue").unwrap();
        app.world().get::<Pressable>(button).unwrap().disabled
    }

    /// LR-06 acceptance in the UI tree: the card is the top modal, Continue
    /// waits for the switch, Continue saves the record, closes the card and
    /// announces [`LegalAccepted`]; the gate opens.
    #[test]
    fn the_card_waits_for_the_switch_then_saves_and_opens_the_gate() {
        let directory = scratch("card");
        let file = directory.join(ACCEPTANCE_FILE);
        let mut app = card_app(LegalConsent::for_test(ru(), Some(file.clone())));
        app.update();
        app.update();
        assert!(harness::find(app.world_mut(), "LegalCard").is_none());
        assert_eq!(
            app.world().resource::<crate::ui::ModalStack>().top(),
            Some(ModalId::Legal)
        );
        assert!(
            !app.world_mut()
                .run_system_cached(bound_and_accepted)
                .unwrap()
        );
        assert_eq!(
            text_of(&mut app, "LegalCardAgeText").as_deref(),
            Some("12+")
        );
        for id in [
            "LegalTerms",
            "LegalPrivacy",
            "LegalAccept",
            "LegalLanguageButton",
        ] {
            assert!(harness::find(app.world_mut(), id).is_some(), "{id}");
        }
        // Support and community are not provided yet: not on the card.
        assert!(harness::find(app.world_mut(), "LegalCommunityVk").is_none());
        assert_eq!(
            text_of(&mut app, "LegalCardVersion").as_deref(),
            Some("Documents version: 0-draft")
        );
        assert!(continue_disabled(&mut app));
        // A press on the disabled Continue changes nothing.
        harness::press(app.world_mut(), "LegalContinue");
        app.update();
        assert!(app.world().resource::<LegalConsent>().card_open());
        assert!(!file.exists());

        harness::press(app.world_mut(), "LegalTerms");
        app.update();
        assert_eq!(
            app.world().resource::<LegalConsent>().opened.as_deref(),
            Some("https://omoba.ru/terms")
        );
        harness::press(app.world_mut(), "LegalAccept");
        app.update();
        assert!(!continue_disabled(&mut app));
        harness::press(app.world_mut(), "LegalContinue");
        app.update();
        let announced = app
            .world_mut()
            .resource_mut::<Messages<LegalAccepted>>()
            .drain()
            .count();
        assert_eq!(announced, 1);
        app.update();
        assert!(app.world().resource::<LegalConsent>().accepted());
        assert!(
            app.world_mut()
                .run_system_cached(bound_and_accepted)
                .unwrap()
        );
        assert!(
            app.world_mut()
                .query::<&LegalCardRoot>()
                .iter(app.world())
                .next()
                .is_none(),
            "the card is gone"
        );
        assert_eq!(app.world().resource::<crate::ui::ModalStack>().top(), None);
        assert!(file.exists());
        // The next start shows no card.
        let mut again = card_app(LegalConsent::for_test(ru(), Some(file)));
        again.update();
        assert!(harness::find(again.world_mut(), "LegalContinue").is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    /// Settings → About lists what the table has: the build, the age sign,
    /// the documents and this release's source; support and community appear
    /// once the owner provides them.
    #[test]
    fn about_lists_the_build_age_documents_source_and_provided_channels() {
        let names = |app: &mut App| -> Vec<String> {
            app.world_mut()
                .query::<&crate::ui::TestId>()
                .iter(app.world())
                .map(|id| id.as_str().to_owned())
                .collect()
        };
        // Accepted: no card above Settings.
        let mut accepted = LegalConsent::for_test(ru(), None);
        assert!(accept_now(&mut accepted));
        let mut app = card_app(accepted);
        harness::spawn_ui(app.world_mut(), |parent| {
            spawn_about_for(parent, Market::Ru.legal())
        });
        app.update();
        let ids = names(&mut app);
        for id in ["AboutTerms", "AboutPrivacy", "AboutSource"] {
            assert!(ids.iter().any(|found| found == id), "{id}: {ids:?}");
        }
        for id in ["AboutCommunityVk", "AboutCommunityTelegram"] {
            assert!(!ids.iter().any(|found| found == id), "{id}");
        }
        assert_eq!(text_of(&mut app, "AboutAgeText").as_deref(), Some("12+"));
        assert_eq!(
            text_of(&mut app, "AboutBuild"),
            Some(format!("Build: {}", crate::build_info::label()))
        );
        assert!(text_of(&mut app, "AboutSupport").is_none());
        harness::press(app.world_mut(), "AboutSource");
        app.update();
        assert_eq!(
            app.world().resource::<LegalConsent>().opened,
            Some(format!(
                "https://github.com/o-moba/omoba-bevy/tree/v{}-ru",
                env!("CARGO_PKG_VERSION")
            ))
        );

        let provided = Legal {
            support_email: Some("help@example.org"),
            community_vk: Some("https://vk.com/example"),
            community_telegram: Some("https://t.me/example"),
            ..Market::Ru.legal()
        };
        let mut app = card_app(LegalConsent::for_test(ru(), None));
        harness::spawn_ui(app.world_mut(), |parent| spawn_about_for(parent, provided));
        app.update();
        let ids = names(&mut app);
        for id in ["AboutCommunityVk", "AboutCommunityTelegram"] {
            assert!(ids.iter().any(|found| found == id), "{id}: {ids:?}");
        }
        assert_eq!(
            text_of(&mut app, "AboutSupport").as_deref(),
            Some("Support: help@example.org")
        );
    }

    /// The card in Russian (the RU first-run language).
    #[test]
    fn the_card_reads_in_russian() {
        if crate::i18n::testing::isolated("legal::tests::the_card_reads_in_russian") {
            return;
        }
        let mut app = card_app(LegalConsent::for_test(ru(), None));
        app.world_mut()
            .resource_mut::<Locale>()
            .set(crate::i18n::LocaleId::parse("ru").unwrap());
        app.update();
        app.update();
        assert_eq!(
            text_of(&mut app, "LegalCardTitle").as_deref(),
            Some("Перед игрой")
        );
        assert_eq!(
            text_of(&mut app, "LegalCardVersion").as_deref(),
            Some("Версия документов: 0-draft")
        );
        let language = harness::find(app.world_mut(), "LegalLanguageValue").unwrap();
        assert_eq!(app.world().get::<Text>(language).unwrap().0, "Русский");
        // The language row switches the language from the card.
        harness::press(app.world_mut(), "LegalLanguageButton");
        app.update();
        assert_ne!(
            app.world().resource::<Locale>().id(),
            crate::i18n::LocaleId::parse("ru").unwrap()
        );
    }
}

/// Test support for the modules that wire the gate: which run conditions a
/// registered system carries. System names need Bevy's `debug` feature, so
/// systems and conditions are matched by their system type.
#[cfg(test)]
pub(crate) mod gate_tests {
    use std::any::TypeId;

    use bevy::ecs::schedule::ScheduleLabel;
    use bevy::prelude::*;

    /// [`super::bound_and_accepted`] as a condition type.
    pub(crate) fn gate() -> TypeId {
        IntoSystem::system_type_id(&super::bound_and_accepted)
    }

    /// `on_message::<LegalAccepted>` as a condition type.
    pub(crate) fn on_acceptance() -> TypeId {
        IntoSystem::system_type_id(&on_message::<super::LegalAccepted>)
    }

    /// Asserts that the one system of type `system` in `schedule` carries the
    /// condition `condition`.
    pub(crate) fn assert_carries(
        app: &App,
        schedule: impl ScheduleLabel,
        system: TypeId,
        condition: TypeId,
        what: &str,
    ) {
        let found: Vec<Vec<TypeId>> = app
            .get_schedule(schedule)
            .unwrap_or_else(|| panic!("{what}: no such schedule"))
            .graph()
            .systems
            .iter()
            .filter(|(_, candidate, _)| (***candidate).system_type() == system)
            .map(|(_, _, conditions)| {
                conditions
                    .iter()
                    .map(|condition| (*condition.condition).system_type())
                    .collect()
            })
            .collect();
        assert_eq!(found.len(), 1, "{what}: registered {} times", found.len());
        assert!(
            found[0].contains(&condition),
            "{what} does not carry the condition"
        );
    }
}
