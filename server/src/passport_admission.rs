//! Bounded asynchronous ticket verification, outside the gameplay tick.
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ekza_bevy_sdk::passport::ProtectedAvatar;
use ekza_bevy_sdk::store::StoreAvatar;
use omoba_passport::PassportApi;
use shared::wire::ClientPacket;

const MAX_PENDING: usize = 16;
/// A known free avatar is trusted this long before the registry is asked again,
/// which also bounds how long a withdrawn approval keeps working.
const FREE_TTL: Duration = Duration::from_secs(300);
/// A slug the list does not contain may trigger a new read at most this often, so
/// a freshly approved avatar works within seconds and junk slugs cannot flood the
/// registry.
const FREE_RETRY: Duration = Duration::from_secs(10);

type FetchWeapons = fn(&str) -> Result<Vec<ekza_bevy_sdk::assets::StoreAsset>, String>;

#[derive(Default)]
struct WeaponCatalogue {
    items: HashSet<String>,
    fetched: Option<Instant>,
    attempted: Option<Instant>,
}
impl WeaponCatalogue {
    fn apply(
        &mut self,
        result: Result<Vec<ekza_bevy_sdk::assets::StoreAsset>, String>,
        now: Instant,
    ) {
        self.attempted = Some(now);
        if let Ok(items) = result {
            self.items = items
                .into_iter()
                .filter(|item| {
                    ekza_bevy_sdk::assets::validate_item(
                        item,
                        ekza_bevy_sdk::assets::AssetKind::Weapon,
                        &omoba_passport::weapon_store::selector(),
                    )
                    .is_ok()
                })
                .map(|item| item.slug)
                .collect();
            self.fetched = Some(now);
        }
    }
}

type FetchFree = fn(&str) -> Result<Vec<StoreAvatar>, String>;

/// Free avatars approved for Omoba, as read by THIS server from the registry.
/// Nothing a client sends can add an entry.
#[derive(Default)]
struct FreeCatalogue {
    items: HashMap<String, StoreAvatar>,
    fetched: Option<Instant>,
    attempted: Option<Instant>,
}

enum FreeLookup {
    /// Boxed: a catalogue entry dwarfs the other variants.
    Known(Box<StoreAvatar>),
    Unknown,
    NeedsRead,
}

impl FreeCatalogue {
    fn lookup(&self, slug: &str, now: Instant) -> FreeLookup {
        let recent = |at: Option<Instant>, limit: Duration| {
            at.is_some_and(|at| now.saturating_duration_since(at) < limit)
        };
        match self.items.get(slug) {
            Some(item) if recent(self.fetched, FREE_TTL) => {
                FreeLookup::Known(Box::new(item.clone()))
            }
            _ if recent(self.attempted, FREE_RETRY) => FreeLookup::Unknown,
            _ => FreeLookup::NeedsRead,
        }
    }

    /// Returns the entry for `slug` after applying a read. A failed read keeps what
    /// was known: a registry outage must not lock out avatars already approved.
    fn apply(
        &mut self,
        result: Result<Vec<StoreAvatar>, String>,
        slug: &str,
        now: Instant,
    ) -> Option<StoreAvatar> {
        self.attempted = Some(now);
        if let Ok(items) = result {
            self.items = items
                .into_iter()
                .filter(|item| item.free)
                .map(|item| (item.slug.clone(), item))
                .collect();
            self.fetched = Some(now);
        }
        self.items.get(slug).cloned()
    }
}

pub(super) enum Admission {
    Free,
    Pending,
    Denied,
}

pub(super) struct CompletedAdmission {
    pub addr: SocketAddr,
    pub packet: ClientPacket,
    pub allowed: bool,
    weapon_only: bool,
    resume_session: Option<String>,
    skip_avatar: bool,
}

pub(super) struct PassportAdmissions {
    weapons: Arc<Mutex<WeaponCatalogue>>,
    fetch_weapons: FetchWeapons,
    api: Option<PassportApi>,
    free: Arc<Mutex<FreeCatalogue>>,
    fetch_free: FetchFree,
    registry: String,
    pending: HashSet<SocketAddr>,
    sender: mpsc::Sender<CompletedAdmission>,
    receiver: Mutex<mpsc::Receiver<CompletedAdmission>>,
}

impl Default for PassportAdmissions {
    fn default() -> Self {
        Self::for_market(shared::market::FEATURES.ekza)
    }
}

/// Why a build without Ekza admits no hosted item; no request is made.
const NO_EKZA: &str = "this build has no Ekza services";

fn no_weapons(_: &str) -> Result<Vec<ekza_bevy_sdk::assets::StoreAsset>, String> {
    Err(NO_EKZA.into())
}

fn no_free_avatars(_: &str) -> Result<Vec<StoreAvatar>, String> {
    Err(NO_EKZA.into())
}

impl PassportAdmissions {
    /// The admissions of a build with `ekza`. Without Ekza there is no passport
    /// and every registry read fails at once, without a request: a protected
    /// avatar or `ekza-` weapon is denied, shipped content stays free.
    fn for_market(ekza: bool) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            weapons: Arc::default(),
            fetch_weapons: if ekza {
                omoba_passport::weapon_store::fetch_approved
            } else {
                no_weapons
            },
            api: if ekza {
                PassportApi::from_env().ok()
            } else {
                None
            },
            free: Arc::default(),
            fetch_free: if ekza {
                omoba_passport::community::fetch_free
            } else {
                no_free_avatars
            },
            registry: if ekza {
                omoba_passport::community::registry_url()
            } else {
                String::new()
            },
            pending: HashSet::new(),
            sender,
            receiver: Mutex::new(receiver),
        }
    }
}

impl PassportAdmissions {
    pub fn is_pending(&self, addr: SocketAddr) -> bool {
        self.pending.contains(&addr)
    }

    pub fn begin(&mut self, addr: SocketAddr, packet: &ClientPacket) -> Admission {
        self.begin_with_session(addr, packet, None)
    }

