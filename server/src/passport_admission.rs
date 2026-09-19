//! Bounded asynchronous ticket verification, outside the gameplay tick.
use super::*;
use ekza_bevy_sdk::{passport::ProtectedAvatar, store::StoreAvatar};
use omoba_passport::PassportApi;
use std::sync::{Arc, Mutex, mpsc};

const MAX_PENDING: usize = 16;
/// A known free avatar is trusted this long before the registry is asked again,
/// which also bounds how long a withdrawn approval keeps working.
const FREE_TTL: Duration = Duration::from_secs(300);
/// A slug the list does not contain may trigger a new read at most this often, so
/// a freshly approved avatar works within seconds and junk slugs cannot flood the
/// registry.
const FREE_RETRY: Duration = Duration::from_secs(10);

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
    Known(StoreAvatar),
    Unknown,
    NeedsRead,
}

impl FreeCatalogue {
    fn lookup(&self, slug: &str, now: Instant) -> FreeLookup {
        let recent = |at: Option<Instant>, limit: Duration| {
            at.is_some_and(|at| now.saturating_duration_since(at) < limit)
        };
        match self.items.get(slug) {
            Some(item) if recent(self.fetched, FREE_TTL) => FreeLookup::Known(item.clone()),
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
}

pub(super) struct PassportAdmissions {
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
        let (sender, receiver) = mpsc::channel();
        Self {
            api: PassportApi::from_env().ok(),
            free: Arc::default(),
            fetch_free: omoba_passport::community::fetch_free,
            registry: omoba_passport::community::registry_url(),
            pending: HashSet::new(),
            sender,
            receiver: Mutex::new(receiver),
        }
    }
}

impl PassportAdmissions {
    pub fn begin(&mut self, addr: SocketAddr, packet: &ClientPacket) -> Admission {
        let ClientPacket::Join {
            avatar,
            passport_ticket,
            session_id,
            ..
        } = packet
        else {
            return Admission::Free;
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
                        });
                    });
                    return Admission::Pending;
                }
            }
        }
        let definition = shared::avatar_definition(slug);
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
            });
        });
        Admission::Pending
    }

    pub fn completed(&mut self) -> Vec<CompletedAdmission> {
        let responses: Vec<_> = self.receiver.lock().unwrap().try_iter().collect();
        for response in &responses {
            self.pending.remove(&response.addr);
        }
        responses
    }
}

/// Make the granted avatar visible to slug normalization and to the clients
/// that will render it. Roster entries are already known.
fn register(slug: &str, granted: ProtectedAvatar) -> bool {
    if shared::avatar_definition(slug).is_some() {
        return true;
    }
    shared::register_store_avatar(shared::AvatarDefinition {
        slug: slug.to_owned(),
        display_name: "Ekza avatar".into(),
        collection: "Ekza store".into(),
        license: "See creator terms".into(),
        source_url: granted.support.rendition.url.clone(),
        author: None,
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
    if let Some(existing) = shared::avatar_definition(&item.slug) {
        return existing.passport.as_ref() == Some(&item.protected);
    }
    shared::register_store_avatar(shared::AvatarDefinition {
        slug: item.slug.clone(),
        display_name: item.name.clone(),
        collection: "Ekza community".into(),
        license: item
            .license
            .clone()
            .unwrap_or_else(|| "See creator terms".into()),
        source_url: item.protected.support.rendition.url.clone(),
        author: item.author.clone(),
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
    use super::*;
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
        assert!(matches!(
            gate.begin(addr, &join(&free.slug)),
            Admission::Pending
        ));
        let done = settle(&mut gate);
        assert!(done.len() == 1 && done[0].allowed && done[0].addr == addr);
        // Now it is known: immediate, even with surrounding whitespace.
        assert!(matches!(
            gate.begin(addr, &join(&format!(" {} ", free.slug))),
            Admission::Free
        ));
        let entry = shared::avatar_definition(&free.slug).expect("registered for rendering");
        assert!(entry.free && entry.passport.as_ref() == Some(&free.protected));
        assert_eq!(
            shared::normalize_avatar_slug(Some(&free.slug)),
            Some(free.slug.as_str())
        );

        // The same feed lists an owned template: no ticket, no entry.
        assert!(matches!(
            gate.begin(addr, &join(&owned.slug)),
            Admission::Denied
        ));
        assert!(shared::avatar_definition(&owned.slug).is_none());
        // A well-formed slug the registry never listed: a client cannot declare it free.
        assert!(matches!(
            gate.begin(addr, &join(&absent.slug)),
            Admission::Denied
        ));
        assert!(shared::avatar_definition(&absent.slug).is_none());
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
            shared::normalize_avatar_slug(Some(&slug)),
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
        assert!(shared::avatar_definition(&format!("ekza-{}", "a".repeat(64))).is_none());
        let free: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":shared::avatar_roster()[0].slug
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &free), Admission::Free));
    }
}
