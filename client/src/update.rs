//! The client update path (LR-14, launch default O-12): client and server ship
//! together, so an incompatible game shows **Update required** with a download
//! button, and a newer compatible release a dismissible banner on Home.
//!
//! - Source of truth: the market's release catalog, `GET /v1/releases` of the
//!   Account API this build talks to (`OMOBA_ACCOUNT_API_URL`,
//!   `omoba_passport::release_catalog`). It is read at boot and again after a
//!   mismatch, and only the entries of this build's channel count
//!   ([`release_channel`], `direct` now). A build without an Account API never
//!   asks and never offers an update banner; a mismatch still says what to do.
//! - Required or not is the compatibility check's verdict
//!   (`ClientSession::compatibility_verdict`): the server's release and its
//!   update notice decide whether this game or the servers are behind. The
//!   catalog's `min_release` for this channel and platform also requires the
//!   update, even while the servers accept the game.
//! - The button target is chosen per (channel, platform) ([`update_link`]): a
//!   direct build opens a page on the portal it trusts (the catalog's
//!   `update_url`, else the server notice's link, else the portal's download
//!   page, `shared::compatibility::download_page`); without a portal, the
//!   catalog's package link. It is always a download page, never a payment
//!   page. A store build (iPhone) opens only its catalog update page, never a
//!   web download; without one the panel names the App Store or TestFlight.
//!   A direct build that knows no link names the official site.
// i18n-strict
use bevy::prelude::*;
use omoba_passport::release_catalog::{self, CatalogRelease};
use std::cmp::Ordering;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::net::{ClientSession, CompatibilityVerdict};

/// The catalog's platform names for this game's packages (`scripts/release.py`
/// `CATALOG_PLATFORMS`, kept in step by `scripts/test_release.py`): Rust
/// `target_os` → catalog `platform`. iOS is not in the catalog.
const CATALOG_PLATFORMS: &[(&str, &str)] = &[
    ("macos", "macos"),
    ("windows", "windows"),
    ("linux", "linux"),
    ("android", "android"),
];

/// A mismatch reads the catalog again at most this often.
const MISMATCH_REFRESH: Duration = Duration::from_secs(60);

/// This build's release channel: the only place that names it (LR-68 rules,
/// `docs/plans/launch-readiness.md` section 7). Every build is `direct` until
/// LR-68 makes the channel a runtime value.
fn release_channel() -> &'static str {
    release_catalog::DIRECT
}

/// A build whose updates come from a store: the iPhone build (App Store,
/// TestFlight) and, after LR-68, every channel but `direct`.
fn store_build() -> bool {
    cfg!(target_os = "ios") || release_channel() != release_catalog::DIRECT
}

/// The text key of the Update required panel: download from the link; with no
/// link, the App Store or TestFlight on iPhone, otherwise the official site.
pub(crate) fn required_body(link: bool) -> &'static str {
    required_body_for(link, cfg!(target_os = "ios"))
}

fn required_body_for(link: bool, ios: bool) -> &'static str {
    if link {
        "update.required.body"
    } else if ios {
        "update.required.body_store"
    } else {
        "update.required.body_site"
    }
}

/// This game's catalog platform, if it has one.
fn catalog_platform() -> Option<&'static str> {
    CATALOG_PLATFORMS
        .iter()
        .find(|(os, _)| *os == std::env::consts::OS)
        .map(|(_, platform)| *platform)
}

/// This game's catalog architecture (`arm64`, `x86_64`); a `universal`
/// package matches any.
fn catalog_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

/// What Home offers (rebuilt when it changes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateOffer {
    /// Online play needs a newer release: Home replaces Play with the update.
    Required {
        /// The newest catalog release, when it is newer than this game.
        latest: Option<String>,
        link: Option<String>,
    },
    /// A newer release is published: a dismissible banner.
    Available {
        version: String,
        link: Option<String>,
    },
}

impl UpdateOffer {
    pub(crate) fn link(&self) -> Option<&str> {
        match self {
            Self::Required { link, .. } | Self::Available { link, .. } => link.as_deref(),
        }
    }
}

