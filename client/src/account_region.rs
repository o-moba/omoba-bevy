//! The account's region is remembered independently of regional identities.
//! No account proof is sent to another region to discover where it belongs.
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use serde::{Deserialize, Serialize};
pub(crate) use shared::region::RegionId;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const FILE: &str = "account_region.json";
const MAX_BYTES: u64 = 4096;
static SWITCH_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn switch_pending() -> bool {
    SWITCH_PENDING.load(Ordering::Acquire)
}
pub(crate) fn selectable() -> bool {
    option_env!("OMOBA_MARKET") != Some("ru")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountBinding {
    pub region: RegionId,
    pub profile_id: String,
    pub public_key: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    pub schema: u32,
    pub selected: Option<RegionId>,
    pub pending: Option<RegionId>,
    pub account: Option<AccountBinding>,
}
impl Selection {
    pub(crate) fn region(&self) -> Option<RegionId> {
        self.pending
            .or_else(|| self.account.as_ref().map(|a| a.region))
            .or(self.selected)
    }
    fn valid(&self) -> bool {
        self.schema == 1
            && self.account.as_ref().is_none_or(|a| {
                [a.public_key.as_str(), a.profile_id.as_str()]
                    .iter()
                    .all(|s| {
                        s.len() == 64
                            && s.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    && self.selected.is_none_or(|r| r == a.region)
            })
    }
}

pub(crate) fn read(path: &Path) -> Option<Selection> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_BYTES {
        return None;
    }
    let value: Selection = serde_json::from_reader(file).ok()?;
    value.valid().then_some(value)
}
fn save(path: &Path, value: &Selection) -> Result<(), String> {
    if !value.valid() {
        return Err("Invalid account region record.".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Cannot save account region.")?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "Invalid account region record.")?;
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| "Cannot save account region.")?;
    Ok(())
}
pub(crate) fn path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("OMOBA_CLIENT_CONFIG_DIR").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(base).join(FILE));
    }
    let global = crate::platform::preferences_directory_for(shared::market::Market::Global)?;
    let suffix = crate::session_config::BUILD_CONTOUR
        .map(|s| format!("-{s}"))
        .unwrap_or_default();
    global
        .parent()
        .map(|parent| parent.join(format!("omoba-bevy-device{suffix}")).join(FILE))
}
/// Existing regional installations identify their account without contacting a
/// service. Ambiguous or malformed state asks the player instead of guessing.
pub(crate) fn choose(saved: Option<&Selection>, legacy: &[RegionId]) -> Option<RegionId> {
    saved.and_then(Selection::region).or(match legacy {
        [only] => Some(*only),
        _ => None,
    })
}
pub(crate) fn install() {
    let file = path();
    let saved = file.as_deref().and_then(read);
    let locked = if option_env!("OMOBA_MARKET") == Some("ru") {
        Some(RegionId::Ru)
    } else {
        None
    };
    let legacy: Vec<_> = if cfg!(test) || file.as_deref().is_some_and(Path::exists) {
        vec![]
    } else {
        RegionId::ALL
            .into_iter()
            .filter(|region| {
                regional_directory(*region)
                    .is_some_and(|dir| dir.join("career_identity.json").is_file())
            })
            .collect()
    };
    if let Some(region) = locked.or_else(|| choose(saved.as_ref(), &legacy)) {
        if let Err(error) = shared::region::bind(region) {
            eprintln!("Account region: {error}");
        }
        if saved.as_ref().is_some_and(|s| s.pending.is_some()) {
            let next = Selection {
                schema: 1,
                selected: Some(region),
                pending: None,
                account: None,
            };
            if let Some(file) = file {
                let _ = save(&file, &next);
            }
        }
    }
}
pub(crate) fn bound() -> bool {
    !switch_pending() && (cfg!(test) || shared::region::active().is_some())
}

/// No regional integration starts before first confirmation or after a pending
/// switch. In particular, polling a screen cannot construct an Ekza HTTP client.
pub(crate) fn features() -> shared::market::Features {
    if bound() {
        shared::region::features()
    } else {
        shared::market::Features {
            ekza: false,
            supporter: false,
            vk_sign_in: false,
        }
    }
}
pub(crate) fn payment_providers() -> &'static [shared::market::PaymentProvider] {
    if bound() {
        shared::region::payment_providers()
    } else {
        &[]
    }
}

pub(crate) fn request_switch(region: RegionId) -> Result<(), String> {
    let Some(active) = shared::region::active() else {
        return bind_initial(region);
    };
    if active.id == region {
        return Ok(());
    }
    let file = path().ok_or("No writable account settings directory.")?;
    let mut selection = read(&file).unwrap_or(Selection {
        schema: 1,
        selected: Some(active.id),
        ..default()
    });
    selection.pending = Some(region);
    save(&file, &selection)?;
    SWITCH_PENDING.store(true, Ordering::Release);
    Ok(())
}
pub(crate) fn regional_directory(region: RegionId) -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("OMOBA_CLIENT_CONFIG_DIR").filter(|s| !s.is_empty()) {
        return Some(override_directory(
            PathBuf::from(base),
            region,
            shared::market::MARKET,
        ));
    }
    crate::platform::preferences_directory_for(region.market())
}
pub(crate) fn override_directory(
    base: PathBuf,
    region: RegionId,
    compiled: shared::market::Market,
) -> PathBuf {
    // Keep existing single-market test/self-host directories intact. A unified
    // install's RU account gets a separate identity; no keys are copied.
    if region == RegionId::Ru && compiled != shared::market::Market::Ru {
        base.join("regions/ru")
    } else {
        base
    }
}
pub(crate) fn bind_initial(region: RegionId) -> Result<(), String> {
    if let Some(active) = shared::region::active() {
        return if active.id == region {
            Ok(())
        } else {
            Err("Changing account region requires a restart.".into())
        };
    }
    let file = path().ok_or("No writable account settings directory.")?;
    save(
        &file,
        &Selection {
            schema: 1,
            selected: Some(region),
            pending: None,
            account: None,
        },
    )?;
    shared::region::bind(region).map_err(str::to_owned)
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ActivateRegion;
pub(crate) struct AccountRegionPlugin;
impl Plugin for AccountRegionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            open_region_settings.after(crate::ui::UiSet::Dispatch),
        )
        .add_systems(Update, stop_pending_transport.run_if(switch_pending))
        .add_systems(Update, sync_region_label)
        .add_systems(
            Update,
            activate
                .in_set(ActivateRegion)
                .run_if(on_message::<crate::legal::LegalAccepted>),
        );
    }
}