    pub fn begin_with_session(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        session: Option<&str>,
    ) -> Admission {
        self.begin_loadout(addr, packet, session, false)
    }

    pub fn begin_loadout(
        &mut self,
        addr: SocketAddr,
        packet: &ClientPacket,
        session: Option<&str>,
        skip_avatar: bool,
    ) -> Admission {
        let handheld = match packet {
            ClientPacket::Join { handheld, .. } => Some(handheld),
            ClientPacket::Prematch {
                request:
                    shared::prematch::PrematchRequest {
                        action: shared::prematch::PrematchAction::Select { handheld, .. },
                        ..
                    },
            } => Some(handheld),
            _ => None,
        };
        if let Some(shared::handheld::HandheldSelection::Item(id)) = handheld {
            // Only shipped props bypass hosted approval. Operator imports are
            // useful for local sandbox rehearsal but do not grant online use.
            let shipped = omoba_passport::weapons::Catalog::parse(include_bytes!(
                "../../client/assets/weapons/manifest.json"
            ))
            .expect("shipped props");
            if !shipped.items.iter().any(|item| item.id == *id) {
                if !is_ekza_weapon_slug(id) {
                    return Admission::Denied;
                }
                let cache = self.weapons.lock().unwrap();
                let fresh = cache.fetched.is_some_and(|at| at.elapsed() < FREE_TTL);
                let known = cache.items.contains(id);
                if !(known
                    && (fresh || cache.attempted.is_some_and(|at| at.elapsed() < FREE_RETRY)))
                {
                    if cache.attempted.is_some_and(|at| at.elapsed() < FREE_RETRY) {
                        return Admission::Denied;
                    }
                    drop(cache);
                    if self.pending.contains(&addr) {
                        return Admission::Pending;
                    }
                    if self.pending.len() >= MAX_PENDING {
                        return Admission::Denied;
                    }
                    self.pending.insert(addr);
                    let (cache, fetch, registry, sender, id, packet, resume_session) = (
                        self.weapons.clone(),
                        self.fetch_weapons,
                        self.registry.clone(),
                        self.sender.clone(),
                        id.clone(),
                        packet.clone(),
                        session.map(str::to_owned),
                    );
                    std::thread::spawn(move || {
                        let result = fetch(&registry);
                        let allowed = {
                            let mut cache = cache.lock().unwrap();
                            cache.apply(result, Instant::now());
                            cache.items.contains(&id)
                        };
                        let _ = sender.send(CompletedAdmission {
                            addr,
                            packet,
                            allowed,
                            weapon_only: true,
                            resume_session,
                            skip_avatar,
                        });
                    });
                    return Admission::Pending;
                }
            }
        }
        if skip_avatar {
            return Admission::Free;
        }
        let draft_session = session.map(str::to_owned);
        let (avatar, passport_ticket, session_id) = match packet {
            ClientPacket::Join {
                avatar,
                passport_ticket,
                session_id,
                ..
            } => (avatar, passport_ticket, session_id),
            ClientPacket::Prematch {
                request:
                    shared::prematch::PrematchRequest {
                        action:
                            shared::prematch::PrematchAction::Select {
                                avatar,
                                passport_ticket,
                                ..
                            },
                        ..
                    },
            } => (avatar, passport_ticket, &draft_session),
            _ => return Admission::Free,
        };
        let Some(slug) = avatar.as_deref() else {
            return Admission::Free;
        };
        // Gameplay normalizes surrounding whitespace before assigning a model.
        // The gate must inspect the same slug to prevent a padded paid bypass.
        let slug = slug.trim();
        // A free community avatar needs no ticket, but only this server's own read
        // of the registry can say an avatar is free.
        if passport_ticket.is_none() && ekza_bevy_sdk::passport::is_protected_slug(slug) {
            let lookup = self.free.lock().unwrap().lookup(slug, Instant::now());
            match lookup {
                FreeLookup::Known(item) => {
                    return if register_free(&item) {
                        Admission::Free
                    } else {
                        Admission::Denied
                    };
                }
                FreeLookup::Unknown => {}
                FreeLookup::NeedsRead => {
                    if self.pending.contains(&addr) {
                        return Admission::Pending;
                    }
                    if self.pending.len() >= MAX_PENDING {
                        return Admission::Denied;
                    }
                    self.pending.insert(addr);
                    let (free, fetch, registry) =
                        (self.free.clone(), self.fetch_free, self.registry.clone());
                    let (slug, packet, sender) =
                        (slug.to_owned(), packet.clone(), self.sender.clone());
                    std::thread::spawn(move || {
                        let result = fetch(&registry);
                        let item = free.lock().unwrap().apply(result, &slug, Instant::now());
                        let allowed = item.is_some_and(|item| register_free(&item));
                        let _ = sender.send(CompletedAdmission {
                            addr,
                            packet,
                            allowed,
                            weapon_only: false,
                            resume_session: None,
                            skip_avatar: false,
                        });
                    });
                    return Admission::Pending;
                }
            }
        }
        let definition = omoba_passport::avatars::avatar_definition(slug);
        // A shipped or already registered entry pins the exact rendition. A
        // store slug the server has never seen is still admissible: the
        // passport's consumed ticket names the rendition, and the slug is a
        // hash of exactly that, so no pre-synced manifest is needed.
        let expected = definition.and_then(|entry| entry.passport.clone());
        if expected.is_none()
            && (definition.is_some() || !ekza_bevy_sdk::passport::is_protected_slug(slug))
        {
            return if slug.starts_with("ekza-") && definition.is_none() {
                Admission::Denied
            } else {
                Admission::Free
            };
        }
        if self.pending.contains(&addr) {
            return Admission::Pending;
        }
        let (Some(api), Some(ticket), Some(session)) = (
            self.api.clone(),
            passport_ticket.clone(),
            session_id.clone(),
        ) else {
            return Admission::Denied;
        };
        if self.pending.len() >= MAX_PENDING
            || !omoba_passport::valid_session_id(&session)
            || ticket.len() < 16
            || ticket.len() > 4096
            || expected.as_ref().is_some_and(|protected| {
                protected.validate().is_err() || omoba_passport::protected_slug(protected) != slug
            })
        {
            return Admission::Denied;
        }
        self.pending.insert(addr);
        let slug = slug.to_owned();
        let packet = packet.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let allowed = verify_admission(
                &slug,
                expected.as_ref(),
                &session,
                &ticket,
                |ticket, session| api.consume(ticket, session),
            )
            .is_ok_and(|granted| register(&slug, granted));
            let _ = sender.send(CompletedAdmission {
                addr,
                packet,
                allowed,
                weapon_only: false,
                resume_session: None,
                skip_avatar: false,
            });
        });
        Admission::Pending
    }

    pub fn completed(&mut self) -> Vec<CompletedAdmission> {
        let responses: Vec<_> = self.receiver.lock().unwrap().try_iter().collect();
        let mut completed = Vec::new();
        for mut response in responses {
            self.pending.remove(&response.addr);
            if response.weapon_only && response.allowed {
                // Complete the same immutable request through avatar admission;
                // a weapon approval must never bypass the avatar's own gate.
                match self.begin_loadout(
                    response.addr,
                    &response.packet,
                    response.resume_session.as_deref(),
                    response.skip_avatar,
                ) {
                    Admission::Free => {}
                    Admission::Denied => response.allowed = false,
                    Admission::Pending => continue,
                }
            }
            completed.push(response);
        }
        completed
    }
}