/// The update link for this build's (channel, platform), see the module docs:
/// `catalog_page` is the catalog's `update_url` for this channel and platform,
/// `notice_url` the server notice's link, `package_url` a newer catalog
/// package.
fn update_link(
    store: bool,
    portal: Option<&str>,
    platform: Option<&str>,
    catalog_page: Option<&str>,
    notice_url: Option<&str>,
    package_url: Option<&str>,
) -> Option<String> {
    if store {
        return catalog_page.map(str::to_owned);
    }
    let Some(portal) = portal else {
        return catalog_page.or(package_url).map(str::to_owned);
    };
    let prefix = format!("{}/", portal.trim_end_matches('/'));
    let on_portal =
        |url: &&str| url.starts_with(&prefix) && shared::compatibility::valid_update_url(url);
    match catalog_page
        .filter(on_portal)
        .or(notice_url.filter(on_portal))
    {
        Some(url) if url.contains('?') => Some(url.to_owned()),
        Some(url) => Some(match platform {
            Some(platform) => format!("{url}?platform={platform}"),
            None => url.to_owned(),
        }),
        None => Some(shared::compatibility::download_page(portal, platform)),
    }
}

type CatalogResult = Result<Vec<CatalogRelease>, String>;

/// The update state of this run.
#[derive(Resource)]
pub(crate) struct ClientUpdate {
    /// The portal origin this build trusts (its download page); `None` when
    /// the build has no Account API, and then the catalog is never read.
    portal: Option<String>,
    /// The newest catalog release of this channel for this platform, once
    /// read.
    newest: Option<CatalogRelease>,
    pending: Option<Mutex<mpsc::Receiver<CatalogResult>>>,
    last_request: Option<Instant>,
    /// The version the player put off with "Later" in this run.
    dismissed: Option<String>,
    /// The link the last press opened.
    pub(crate) opened: Option<String>,
    fetch: fn() -> CatalogResult,
    open: fn(&str) -> Result<(), String>,
}

impl Default for ClientUpdate {
    fn default() -> Self {
        Self {
            portal: omoba_passport::web_account::WebAccountApi::from_env()
                .ok()
                .map(|api| api.origin),
            newest: None,
            pending: None,
            last_request: None,
            dismissed: None,
            opened: None,
            fetch: || omoba_passport::web_account::WebAccountApi::from_env()?.releases(),
            open: crate::platform::open_external_url,
        }
    }
}

impl ClientUpdate {
    /// What Home offers now, from the compatibility verdict and the catalog.
    pub(crate) fn offer(&self, session: &ClientSession) -> Option<UpdateOffer> {
        let current = env!("CARGO_PKG_VERSION");
        let newer = self.newest.as_ref().filter(|release| {
            shared::compatibility::release_order(&release.version, current)
                == Some(Ordering::Greater)
        });
        // The catalog's minimum for this channel and platform (it never
        // exceeds its entry's release, so a newer release exists).
        let below_minimum = self
            .newest
            .as_ref()
            .and_then(|release| release.min_release.as_deref())
            .is_some_and(|minimum| {
                shared::compatibility::release_order(current, minimum) == Some(Ordering::Less)
            });
        let link = || {
            update_link(
                store_build(),
                self.portal.as_deref(),
                catalog_platform(),
                self.newest
                    .as_ref()
                    .and_then(|release| release.update_url.as_deref()),
                session
                    .server_update
                    .as_ref()
                    .and_then(|notice| notice.url.as_deref()),
                newer.map(|release| release.url.as_str()),
            )
        };
        if below_minimum
            || session.compatibility_verdict() == Some(CompatibilityVerdict::UpdateRequired)
        {
            return Some(UpdateOffer::Required {
                latest: newer.map(|release| release.version.clone()),
                link: link(),
            });
        }
        let newer = newer.filter(|release| self.dismissed.as_ref() != Some(&release.version))?;
        Some(UpdateOffer::Available {
            version: newer.version.clone(),
            link: link(),
        })
    }

