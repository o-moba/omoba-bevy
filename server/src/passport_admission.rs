//! Bounded asynchronous ticket verification, outside the gameplay tick.
use super::*;
use ekza_bevy_sdk::passport::ProtectedAvatar;
use omoba_passport::PassportApi;
use std::sync::{Mutex, mpsc};

const MAX_PENDING: usize = 16;

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
    pending: HashSet<SocketAddr>,
    sender: mpsc::Sender<CompletedAdmission>,
    receiver: Mutex<mpsc::Receiver<CompletedAdmission>>,
}

impl Default for PassportAdmissions {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            api: PassportApi::from_env().ok(),
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
        assert_eq!(shared::normalize_avatar_slug(Some(&slug)), Some(slug.as_str()));

        // A valid ticket for some other owned avatar cannot unlock this slug.
        let mut other = response.clone();
        other.avatar_id = format!("solana:devnet:avatar-data:{}", "4".repeat(32));
        other.support.rendition.sha256 = "c".repeat(64);
        assert!(
            verify_admission(&slug, None, "session-1", "scoped-proof-123456", |_, _| Ok(other))
                .is_err()
        );
        // Nor can a ticket approved for another project or profile.
        let mut foreign = response;
        foreign.support.project_id = "ekza-space".into();
        assert!(
            verify_admission(&slug, None, "session-1", "scoped-proof-123456", |_, _| Ok(foreign))
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
        let mut gate = PassportAdmissions::default();
        let addr = "127.0.0.1:12345".parse().unwrap();
        let paid: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":"ekza-forged", "session_id":"session-1"
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &paid), Admission::Denied));
        // A well-formed store slug without a ticket never reaches the passport.
        let ticketless: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":format!("ekza-{}", "a".repeat(64)),
            "session_id":"session-1"
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &ticketless), Admission::Denied));
        let free: ClientPacket = serde_json::from_value(serde_json::json!({
            "type":"join", "team":"green", "avatar":shared::avatar_roster()[0].slug
        }))
        .unwrap();
        assert!(matches!(gate.begin(addr, &free), Admission::Free));
    }
}