fn open_region_settings(
    mut actions: MessageReader<crate::ui::Activated<crate::pause_menu::PauseAction>>,
    mut consent: ResMut<crate::legal::LegalConsent>,
    session: Res<crate::net::ClientSession>,
    purchase: Option<Res<crate::supporter::SupporterPlatformState>>,
) {
    for event in actions.read() {
        if event.action == crate::pause_menu::PauseAction::AccountRegion
            && selectable()
            && !session.has_committed_join()
            && !purchase
                .as_ref()
                .is_some_and(|p| p.busy || p.pending_receipts)
        {
            consent.open_region_choice();
        }
    }
}
fn sync_region_label(mut labels: Query<&mut Text, With<crate::pause_menu::AccountRegionValue>>) {
    let key = if shared::region::market() == shared::market::Market::Ru {
        "region.ru"
    } else {
        "region.global"
    };
    let label = crate::i18n::tr(key);
    for mut text in &mut labels {
        if text.0 != label {
            text.0 = label.to_owned();
        }
    }
}
fn stop_pending_transport(world: &mut World) {
    crate::net::stop_for_region_switch(world);
}
fn activate(world: &mut World) {
    if shared::region::active().is_none() {
        return;
    }
    crate::connection_profile::install();
    if world.contains_resource::<crate::connection_profile::ActiveConnection>() {
        world.insert_resource(crate::connection_profile::ActiveConnection::current());
    }
    world.insert_resource(crate::team::TeamSelection::default());
    world.insert_resource(crate::supporter::PurchaseOffer::default());
    world.insert_resource(crate::career::protection::ProtectPrompt::load());
    crate::passport::initialize();
    crate::passport::initialize_store();
    crate::passport::poll_account();
    world
        .run_system_once(crate::persistence::load_persistent_client_settings)
        .expect("regional preferences resources");
    world
        .run_system_once(crate::career_identity::load_identity)
        .expect("regional identity resources");
}
pub(crate) fn remember_account(public_key: &str, profile_id: &str) {
    let Some(region) = shared::region::active().map(|p| p.id) else {
        return;
    };
    let Some(file) = path() else {
        return;
    };
    let account = AccountBinding {
        region,
        profile_id: profile_id.into(),
        public_key: public_key.into(),
    };
    let mut state = read(&file).unwrap_or(Selection {
        schema: 1,
        ..default()
    });
    if state.account.as_ref() == Some(&account) {
        return;
    }
    state.selected = Some(region);
    state.account = Some(account);
    if let Err(error) = save(&file, &state) {
        warn!("Account region not saved: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_account_binding_is_bounded_validated_and_private() {
        let root = std::env::temp_dir().join(format!(
            "omoba-region-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join(FILE);
        let value = Selection {
            schema: 1,
            selected: Some(RegionId::Ru),
            pending: None,
            account: Some(AccountBinding {
                region: RegionId::Ru,
                profile_id: "a".repeat(64),
                public_key: "b".repeat(64),
            }),
        };
        save(&path, &value).unwrap();
        assert_eq!(read(&path), Some(value.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let mut invalid = value.clone();
        invalid.selected = Some(RegionId::Global);
        assert!(save(&path, &invalid).is_err());
        invalid = value.clone();
        invalid.account.as_mut().unwrap().public_key = "not-a-key".into();
        assert!(save(&path, &invalid).is_err());
        assert_eq!(read(&path), Some(value));
        std::fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert_eq!(read(&path), None);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn account_region_wins_over_legacy_addresses_and_ambiguous_accounts_require_choice() {
        let mut saved = Selection {
            schema: 1,
            selected: Some(RegionId::Ru),
            ..default()
        };
        assert_eq!(
            choose(Some(&saved), &[RegionId::Global]),
            Some(RegionId::Ru)
        );
        assert_eq!(choose(None, &RegionId::ALL), None);
        assert_eq!(choose(None, &[RegionId::Global]), Some(RegionId::Global));
        saved.pending = Some(RegionId::Global);
        assert_eq!(saved.region(), Some(RegionId::Global));
    }
    #[test]
    fn unified_account_directories_never_share_signing_keys() {
        let base = PathBuf::from("/tmp/account-routing");
        let global = override_directory(
            base.clone(),
            RegionId::Global,
            shared::market::Market::Global,
        );
        let ru = override_directory(base.clone(), RegionId::Ru, shared::market::Market::Global);
        assert_ne!(global, ru);
        assert_eq!(global, base);
        assert_eq!(
            override_directory(base.clone(), RegionId::Ru, shared::market::Market::Ru),
            base
        );
    }
}