    /// Opens the update page for the current offer.
    pub(crate) fn open_link(&mut self, session: &ClientSession) {
        let Some(link) = self
            .offer(session)
            .and_then(|offer| offer.link().map(str::to_owned))
        else {
            return;
        };
        if let Err(error) = (self.open)(&link) {
            warn!("Update link {link} did not open: {error}");
        }
        self.opened = Some(link);
    }

    /// "Later": hides the banner for the offered version until the next run.
    pub(crate) fn dismiss(&mut self) {
        self.dismissed = self.newest.as_ref().map(|release| release.version.clone());
    }

    /// Starts one catalog read in the background.
    fn request(&mut self, now: Instant) {
        if self.portal.is_none() || self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let fetch = self.fetch;
        std::thread::spawn(move || {
            let _ = tx.send(fetch());
        });
        self.pending = Some(Mutex::new(rx));
        self.last_request = Some(now);
    }

    /// Takes a finished read. A failed read keeps what an earlier one found.
    fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let received = pending.lock().ok().map(|rx| rx.try_recv());
        let result = match received {
            Some(Err(mpsc::TryRecvError::Empty)) => return,
            Some(Ok(result)) => Some(result),
            _ => None,
        };
        self.pending = None;
        let Some(result) = result else {
            warn!("Release catalog read stopped without an answer");
            return;
        };
        match result {
            Ok(releases) => {
                self.newest = catalog_platform().and_then(|platform| {
                    release_catalog::newest(
                        &releases,
                        release_channel(),
                        platform,
                        catalog_architecture(),
                    )
                    .cloned()
                });
                info!(
                    "Release catalog: {} releases, newest for this platform {:?}",
                    releases.len(),
                    self.newest.as_ref().map(|release| &release.version)
                );
            }
            Err(error) => warn!("Release catalog unavailable: {error}"),
        }
    }

    /// Pretends the catalog listed `version` for this platform.
    #[cfg(test)]
    pub(crate) fn publish_for_test(&mut self, version: &str) {
        self.newest = Some(CatalogRelease {
            platform: catalog_platform().unwrap_or("linux").into(),
            architecture: release_catalog::UNIVERSAL.into(),
            version: version.into(),
            url: format!("https://downloads.example/omoba-{version}"),
            channel: release_catalog::DIRECT.into(),
            min_release: None,
            update_url: None,
        });
    }

    #[cfg(test)]
    pub(crate) fn for_test(portal: Option<&str>, fetch: fn() -> CatalogResult) -> Self {
        Self {
            portal: portal.map(str::to_owned),
            newest: None,
            pending: None,
            last_request: None,
            dismissed: None,
            opened: None,
            fetch,
            open: |_| Ok(()),
        }
    }
}

pub(crate) struct ClientUpdatePlugin;

impl Plugin for ClientUpdatePlugin {
    fn build(&self, app: &mut App) {
        // The boot read waits for the region's legal acceptance (LR-06).
        app.init_resource::<ClientUpdate>()
            .add_systems(
                Startup,
                read_catalog_at_boot.run_if(crate::legal::bound_and_accepted),
            )
            .add_systems(
                Update,
                (
                    read_catalog_at_boot.run_if(on_message::<crate::legal::LegalAccepted>),
                    read_catalog_after_mismatch,
                    poll_catalog,
                )
                    .chain(),
            );
    }
}

fn read_catalog_at_boot(mut update: ResMut<ClientUpdate>) {
    if crate::frontend::automation_bypass() {
        return;
    }
    update.request(Instant::now());
}

/// "On any mismatch": an update-required verdict reads the catalog again, so
/// the panel can name the new version (at most once a minute).
fn read_catalog_after_mismatch(session: Res<ClientSession>, mut update: ResMut<ClientUpdate>) {
    if session.compatibility_verdict() != Some(CompatibilityVerdict::UpdateRequired) {
        return;
    }
    let now = Instant::now();
    if update
        .last_request
        .is_none_or(|at| now.saturating_duration_since(at) >= MISMATCH_REFRESH)
    {
        update.request(now);
    }
}