/// The store slug shape of a hosted handheld (`ekza-` and 32 hex digits).
fn is_ekza_weapon_slug(id: &str) -> bool {
    id.len() == 37 && id.starts_with("ekza-") && id[5..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// The operator line for a loadout decision that names Ekza items: a hosted
/// handheld and/or a store avatar (Studio or passport). Shipped props, roster
/// avatars and malformed ids are not echoed, and tickets and session ids are
/// never part of it. `None` when the loadout names no Ekza item.
pub(crate) fn ekza_decision_line(
    player: Option<u64>,
    packet: &ClientPacket,
    allowed: bool,
) -> Option<String> {
    use shared::prematch::{PrematchAction, PrematchRequest};
    let (handheld, avatar) = match packet {
        ClientPacket::Join {
            handheld, avatar, ..
        }
        | ClientPacket::Prematch {
            request:
                PrematchRequest {
                    action:
                        PrematchAction::Select {
                            handheld, avatar, ..
                        },
                    ..
                },
        } => (handheld, avatar),
        _ => return None,
    };
    let handheld = match handheld {
        shared::handheld::HandheldSelection::Item(id) if is_ekza_weapon_slug(id) => Some(id),
        _ => None,
    };
    let avatar = avatar
        .as_deref()
        .map(str::trim)
        .filter(|slug| ekza_bevy_sdk::passport::is_protected_slug(slug));
    if handheld.is_none() && avatar.is_none() {
        return None;
    }
    Some(format!(
        "Ekza loadout {} for player {}: handheld {}, avatar {}",
        if allowed { "admitted" } else { "denied" },
        player.map_or_else(|| "?".to_owned(), |id| id.to_string()),
        handheld.map_or("-", String::as_str),
        avatar.unwrap_or("-"),
    ))
}

/// Print [`ekza_decision_line`], if any.
pub(crate) fn log_ekza_decision(player: Option<u64>, packet: &ClientPacket, allowed: bool) {
    if let Some(line) = ekza_decision_line(player, packet, allowed) {
        println!("{line}");
    }
}

/// Make the granted avatar visible to slug normalization and to the clients
/// that will render it. Roster entries are already known.
fn register(slug: &str, granted: ProtectedAvatar) -> bool {
    if omoba_passport::avatars::avatar_definition(slug).is_some() {
        return true;
    }
    omoba_passport::avatars::register_store_avatar(omoba_passport::avatars::AvatarDefinition {
        slug: slug.to_owned(),
        display_name: "Ekza avatar".into(),
        collection: "Ekza store".into(),
        license: "See creator terms".into(),
        source_url: granted.support.rendition.url.clone(),
        author: None,
        attribution: None,
        thumbnail: None,
        passport: Some(granted),
        free: false,
    })
    .is_some()
}

/// Make a free avatar known to slug normalization and to the clients that render
/// it. The item comes from this server's registry read and is re-validated here.
fn register_free(item: &StoreAvatar) -> bool {
    if !item.free || ekza_bevy_sdk::store::validate_item(item, &omoba_passport::selector()).is_err()
    {
        return false;
    }
    if let Some(existing) = omoba_passport::avatars::avatar_definition(&item.slug) {
        return existing.passport.as_ref() == Some(&item.protected);
    }
    omoba_passport::avatars::register_store_avatar(omoba_passport::avatars::AvatarDefinition {
        slug: item.slug.clone(),
        display_name: item.name.clone(),
        collection: "Ekza community".into(),
        license: item
            .license
            .clone()
            .unwrap_or_else(|| "See creator terms".into()),
        source_url: item.protected.support.rendition.url.clone(),
        author: item.author.clone(),
        attribution: None,
        thumbnail: None,
        passport: Some(item.protected.clone()),
        free: true,
    })
    .is_some()
}

/// Consume the ticket at the configured origin and return the exact rendition
/// it grants. The slug the client asked for must be the hash of that grant,
/// and must equal the pinned roster entry when one exists.
fn verify_admission(
    slug: &str,
    expected: Option<&ProtectedAvatar>,
    session: &str,
    ticket: &str,
    consume: impl FnOnce(&str, &str) -> Result<ekza_bevy_sdk::passport::ConsumedTicket, String>,
) -> Result<ProtectedAvatar, String> {
    if let Some(expected) = expected {
        expected.validate()?;
    }
    if !omoba_passport::valid_session_id(session) || ticket.len() < 16 {
        return Err("Invalid admission proof".into());
    }
    let response = consume(ticket, session)?;
    let granted = ProtectedAvatar {
        avatar_id: response.avatar_id.clone(),
        support: response.support.clone(),
    };
    // Validates the Omoba approval, identity, wallet and mint shapes.
    granted.validate_consumed_ticket(&response)?;
    if expected.is_some_and(|expected| expected != &granted)
        || omoba_passport::protected_slug(&granted) != slug
    {
        return Err("Ticket does not grant this exact approved avatar rendition".into());
    }
    Ok(granted)
}

#[cfg(test)]
mod tests {
    use shared::wire::ClientPacket;
    use std::net::{SocketAddr, UdpSocket};
    use std::time::{Duration, Instant};

    use shared::HeroClass;
    use shared::map::Team;
    use shared::wire::CharacterChoice;

    use super::*;
    use crate::match_rules::MatchConfig;
    use crate::prematch;
    use crate::runtime::ServerRuntime;
    use ekza_bevy_sdk::passport::{ConsumedTicket, ProjectSupport, Rendition};

    fn fixture() -> (ProtectedAvatar, ConsumedTicket) {
        let expected = ProtectedAvatar {
            avatar_id: format!("solana:devnet:avatar-data:{}", "1".repeat(32)),
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: "humanoid-glb-v1".into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: "r1".into(),
                    url: "https://example.test/a.glb".into(),
                    sha256: "a".repeat(64),
                    size_bytes: 20,
                    format: "glb".into(),
                },
            },
        };
        let response = ConsumedTicket {
            wallet: "2".repeat(32),
            mint: "3".repeat(32),
            avatar_id: expected.avatar_id.clone(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            support: expected.support.clone(),
        };
        (expected, response)
    }

    fn store_item(id: &str, sha: char, free: bool) -> StoreAvatar {
        let (mut protected, _) = fixture();
        protected.avatar_id = id.into();
        protected.support.rendition.sha256 = sha.to_string().repeat(64);
        StoreAvatar {
            slug: omoba_passport::protected_slug(&protected),
            name: "Community Robert".into(),
            author: Some("alice".into()),
            license: Some("CC0".into()),
            attribution: None,
            thumbnail_url: None,
            protected,
            free,
        }
    }
    const STUDIO_FREE: &str = "ekza:avatar:2f0c1f0e-7b1a-4c55-9d53-0a6d3c1b9e77";
    const STUDIO_OTHER: &str = "ekza:avatar:9b1c1f0e-7b1a-4c55-9d53-0a6d3c1b9e11";

    fn feed(_: &str) -> Result<Vec<StoreAvatar>, String> {
        let chain = format!("solana:devnet:avatar-data:{}", "4".repeat(32));
        Ok(vec![
            store_item(STUDIO_FREE, 'c', true),
            store_item(&chain, 'd', false),
        ])
    }
    fn outage(_: &str) -> Result<Vec<StoreAvatar>, String> {
        Err("registry unreachable".into())
    }
    fn admissions(fetch_free: FetchFree) -> PassportAdmissions {
        PassportAdmissions {
            api: None,
            fetch_free,
            registry: "https://registry.test".into(),
            ..Default::default()
        }
    }
    fn join(avatar: &str) -> ClientPacket {
        ClientPacket::Join {
            handheld: Default::default(),
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Mage,
            avatar: Some(avatar.into()),
            sprite_character: None,
            session_id: Some("session-1".into()),
            passport_ticket: None,
        }
    }
    fn settle(admissions: &mut PassportAdmissions) -> Vec<CompletedAdmission> {
        for _ in 0..200 {
            let done = admissions.completed();
            if !done.is_empty() {
                return done;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the registry read never completed");
    }

    #[test]
    fn draft_selection_uses_the_same_server_catalogue_and_rejected_paid_choice_keeps_loadout() {
        use shared::prematch::{PrematchAction, PrematchRequest, Role};
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
        rt.passport_admissions = admissions(feed);
        let addr: SocketAddr = "127.0.0.1:59901".parse().unwrap();
        let now = Instant::now();
        rt.handle_packet(
            addr,
            ClientPacket::Join {
                handheld: Default::default(),
                prematch: true,
                team: Team::Blue,
                character: CharacterChoice::Ipfs,
                hero_class: HeroClass::Mage,
                avatar: None,
                sprite_character: None,
                session_id: Some("draft-free".into()),
                passport_ticket: None,
            },
            now,
        );
        let generation = prematch::snapshot(
            &rt.prematch,
            &rt.world.players,
            &rt.world.players[&addr],
            rt.rules,
            now,
        )
        .unwrap()
        .generation;
        let free = store_item(STUDIO_FREE, 'c', true);
        let owned = store_item(
            &format!("solana:devnet:avatar-data:{}", "4".repeat(32)),
            'd',
            false,
        );
        let action = |slug: String| PrematchAction::Select {
            handheld: Default::default(),
            character: CharacterChoice::Ipfs,
            hero_class: HeroClass::Cleric,
            avatar: Some(slug),
            sprite_character: None,
            role: Role::Support,
            passport_ticket: None,
        };
        let request = |id, action| PrematchRequest {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            generation,
            request_id: id,
            action,
        };
        let approved_request = request(1, action(free.slug.clone()));
        let mut denied_request = request(2, action(owned.slug));
        if let PrematchAction::Select {
            hero_class,
            handheld,
            ..
        } = &mut denied_request.action
        {
            *hero_class = HeroClass::Warrior;
            *handheld = shared::handheld::HandheldSelection::Unequipped;
        }
        rt.handle_packet(
            addr,
            ClientPacket::Prematch {
                request: approved_request.clone(),
            },
            now,
        );
        // Receipt alone must not look like a final approval to the frontend.
        assert_eq!(
            prematch::snapshot(
                &rt.prematch,
                &rt.world.players,
                &rt.world.players[&addr],
                rt.rules,
                now
            )
            .unwrap()
            .last_request_id,
            0
        );
        rt.handle_packet(
            addr,
            ClientPacket::Prematch {
                request: approved_request,
            },
            now,
        );
        assert_eq!(
            prematch::snapshot(
                &rt.prematch,
                &rt.world.players,
                &rt.world.players[&addr],
                rt.rules,
                now
            )
            .unwrap()
            .last_request_id,
            0
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while rt.world.players[&addr].hero.identity.avatar.as_deref() != Some(free.slug.as_str())
            && Instant::now() < deadline
        {
            rt.receive_packets();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            rt.world.players[&addr].hero.identity.avatar.as_deref(),
            Some(free.slug.as_str())
        );
        assert_eq!(
            rt.world.players[&addr].hero.identity.hero_class,
            HeroClass::Cleric
        );
        assert_eq!(
            prematch::snapshot(
                &rt.prematch,
                &rt.world.players,
                &rt.world.players[&addr],
                rt.rules,
                now
            )
            .unwrap()
            .last_request_id,
            1
        );
        rt.handle_packet(
            addr,
            ClientPacket::Prematch {
                request: denied_request,
            },
            Instant::now(),
        );
        assert_eq!(
            rt.world.players[&addr].hero.identity.avatar.as_deref(),
            Some(free.slug.as_str())
        );
        let retained = &rt.world.players[&addr];
        assert_eq!(retained.hero.identity.hero_class, HeroClass::Cleric);
        assert_eq!(
            retained.hero.identity.handheld,
            shared::handheld::HandheldSelection::ClassDefault
        );
        assert_eq!(retained.draft.acknowledged_request_id, 2);
        assert!(!rt.passport_admissions.is_pending(addr));
        assert_eq!(
            retained.draft.error.as_deref(),
            Some(
                "This avatar or weapon is unavailable. Refresh Studio and choose approved equipment."
            )
        );
    }

    #[test]
    fn a_free_approved_avatar_is_admitted_with_no_ticket_and_nothing_else_is() {
        let addr: SocketAddr = "127.0.0.1:4100".parse().unwrap();
        let free = store_item(STUDIO_FREE, 'c', true);
        let owned = store_item(
            &format!("solana:devnet:avatar-data:{}", "4".repeat(32)),
            'd',
            false,
        );
        let absent = store_item(STUDIO_OTHER, 'e', true);

        let mut gate = admissions(feed);
        // First sight: the server reads the registry itself, off the game tick.
        assert!(matches!(
            gate.begin(addr, &join(&free.slug)),
            Admission::Pending
        ));
        // A repeat while the read is in flight stays pending; on a slow runner
        // the reader thread may already have finished, which is the same
        // answer the settled path gives below.
        assert!(matches!(
            gate.begin(addr, &join(&free.slug)),
            Admission::Pending | Admission::Free
        ));
        let done = settle(&mut gate);
        assert!(done.len() == 1 && done[0].allowed && done[0].addr == addr);
        // Now it is known: immediate, even with surrounding whitespace.
        assert!(matches!(
            gate.begin(addr, &join(&format!(" {} ", free.slug))),
            Admission::Free
        ));
        let entry = omoba_passport::avatars::avatar_definition(&free.slug)
            .expect("registered for rendering");
        assert!(entry.free && entry.passport.as_ref() == Some(&free.protected));
        assert_eq!(
            omoba_passport::avatars::normalize_avatar_slug(Some(&free.slug)),
            Some(free.slug.as_str())
        );

        // The same feed lists an owned template: no ticket, no entry.
        assert!(matches!(
            gate.begin(addr, &join(&owned.slug)),
            Admission::Denied
        ));
        assert!(omoba_passport::avatars::avatar_definition(&owned.slug).is_none());
        // A well-formed slug the registry never listed: a client cannot declare it free.
        assert!(matches!(
            gate.begin(addr, &join(&absent.slug)),
            Admission::Denied
        ));
        assert!(omoba_passport::avatars::avatar_definition(&absent.slug).is_none());
    }

    #[test]
    fn unknown_slugs_cannot_flood_the_registry_and_an_outage_keeps_known_avatars() {
        let now = Instant::now();
        let free = store_item(STUDIO_FREE, 'c', true);
        let mut list = FreeCatalogue::default();
        assert!(matches!(
            list.lookup(&free.slug, now),
            FreeLookup::NeedsRead
        ));
        assert!(list.apply(feed(""), "ekza-unknown", now).is_none());
        assert_eq!(
            list.items.len(),
            1,
            "owned entries are dropped from the free list"
        );
        // A miss right after a read does not read again; a hit is served from memory.
        assert!(matches!(
            list.lookup("ekza-unknown", now + Duration::from_secs(1)),
            FreeLookup::Unknown
        ));
        assert!(matches!(
            list.lookup(&free.slug, now + Duration::from_secs(1)),
            FreeLookup::Known(_)
        ));
        // After the retry window a miss may ask again (a just-approved avatar).
        assert!(matches!(
            list.lookup("ekza-unknown", now + FREE_RETRY),
            FreeLookup::NeedsRead
        ));
        // A hit expires too, so a withdrawn approval stops working within the TTL.
        assert!(matches!(
            list.lookup(&free.slug, now + FREE_TTL),
            FreeLookup::NeedsRead
        ));
        // Registry outage: what was approved stays admissible, nothing new appears.
        let later = now + FREE_TTL;
        assert_eq!(
            list.apply(outage(""), &free.slug, later)
                .map(|item| item.slug),
            Some(free.slug.clone())
        );
        assert!(list.apply(outage(""), "ekza-unknown", later).is_none());
        // A successful read that no longer lists it removes it.
        assert!(list.apply(Ok(Vec::new()), &free.slug, later).is_none());
    }

    #[test]
    fn a_forged_free_item_is_never_registered() {
        let mut forged = store_item(STUDIO_FREE, 'f', true);
        forged.slug = format!("ekza-{}", "0".repeat(64)); // not the hash of its boundary
        assert!(!register_free(&forged));
        let mut not_free = store_item(STUDIO_FREE, 'f', true);
        not_free.free = false;
        assert!(!register_free(&not_free));
        let mut other_game = store_item(STUDIO_FREE, 'f', true);
        other_game.protected.support.project_id = "ekza-space".into();
        other_game.slug = omoba_passport::protected_slug(&other_game.protected);
        assert!(!register_free(&other_game));
    }

    #[test]
    fn owner_is_admitted_only_for_exact_rendition() {
        let (expected, response) = fixture();
        let slug = omoba_passport::protected_slug(&expected);
        assert!(
            verify_admission(
                &slug,
                Some(&expected),
                "session-1",
                "scoped-proof-123456",
                |ticket, session| {
                    assert_eq!(ticket, "scoped-proof-123456");
                    assert_eq!(session, "session-1");
                    Ok(response.clone())
                }
            )
            .is_ok()
        );
        let mut wrong = response.clone();
        wrong.support.rendition.sha256 = "b".repeat(64);
        assert!(
            verify_admission(
                &slug,
                Some(&expected),
                "session-1",
                "scoped-proof-123456",
                |_, _| Ok(wrong)
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_store_slug_is_admitted_only_by_a_ticket_for_that_exact_rendition() {
        let (expected, response) = fixture();
        let slug = omoba_passport::protected_slug(&expected);
        // No roster entry: the consumed ticket alone names the rendition.
        let granted = verify_admission(&slug, None, "session-1", "scoped-proof-123456", |_, _| {
            Ok(response.clone())
        })
        .unwrap();
        assert_eq!(granted, expected);
        assert!(register(&slug, granted));
        assert_eq!(
            omoba_passport::avatars::normalize_avatar_slug(Some(&slug)),
            Some(slug.as_str())
        );

        // A valid ticket for some other owned avatar cannot unlock this slug.
        let mut other = response.clone();
        other.avatar_id = format!("solana:devnet:avatar-data:{}", "4".repeat(32));
        other.support.rendition.sha256 = "c".repeat(64);
        assert!(
            verify_admission(&slug, None, "session-1", "scoped-proof-123456", |_, _| Ok(
                other
            ))
            .is_err()
        );
        // Nor can a ticket approved for another project or profile.
        let mut foreign = response;
        foreign.support.project_id = "ekza-space".into();
        assert!(
            verify_admission(&slug, None, "session-1", "scoped-proof-123456", |_, _| Ok(
                foreign
            ))
            .is_err()
        );
    }

    #[test]
    fn nonowner_expired_replay_and_wrong_session_fail_closed() {
        let (expected, _) = fixture();
        let slug = omoba_passport::protected_slug(&expected);
        for error in [
            "non-owner",
            "expired",
            "replayed",
            "wrong session",
            "wrong project",
            "offline",
        ] {
            assert!(
                verify_admission(
                    &slug,
                    Some(&expected),
                    "session-1",
                    "scoped-proof-123456",
                    |_, _| Err(error.into())
                )
                .is_err()
            );
        }
        assert!(
            verify_admission(&slug, None, "session-1", "", |_, _| panic!(
                "missing proof must not contact API"
            ))
            .is_err()
        );
    }

    #[test]
    fn forged_paid_slug_cannot_bypass_gate_without_registry_entry() {
        // Injected registry read: the gate must never touch the network in a test.
        let mut gate = admissions(feed);
        let addr = "127.0.0.1:12345".parse().unwrap();
        let paid: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":"ekza-forged", "session_id":"session-1"
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &paid), Admission::Denied));
        // A well-formed store slug without a ticket never reaches the passport. The
        // server checks its own free list once, finds nothing, and refuses; a repeat
        // inside the retry window is refused without another read.
        let ticketless: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":format!("ekza-{}", "a".repeat(64)),
            "session_id":"session-1"
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &ticketless), Admission::Pending));
        let done = settle(&mut gate);
        assert!(done.len() == 1 && !done[0].allowed);
        assert!(matches!(gate.begin(addr, &ticketless), Admission::Denied));
        assert!(
            omoba_passport::avatars::avatar_definition(&format!("ekza-{}", "a".repeat(64)))
                .is_none()
        );
        let free: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":omoba_passport::avatars::avatar_roster()[0].slug
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &free), Admission::Free));
    }
}

