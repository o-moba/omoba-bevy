//! The client update path (LR-14, launch default O-12): client and server ship
//! together, so an incompatible game shows **Update required** with a download
//! button, and a newer compatible release a dismissible banner on Home.
//!
//! - Source of truth: the market's release catalog, `GET /v1/releases` of the
//!   Account API this build talks to (`OMOBA_ACCOUNT_API_URL`,
//!   `omoba_passport::release_catalog`). It is read at boot and again after a
//!   mismatch. A build without an Account API never asks and never offers an
//!   update banner; a mismatch still says what to do.
//! - Required or not is the compatibility check's verdict
//!   (`ClientSession::compatibility_verdict`): the server's release and its
//!   update notice decide whether this game or the servers are behind.
//! - The button opens the portal's download page for this platform
//!   (`shared::compatibility::download_page`, the portal origin the build
//!   trusts). The server's notice link is used only when it lies on that same
//!   portal; without a portal, the catalog's package link. It is always a
//!   download page, never a payment page. iPhone builds get no link: their
//!   updates come from the App Store or TestFlight.
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

/// The update link (see the module docs), `None` on iPhone.
fn update_link(
    ios: bool,
    portal: Option<&str>,
    platform: Option<&str>,
    notice_url: Option<&str>,
    package_url: Option<&str>,
) -> Option<String> {
    if ios {
        return None;
    }
    let Some(portal) = portal else {
        return package_url.map(str::to_owned);
    };
    let prefix = format!("{}/", portal.trim_end_matches('/'));
    match notice_url
        .filter(|url| url.starts_with(&prefix) && shared::compatibility::valid_update_url(url))
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
    /// The newest catalog release for this platform, once read.
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
        let link = || {
            update_link(
                cfg!(target_os = "ios"),
                self.portal.as_deref(),
                catalog_platform(),
                session
                    .server_update
                    .as_ref()
                    .and_then(|notice| notice.url.as_deref()),
                newer.map(|release| release.url.as_str()),
            )
        };
        if session.compatibility_verdict() == Some(CompatibilityVerdict::UpdateRequired) {
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
                    release_catalog::newest(&releases, platform, catalog_architecture()).cloned()
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
        app.init_resource::<ClientUpdate>()
            .add_systems(Startup, read_catalog_at_boot)
            .add_systems(Update, (read_catalog_after_mismatch, poll_catalog).chain());
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

    const PORTAL: &str = "https://portal.example";

    fn release(version: &str) -> CatalogRelease {
        CatalogRelease {
            platform: catalog_platform().unwrap_or("linux").into(),
            architecture: release_catalog::UNIVERSAL.into(),
            version: version.into(),
            url: format!("https://downloads.example/omoba-{version}"),
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
        // No portal: the catalog package; iPhone: nothing (App Store, TestFlight).
        assert_eq!(
            update_link(
                false,
                None,
                Some("linux"),
                None,
                Some("https://d.example/a")
            ),
            Some("https://d.example/a".to_owned())
        );
        assert_eq!(update_link(false, None, None, None, None), None);
        assert_eq!(update_link(true, Some(PORTAL), None, None, None), None);
        assert_eq!(
            update_link(false, Some(PORTAL), None, None, None),
            Some(format!("{PORTAL}/download"))
        );
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
            Ok(vec![release("999.0.0")])
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