fn poll_catalog(mut update: ResMut<ClientUpdate>) {
    update.poll();
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::compatibility::{CompatibilityIssue, ReleaseContract, UpdateNotice};

    /// LR-06: the boot read of the release catalog (the Account API) waits
    /// for the region's legal acceptance and runs in its frame.
    #[test]
    fn the_boot_catalog_read_waits_for_the_legal_acceptance() {
        use crate::legal::gate_tests::{assert_carries, gate, on_acceptance};
        let mut app = App::new();
        app.add_plugins(ClientUpdatePlugin);
        let read = IntoSystem::system_type_id(&read_catalog_at_boot);
        assert_carries(&app, Startup, read, gate(), "catalog read at boot");
        assert_carries(
            &app,
            Update,
            read,
            on_acceptance(),
            "catalog read on acceptance",
        );
    }

    const PORTAL: &str = "https://portal.example";

    fn release(version: &str) -> CatalogRelease {
        CatalogRelease {
            platform: catalog_platform().unwrap_or("linux").into(),
            architecture: release_catalog::UNIVERSAL.into(),
            version: version.into(),
            url: format!("https://downloads.example/omoba-{version}"),
            channel: release_catalog::DIRECT.into(),
            min_release: None,
            update_url: None,
        }
    }

    fn mismatch(server_release: &str, url: Option<&str>) -> ClientSession {
        let mut session = ClientSession::default();
        session.compatibility_issue = Some(CompatibilityIssue::Protocol);
        session.server_contract = Some(ReleaseContract {
            release: server_release.into(),
            ..ReleaseContract::current()
        });
        session.server_update = Some(UpdateNotice {
            min_release: server_release.into(),
            url: url.map(str::to_owned),
        });
        session
    }

    #[test]
    fn the_link_is_the_portal_download_page_for_this_platform() {
        let page = |notice: Option<&str>| {
            update_link(
                false,
                Some(PORTAL),
                Some("android"),
                None,
                notice,
                Some("https://d.example/a"),
            )
        };
        let download = Some(format!("{PORTAL}/download?platform=android"));
        assert_eq!(page(None), download);
        assert_eq!(page(Some(&format!("{PORTAL}/download"))), download);
        assert_eq!(
            page(Some(&format!("{PORTAL}/download/ru"))),
            Some(format!("{PORTAL}/download/ru?platform=android"))
        );
        // A notice link off this build's own portal is never opened.
        for foreign in [
            "https://evil.example/download",
            "https://portal.example.evil.example/download",
            "http://portal.example/download",
            "https://portal.example@evil.example/",
        ] {
            assert_eq!(page(Some(foreign)), download, "{foreign}");
        }
        // No portal: the catalog package; a store build without a catalog
        // update page (iPhone): nothing (App Store, TestFlight).
        assert_eq!(
            update_link(
                false,
                None,
                Some("linux"),
                None,
                None,
                Some("https://d.example/a")
            ),
            Some("https://d.example/a".to_owned())
        );
        assert_eq!(update_link(false, None, None, None, None, None), None);
        assert_eq!(
            update_link(true, Some(PORTAL), None, None, None, None),
            None
        );
        assert_eq!(
            update_link(false, Some(PORTAL), None, None, None, None),
            Some(format!("{PORTAL}/download"))
        );
    }

    /// LR-68 rules: the target is chosen per (channel, platform). A direct
    /// build takes the catalog's update page when it lies on its portal; a
    /// store build takes only the catalog's page and never a web download.
    #[test]
    fn the_catalog_update_page_is_chosen_per_channel() {
        let ru_page = format!("{PORTAL}/download/ru");
        let direct = |catalog: &str| {
            update_link(
                false,
                Some(PORTAL),
                Some("linux"),
                Some(catalog),
                Some(&format!("{PORTAL}/download")),
                Some("https://d.example/a"),
            )
        };
        assert_eq!(direct(&ru_page), Some(format!("{ru_page}?platform=linux")));
        assert_eq!(
            direct(&format!("{ru_page}?platform=windows")),
            Some(format!("{ru_page}?platform=windows"))
        );
        assert_eq!(
            direct("https://store.example/omoba"),
            Some(format!("{PORTAL}/download?platform=linux")),
            "a direct build stays on its own portal"
        );
        let store = |catalog: Option<&str>| {
            update_link(
                true,
                Some(PORTAL),
                Some("linux"),
                catalog,
                Some(&format!("{PORTAL}/download")),
                Some("https://d.example/a"),
            )
        };
        assert_eq!(
            store(Some("https://store.example/omoba")),
            Some("https://store.example/omoba".to_owned())
        );
        assert_eq!(store(None), None, "no web download in a store build");
        assert_eq!(release_channel(), release_catalog::DIRECT);
        assert_eq!(store_build(), cfg!(target_os = "ios"));
    }

    /// Without a link the panel names the store only on iPhone; any other
    /// build (the global market has no Account API or portal) names the
    /// official site.
    #[test]
    fn the_required_text_names_the_store_only_on_iphone() {
        assert_eq!(required_body_for(true, false), "update.required.body");
        assert_eq!(required_body_for(true, true), "update.required.body");
        assert_eq!(required_body_for(false, true), "update.required.body_store");
        assert_eq!(required_body_for(false, false), "update.required.body_site");
        let update = ClientUpdate::for_test(None, || Ok(vec![]));
        let Some(UpdateOffer::Required { link, .. }) = update.offer(&mismatch("999.0.0", None))
        else {
            panic!("an older game must update")
        };
        assert_eq!(link, None);
        assert_eq!(
            required_body(link.is_some()),
            if cfg!(target_os = "ios") {
                "update.required.body_store"
            } else {
                "update.required.body_site"
            }
        );
    }

    /// The catalog's `min_release` requires the update while the servers
    /// still accept this game; its `update_url` is the button target.
    #[test]
    fn the_catalog_minimum_requires_the_update_without_a_mismatch() {
        let mut update = ClientUpdate::for_test(Some(PORTAL), || Ok(vec![]));
        let session = ClientSession::default();
        let mut newest = release("999.0.0");
        newest.update_url = Some(format!("{PORTAL}/download/ru"));
        update.newest = Some(newest.clone());
        assert!(matches!(
            update.offer(&session),
            Some(UpdateOffer::Available { .. })
        ));
        newest.min_release = Some(env!("CARGO_PKG_VERSION").into());
        update.newest = Some(newest.clone());
        assert!(
            matches!(update.offer(&session), Some(UpdateOffer::Available { .. })),
            "this game meets the minimum"
        );
        newest.min_release = Some("999.0.0".into());
        update.newest = Some(newest);
        let Some(UpdateOffer::Required { latest, link }) = update.offer(&session) else {
            panic!("a game below the catalog minimum must update")
        };
        assert_eq!(latest.as_deref(), Some("999.0.0"));
        let page = format!("{PORTAL}/download/ru");
        let expected = match catalog_platform() {
            Some(platform) if !store_build() => format!("{page}?platform={platform}"),
            _ => page,
        };
        assert_eq!(link, Some(expected));
        // "Later" cannot put it off.
        update.dismiss();
        assert!(matches!(
            update.offer(&session),
            Some(UpdateOffer::Required { .. })
        ));
    }

    #[test]
    fn a_mismatch_with_an_older_game_requires_the_update() {
        let mut update = ClientUpdate::for_test(Some(PORTAL), || Ok(vec![]));
        update.newest = Some(release("999.0.0"));
        let session = mismatch("999.0.0", Some(&format!("{PORTAL}/download")));
        let Some(UpdateOffer::Required { latest, link }) = update.offer(&session) else {
            panic!("an older game must update")
        };
        assert_eq!(latest.as_deref(), Some("999.0.0"));
        let expected = cfg!(not(target_os = "ios"))
            .then(|| shared::compatibility::download_page(PORTAL, catalog_platform()));
        assert_eq!(link, expected);
        // The panel cannot be put off; the press opens the page.
        update.dismiss();
        assert!(matches!(
            update.offer(&session),
            Some(UpdateOffer::Required { .. })
        ));
        update.open_link(&session);
        assert_eq!(update.opened, expected);
        // Without a catalog the panel still asks for the update.
        update.newest = None;
        assert!(matches!(
            update.offer(&session),
            Some(UpdateOffer::Required { latest: None, .. })
        ));
    }

    #[test]
    fn a_newer_game_waits_for_the_servers_and_unverified_servers_ask_nothing() {
        let update = ClientUpdate::for_test(Some(PORTAL), || Ok(vec![]));
        assert_eq!(update.offer(&mismatch("0.0.1", None)), None);
        let mut offline = ClientSession::default();
        offline.compatibility_issue = Some(CompatibilityIssue::Unavailable);
        assert_eq!(update.offer(&offline), None);
        assert_eq!(update.offer(&ClientSession::default()), None);
    }

    #[test]
    fn a_newer_release_shows_a_banner_until_later() {
        let mut update = ClientUpdate::for_test(Some(PORTAL), || Ok(vec![]));
        let session = ClientSession::default();
        // The same or an older release offers nothing (a stale catalog).
        for version in [env!("CARGO_PKG_VERSION"), "0.0.1"] {
            update.newest = Some(release(version));
            assert_eq!(update.offer(&session), None, "{version}");
        }
        update.newest = Some(release("999.0.0"));
        assert!(matches!(
            update.offer(&session),
            Some(UpdateOffer::Available { ref version, .. }) if version == "999.0.0"
        ));
        update.dismiss();
        assert_eq!(update.offer(&session), None);
        // A still newer release is offered again.
        update.newest = Some(release("999.1.0"));
        assert!(update.offer(&session).is_some());
    }

    /// The boot read and the mismatch read go through the catalog seam; a
    /// failed read keeps the last answer; no Account API means no read.
    #[test]
    fn the_catalog_is_read_at_boot_and_after_a_mismatch() {
        fn answered(app: &mut App) {
            for _ in 0..200 {
                app.update();
                if app.world().resource::<ClientUpdate>().pending.is_none() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("the catalog read did not finish");
        }
        let mut app = App::new();
        app.insert_resource(ClientUpdate::for_test(Some(PORTAL), || {
            // Another channel's newer package never counts for this build.
            let store = CatalogRelease {
                channel: "app_store".into(),
                ..release("999.9.0")
            };
            Ok(vec![release("999.0.0"), store])
        }))
        .init_resource::<ClientSession>()
        .add_systems(Startup, read_catalog_at_boot)
        .add_systems(Update, (read_catalog_after_mismatch, poll_catalog).chain());
        answered(&mut app);
        let found = app.world().resource::<ClientUpdate>().newest.clone();
        assert_eq!(
            found.map(|release| release.version),
            catalog_platform().map(|_| "999.0.0".to_owned())
        );

        // A mismatch reads again once the minute has passed; a failure keeps
        // what the boot read found.
        let earlier = Instant::now().checked_sub(MISMATCH_REFRESH);
        {
            let mut update = app.world_mut().resource_mut::<ClientUpdate>();
            update.fetch = || Err("down".into());
            update.last_request = earlier;
        }
        *app.world_mut().resource_mut::<ClientSession>() = mismatch("999.0.0", None);
        app.update();
        assert!(
            app.world().resource::<ClientUpdate>().last_request > earlier,
            "the mismatch must read the catalog again"
        );
        answered(&mut app);
        assert_eq!(
            app.world()
                .resource::<ClientUpdate>()
                .newest
                .as_ref()
                .map(|release| release.version.as_str()),
            catalog_platform().map(|_| "999.0.0")
        );
        let last = app.world().resource::<ClientUpdate>().last_request;
        app.update();
        assert_eq!(
            app.world().resource::<ClientUpdate>().last_request,
            last,
            "no second read within the minute"
        );

        let mut unconfigured = ClientUpdate::for_test(None, || Ok(vec![release("999.0.0")]));
        unconfigured.request(Instant::now());
        assert!(unconfigured.pending.is_none());
    }
}