#[cfg(test)]
mod weapon_tests {
    use super::*;
    use ekza_bevy_sdk::{
        assets::{AssetKind, StoreAsset, asset_slug},
        passport::{ProjectSupport, Rendition},
    };
    use shared::{HeroClass, handheld::HandheldSelection, map::Team, wire::CharacterChoice};
    fn item() -> StoreAsset {
        let id = "ekza:weapon:1c366765-3f44-4d36-b14c-acaba3bfce53".to_string();
        let hash = "b".repeat(64);
        StoreAsset {
            slug: asset_slug(AssetKind::Weapon, &id, &hash),
            asset_id: id,
            asset_kind: AssetKind::Weapon,
            name: "Approved hammer".into(),
            author: None,
            license: None,
            attribution: None,
            thumbnail_url: None,
            free: true,
            support: ProjectSupport {
                project_id: "omoba".into(),
                platform: "desktop".into(),
                profile: omoba_passport::weapons::PROFILE.into(),
                status: "approved".into(),
                rendition: Rendition {
                    id: "weapon-r1".into(),
                    url: "https://example.test/hammer.glb".into(),
                    sha256: hash,
                    size_bytes: 128,
                    format: "glb".into(),
                },
            },
        }
    }
    fn approved(_: &str) -> Result<Vec<StoreAsset>, String> {
        Ok(vec![item()])
    }
    fn join(weapon: &str, avatar: Option<&str>) -> ClientPacket {
        ClientPacket::Join {
            handheld: HandheldSelection::Item(weapon.into()),
            prematch: false,
            team: Team::Green,
            character: CharacterChoice::default(),
            hero_class: HeroClass::Warrior,
            avatar: avatar.map(str::to_owned),
            sprite_character: None,
            session_id: Some("weapon-test".into()),
            passport_ticket: None,
        }
    }
    fn settle(gate: &mut PassportAdmissions) -> CompletedAdmission {
        for _ in 0..200 {
            if let Some(result) = gate.completed().pop() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("weapon admission did not finish")
    }
    #[test]
    fn a_build_without_ekza_denies_every_hosted_item_and_keeps_shipped_content_free() {
        let mut gate = PassportAdmissions::for_market(false);
        assert!(gate.api.is_none() && gate.registry.is_empty());
        // The fetchers fail at once: no registry is asked.
        assert_eq!(
            (gate.fetch_weapons)("https://registry.ekza.io")
                .err()
                .as_deref(),
            Some(NO_EKZA)
        );
        assert!((gate.fetch_free)("https://registry.ekza.io").is_err());
        let addr = |port: u16| SocketAddr::from(([127, 0, 0, 1], port));
        // Shipped props and the default avatar need nothing.
        assert!(matches!(
            gate.begin(addr(50201), &join("forge-sword", None)),
            Admission::Free
        ));
        // A hosted weapon: its (failing) read denies it, then the retry window does.
        assert!(matches!(
            gate.begin(addr(50202), &join(&item().slug, None)),
            Admission::Pending
        ));
        assert!(!settle(&mut gate).allowed);
        assert!(matches!(
            gate.begin(addr(50203), &join(&item().slug, None)),
            Admission::Denied
        ));
        // A protected avatar without a ticket: the free list is unavailable.
        let protected = format!("ekza-{}", "a".repeat(64));
        assert!(matches!(
            gate.begin(addr(50204), &join("forge-sword", Some(&protected))),
            Admission::Pending
        ));
        assert!(!settle(&mut gate).allowed);
        // With a ticket: there is no passport to consume it at.
        let mut ticketed = join("forge-sword", Some(&protected));
        if let ClientPacket::Join {
            passport_ticket, ..
        } = &mut ticketed
        {
            *passport_ticket = Some("t".repeat(32));
        }
        assert!(matches!(
            gate.begin(addr(50205), &ticketed),
            Admission::Denied
        ));
        // An Ekza build reads its registry.
        assert!(!PassportAdmissions::for_market(true).registry.is_empty());
    }

    #[test]
    fn approved_weapon_is_independently_admitted_but_cannot_bypass_avatar_gate() {
        let mut gate = PassportAdmissions {
            api: None,
            fetch_weapons: approved,
            fetch_free: |_| Ok(vec![]),
            ..Default::default()
        };
        let addr = "127.0.0.1:50111".parse().unwrap();
        assert!(matches!(
            gate.begin(addr, &join(&item().slug, None)),
            Admission::Pending
        ));
        let admitted = settle(&mut gate);
        assert!(admitted.allowed);
        assert!(
            matches!(admitted.packet,ClientPacket::Join{handheld:HandheldSelection::Item(ref id),..} if id==&item().slug)
        );
        assert!(matches!(
            gate.begin(
                addr,
                &join(&item().slug, Some(&format!("ekza-{}", "a".repeat(64))))
            ),
            Admission::Pending
        ));
        assert!(
            !settle(&mut gate).allowed,
            "weapon approval must not authorize an unknown avatar"
        );
        assert!(matches!(
            gate.begin(addr, &join(&format!("ekza-{}", "c".repeat(32)), None)),
            Admission::Denied
        ));
    }
    #[test]
    fn ekza_decisions_log_slugs_and_player_but_never_secrets_or_junk() {
        let avatar = format!("ekza-{}", "a".repeat(64));
        let mut packet = join(&item().slug, Some(&format!(" {avatar} ")));
        if let ClientPacket::Join {
            passport_ticket, ..
        } = &mut packet
        {
            *passport_ticket = Some("ticket-secret-0123456789".into());
        }
        let line = ekza_decision_line(Some(7), &packet, true).unwrap();
        assert_eq!(
            line,
            format!(
                "Ekza loadout admitted for player 7: handheld {}, avatar {avatar}",
                item().slug
            )
        );
        assert!(!line.contains("ticket-secret") && !line.contains("weapon-test"));
        assert_eq!(
            ekza_decision_line(None, &join(&item().slug, None), false).unwrap(),
            format!(
                "Ekza loadout denied for player ?: handheld {}, avatar -",
                item().slug
            )
        );
        // A draft selection is reported the same way.
        let select = ClientPacket::Prematch {
            request: shared::prematch::PrematchRequest {
                server_epoch: 1,
                match_id: 1,
                generation: 1,
                request_id: 1,
                action: shared::prematch::PrematchAction::Select {
                    handheld: HandheldSelection::ClassDefault,
                    character: CharacterChoice::default(),
                    hero_class: HeroClass::Mage,
                    avatar: Some(avatar.clone()),
                    sprite_character: None,
                    role: shared::prematch::Role::Support,
                    passport_ticket: None,
                },
            },
        };
        assert_eq!(
            ekza_decision_line(Some(3), &select, false).unwrap(),
            format!("Ekza loadout denied for player 3: handheld -, avatar {avatar}")
        );
        // Shipped props, roster avatars and malformed ids are not Ekza items.
        for (weapon, avatar) in [
            ("dagger", Some("agnes")),
            ("ekza-not-hex\nforged line", Some("ekza-NOT-A-SLUG\nforged")),
            ("ekza-0123", None),
        ] {
            assert_eq!(
                ekza_decision_line(Some(7), &join(weapon, avatar), true),
                None
            );
        }
        assert_eq!(
            ekza_decision_line(Some(7), &ClientPacket::Leave, true),
            None
        );
    }
    #[test]
    fn healthy_empty_revokes_future_weapon_use_and_outage_preserves_previous_catalogue() {
        let now = Instant::now();
        let mut cache = WeaponCatalogue::default();
        cache.apply(approved(""), now);
        assert!(cache.items.contains(&item().slug));
        cache.apply(Err("offline".into()), now);
        assert!(cache.items.contains(&item().slug));
        cache.apply(Ok(vec![]), now);
        assert!(!cache.items.contains(&item().slug));
        let mut gate = PassportAdmissions::default();
        *gate.weapons.lock().unwrap() = cache;
        assert!(matches!(
            gate.begin(
                "127.0.0.1:50112".parse().unwrap(),
                &join(&item().slug, None)
            ),
            Admission::Denied
        ));
    }
    #[test]
    fn ordinary_join_replicates_weapon_and_draft_rejection_preserves_last_equipment() {
        use crate::{match_rules::MatchConfig, prematch, runtime::ServerRuntime};
        use shared::prematch::{PrematchAction, PrematchRequest, Role};
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
        rt.passport_admissions.fetch_weapons = approved;
        let owner: SocketAddr = "127.0.0.1:50211".parse().unwrap();
        let peer: SocketAddr = "127.0.0.1:50212".parse().unwrap();
        let now = Instant::now();
        let mut packet = join(&item().slug, None);
        if let ClientPacket::Join { prematch, .. } = &mut packet {
            *prematch = true
        }
        rt.handle_packet(owner, packet, now);
        for _ in 0..200 {
            rt.receive_packets();
            if rt.world.players[&owner].joined {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(rt.world.players[&owner].joined);
        assert_eq!(
            rt.world.players[&owner].hero.identity.handheld,
            HandheldSelection::Item(item().slug)
        );
        let mut peer_join = join("forge-hammer", None);
        if let ClientPacket::Join {
            session_id,
            prematch,
            ..
        } = &mut peer_join
        {
            *session_id = Some("weapon-peer".into());
            *prematch = true
        }
        rt.handle_packet(peer, peer_join, now);
        let view = prematch::snapshot(
            &rt.prematch,
            &rt.world.players,
            &rt.world.players[&peer],
            rt.rules,
            now,
        )
        .unwrap();
        assert_eq!(
            view.players
                .iter()
                .find(|p| p.player_id == rt.world.players[&owner].hero.identity.id)
                .unwrap()
                .handheld,
            HandheldSelection::Item(item().slug)
        );
        let hp = rt.world.players[&owner].hero.max_hp;
        rt.passport_admissions
            .weapons
            .lock()
            .unwrap()
            .apply(Ok(vec![]), Instant::now());
        let request = |request_id, handheld| PrematchRequest {
            server_epoch: rt.server_epoch,
            match_id: rt.match_id,
            generation: view.generation,
            request_id,
            action: PrematchAction::Select {
                handheld,
                character: CharacterChoice::default(),
                hero_class: HeroClass::Warrior,
                avatar: None,
                sprite_character: None,
                role: Role::Mid,
                passport_ticket: None,
            },
        };
        let denied = request(1, HandheldSelection::Item(item().slug));
        let unequip = request(2, HandheldSelection::Unequipped);
        rt.handle_packet(owner, ClientPacket::Prematch { request: denied }, now);
        assert!(rt.world.players[&owner].draft.error.is_some());
        assert_eq!(
            rt.world.players[&owner].hero.identity.handheld,
            HandheldSelection::Item(item().slug)
        );
        rt.handle_packet(owner, ClientPacket::Prematch { request: unequip }, now);
        assert!(rt.world.players[&owner].draft.error.is_none());
        assert_eq!(
            rt.world.players[&owner].hero.identity.handheld,
            HandheldSelection::Unequipped
        );
        assert_eq!(rt.world.players[&owner].hero.max_hp, hp);
    }

    #[test]
    fn revoked_weapon_reconnect_can_unequip_without_restoring_revoked_prop() {
        use crate::{match_rules::MatchConfig, runtime::ServerRuntime};
        for fallback in [
            HandheldSelection::Unequipped,
            HandheldSelection::ClassDefault,
        ] {
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.set_nonblocking(true).unwrap();
            let mut rt = ServerRuntime::new(socket, MatchConfig::dev());
            rt.passport_admissions.fetch_weapons = approved;
            let owner: SocketAddr = "127.0.0.1:50311".parse().unwrap();
            let resumed: SocketAddr = "127.0.0.1:50312".parse().unwrap();
            let now = Instant::now();
            rt.handle_packet(owner, join(&item().slug, None), now);
            for _ in 0..200 {
                rt.receive_packets();
                if rt.world.players[&owner].joined {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(rt.world.players[&owner].joined);
            rt.world.players.get_mut(&owner).unwrap().last_seen = now - Duration::from_secs(6);
            let id = rt.world.players[&owner].hero.identity.id;
            let hp = rt.world.players[&owner].hero.hp;
            rt.passport_admissions
                .weapons
                .lock()
                .unwrap()
                .apply(Ok(vec![]), Instant::now());
            rt.handle_packet(resumed, join(&item().slug, None), now);
            assert!(
                !rt.world.players[&resumed].joined,
                "revoked ID cannot reclaim a session"
            );
            let mut packet = join(&item().slug, None);
            if let ClientPacket::Join { handheld, .. } = &mut packet {
                *handheld = fallback.clone();
            }
            rt.handle_packet(resumed, packet, now);
            let player = &rt.world.players[&resumed];
            assert!(player.joined);
            assert_eq!(player.hero.identity.id, id);
            assert_eq!(player.hero.hp, hp);
            assert_eq!(player.hero.identity.handheld, fallback);
            assert!(!rt.world.players.contains_key(&owner));
        }
    }

    #[test]
    fn wrong_kind_profile_pending_and_owned_items_never_authorize_weapons() {
        let mut variants = vec![];
        let mut bad = item();
        bad.support.status = "pending".into();
        variants.push(bad);
        let mut bad = item();
        bad.free = false;
        variants.push(bad);
        let mut bad = item();
        bad.asset_kind = AssetKind::Avatar;
        variants.push(bad);
        let mut bad = item();
        bad.support.profile = "humanoid-glb-v1".into();
        variants.push(bad);
        for bad in variants {
            let mut cache = WeaponCatalogue::default();
            cache.apply(Ok(vec![bad]), Instant::now());
            assert!(cache.items.is_empty());
        }
    }
}
