//! New-admission decisions from an independently pinned directory. Callers keep
//! retained party/worker ownership separate from refresh and new selection.
use super::{DirectoryCache, DirectoryError, DirectoryTrust, VerifiedDirectory};
use shared::{
    compatibility::ReleaseContract,
    game_directory::{Directory, Lobby, LobbyStatus, Policy, Selection},
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Network,
    Cache,
    PinnedFallback,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    region: shared::region::RegionId,
    contour: String,
    lobby_id: String,
    display: String,
    endpoint: SocketAddr,
    sequence: Option<u64>,
    origin: Origin,
}
impl Route {
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn display(&self) -> &str {
        &self.display
    }
    pub fn lobby_id(&self) -> &str {
        &self.lobby_id
    }
    pub fn sequence(&self) -> Option<u64> {
        self.sequence
    }
    pub fn origin(&self) -> Origin {
        self.origin
    }
    /// Retained only by the caller that actually opens this lobby. A future
    /// directory refresh must not reinterpret a live seat's return destination.
    pub fn retain(self) -> OwnedLobby {
        OwnedLobby(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedLobby(Route);
impl OwnedLobby {
    /// Retain the exact address through policy changes. Before opening a fresh
    /// transport, resume_at must revalidate current ownership; this value alone
    /// is not permission to reconnect to an expired or removed destination.
    pub fn resume_route(&self) -> &Route {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Route(Route),
    UpdateRequired { url: Option<String> },
    Maintenance { message: Option<String> },
    Full,
    Incompatible,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeRoute {
    lobby: Route,
    worker: SocketAddr,
}
impl ResumeRoute {
    pub fn lobby(&self) -> &Route {
        &self.lobby
    }
    pub fn worker(&self) -> SocketAddr {
        self.worker
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeDecision {
    Pending,
    Ready(ResumeRoute),
    Unavailable,
}
#[derive(Clone, Copy)]
enum AuthorityUnavailable {
    Pending,
    Unavailable,
}
struct Authority<'a> {
    directory: &'a Directory,
    sequence: Option<u64>,
    origin: Origin,
}

/// A build/operator-supplied fallback, never a hostname resolved after a failed
/// directory read. Construction applies the same address/scope guards as D1.
#[derive(Clone)]
pub struct PinnedFallback {
    directory: Directory,
}
impl PinnedFallback {
    pub fn new(trust: &DirectoryTrust, lobby: Lobby) -> Result<Self, DirectoryError> {
        if lobby.status != LobbyStatus::Open {
            return Err(DirectoryError::Invalid("closed fallback lobby"));
        }
        let directory = Directory {
            version: 1,
            region: trust.region,
            contour: trust.contour.clone(),
            sequence: 1,
            issued_at_ms: 1,
            expires_at_ms: 2,
            policy: Policy {
                min_release: lobby.contract.release.clone(),
                maintenance: false,
                update_url: None,
                message: None,
            },
            lobbies: vec![lobby],
        };
        // The static fallback has no publication clock. Reuse schema/address
        // validation at a fixed internal instant, never as an authenticated D1.
        directory
            .validate_at(trust.region, &trust.contour, 1, 0)
            .map_err(DirectoryError::Invalid)?;
        trust.validate_destinations(&directory.lobbies)?;
        Ok(Self { directory })
    }
}

#[derive(Clone)]
pub struct Admission {
    cache: DirectoryCache,
    fallback: Option<PinnedFallback>,
    // Keep authenticated history even after expiry or a failed durable write.
    // Otherwise a subsequent outage could revive the older open policy.
    floor: Option<VerifiedDirectory>,
    current: Option<VerifiedDirectory>,
    origin: Origin,
    attempted: bool,
    error: Option<DirectoryError>,
}
impl Admission {
    pub fn new(
        cache_path: PathBuf,
        trust: DirectoryTrust,
        fallback: Option<PinnedFallback>,
    ) -> Result<Self, DirectoryError> {
        if fallback.as_ref().is_some_and(|f| {
            f.directory.region != trust.region || f.directory.contour != trust.contour
        }) {
            return Err(DirectoryError::Invalid("fallback authority mismatch"));
        }
        Ok(Self {
            cache: DirectoryCache::new(cache_path, trust),
            fallback,
            floor: None,
            current: None,
            origin: Origin::Cache,
            attempted: false,
            error: None,
        })
    }
    fn observe(&mut self, document: &VerifiedDirectory) -> Result<(), DirectoryError> {
        if let Some(floor) = &self.floor {
            if document.directory.sequence < floor.directory.sequence {
                return Err(DirectoryError::Rollback);
            }
            if document.directory.sequence == floor.directory.sequence
                && document.envelope.payload != floor.envelope.payload
            {
                return Err(DirectoryError::Equivocation);
            }
        }
        self.floor = Some(document.clone());
        Ok(())
    }
    fn cached(&mut self, now_ms: u64) -> Result<Origin, DirectoryError> {
        let saved = self.cache.load_history_at(now_ms)?;
        if let Some(saved) = &saved {
            // A valid newer policy that failed to persist fences old disk data.
            self.observe(saved)?;
        } else if self.floor.is_some() {
            return Err(DirectoryError::CacheInvalid);
        }
        self.current = saved.filter(|d| d.directory.expires_at_ms > now_ms);
        if self.current.is_none() && self.fallback.is_none() {
            return Err(DirectoryError::Invalid(
                "no valid directory or pinned fallback",
            ));
        }
        self.origin = if self.current.is_some() {
            Origin::Cache
        } else {
            Origin::PinnedFallback
        };
        Ok(self.origin)
    }
    /// Run after the bounded fetch on a worker thread. Network failures and
    /// unauthenticated bytes may use the currently valid cache or pinned fallback;
    /// cache corruption, equivocation and persistence failures fail closed.
    pub fn refresh_at(
        &mut self,
        bytes: Result<&[u8], &'static str>,
        now_ms: u64,
    ) -> Result<Origin, DirectoryError> {
        self.attempted = true;
        let result = self.refresh_inner(bytes, now_ms);
        self.error = result.as_ref().err().cloned();
        result
    }
    fn refresh_inner(
        &mut self,
        bytes: Result<&[u8], &'static str>,
        now_ms: u64,
    ) -> Result<Origin, DirectoryError> {
        // Read durable history before accepting anything, including expired
        // sequence floors. An in-memory newer floor may be repaired by a fresh
        // equally/newer publication, but never by silently using older cache.
        let saved = self.cache.load_history_at(now_ms)?;
        if let Some(saved) = saved
            && self
                .floor
                .as_ref()
                .is_none_or(|f| saved.directory.sequence >= f.directory.sequence)
        {
            self.observe(&saved)?;
        }
        let candidate = bytes.ok().and_then(|bytes| {
            self.cache
                .trust
                .verify_at(bytes, now_ms, 0)
                .ok()
                .map(|verified| (bytes, verified))
        });
        let Some((bytes, candidate)) = candidate else {
            return self.cached(now_ms);
        };
        // Remember authentic policy before disk publication. A failed write of
        // new maintenance/update policy must not expose the old open cache.
        match self.observe(&candidate) {
            Ok(()) => (),
            Err(DirectoryError::Rollback) => return self.cached(now_ms),
            Err(error) => return Err(error),
        }
        match self.cache.accept_at(bytes, now_ms) {
            Ok(accepted) => {
                self.current = Some(accepted);
                self.origin = Origin::Network;
                Ok(Origin::Network)
            }
            Err(DirectoryError::Rollback) => self.cached(now_ms),
            Err(error) => Err(error),
        }
    }
    fn authority_at(&self, now_ms: u64) -> Result<Authority<'_>, AuthorityUnavailable> {
        if !self.attempted {
            return Err(AuthorityUnavailable::Pending);
        }
        if self.error.is_some()
            || self
                .floor
                .as_ref()
                .is_some_and(|f| f.directory.issued_at_ms > now_ms.saturating_add(30_000))
        {
            return Err(AuthorityUnavailable::Unavailable);
        }
        let (directory, sequence, origin) = match &self.current {
            Some(current) if current.directory.expires_at_ms > now_ms => (
                &current.directory,
                Some(current.directory.sequence),
                self.origin,
            ),
            _ => match &self.fallback {
                Some(fallback) => (&fallback.directory, None, Origin::PinnedFallback),
                None => return Err(AuthorityUnavailable::Unavailable),
            },
        };
        Ok(Authority {
            directory,
            sequence,
            origin,
        })
    }
    /// Return to the retained lobby without applying new-search selection.
    /// Current signed ownership (or the explicit fallback) remains mandatory.
    pub fn owner_at(&self, now_ms: u64, hint: &str) -> Decision {
        let authority = match self.authority_at(now_ms) {
            Ok(authority) => authority,
            Err(AuthorityUnavailable::Pending) => return Decision::Pending,
            Err(AuthorityUnavailable::Unavailable) => return Decision::Unavailable,
        };
        for lobby in &authority.directory.lobbies {
            if let Some(endpoint) = lobby_endpoint(lobby, hint) {
                return Decision::Route(Route {
                    region: authority.directory.region,
                    contour: authority.directory.contour.clone(),
                    lobby_id: lobby.id.clone(),
                    display: lobby.display_endpoint(),
                    endpoint,
                    sequence: authority.sequence,
                    origin: authority.origin,
                });
            }
        }
        Decision::Unavailable
    }
    /// Revalidate an untrusted saved hint against current authenticated ownership,
    /// independently of new-admission policy/weights/contracts. Never select a
    /// different lobby, resolve saved DNS, or use expired directory authority.
    pub fn resume_at(&self, now_ms: u64, lobby_hint: &str, worker_hint: &str) -> ResumeDecision {
        let authority = match self.authority_at(now_ms) {
            Ok(authority) => authority,
            Err(AuthorityUnavailable::Pending) => return ResumeDecision::Pending,
            Err(_) => return ResumeDecision::Unavailable,
        };
        for lobby in &authority.directory.lobbies {
            let endpoints = lobby.network_endpoints();
            let Some(mut owner) = lobby_endpoint(lobby, lobby_hint) else {
                continue;
            };
            let worker = if let Ok(endpoint) = worker_hint.parse::<SocketAddr>() {
                let endpoint = canonical_socket(endpoint);
                if endpoint.port() == 0
                    || !endpoints.iter().any(|known| known.ip() == endpoint.ip())
                {
                    return ResumeDecision::Unavailable;
                }
                // A legacy hint saved a DNS label, not the selected IP. Preserve
                // its already concrete worker IP when that IP is authorized.
                if lobby_hint.parse::<SocketAddr>().is_err() {
                    owner = SocketAddr::new(endpoint.ip(), lobby.port);
                }
                endpoint
            } else {
                let Some((host, port)) = worker_hint.rsplit_once(':') else {
                    return ResumeDecision::Unavailable;
                };
                let Ok(port) = port.parse::<u16>() else {
                    return ResumeDecision::Unavailable;
                };
                if port == 0 || host.contains(':') || host.to_ascii_lowercase() != lobby.host {
                    return ResumeDecision::Unavailable;
                }
                SocketAddr::new(owner.ip(), port)
            };
            return ResumeDecision::Ready(ResumeRoute {
                lobby: Route {
                    region: authority.directory.region,
                    contour: authority.directory.contour.clone(),
                    lobby_id: lobby.id.clone(),
                    display: lobby.display_endpoint(),
                    endpoint: owner,
                    sequence: authority.sequence,
                    origin: authority.origin,
                },
                worker,
            });
        }
        ResumeDecision::Unavailable
    }
    pub fn decision_at(
        &self,
        now_ms: u64,
        client: &ReleaseContract,
        entropy: u64,
        party: Option<&OwnedLobby>,
    ) -> Decision {
        let authority = match self.authority_at(now_ms) {
            Ok(authority) => authority,
            Err(AuthorityUnavailable::Pending) => return Decision::Pending,
            Err(AuthorityUnavailable::Unavailable) => return Decision::Unavailable,
        };
        let Authority {
            directory,
            sequence,
            origin,
        } = authority;
        match directory.select(client, entropy) {
            Selection::UpdateRequired => Decision::UpdateRequired {
                url: directory.policy.update_url.clone(),
            },
            Selection::Maintenance => Decision::Maintenance {
                message: directory.policy.message.clone(),
            },
            Selection::Full => Decision::Full,
            Selection::Incompatible => Decision::Incompatible,
            Selection::Lobby(selected) => {
                let lobby = if let Some(OwnedLobby(owned)) = party {
                    if owned.region != directory.region || owned.contour != directory.contour {
                        return Decision::Unavailable;
                    }
                    let Some(lobby) = directory.lobbies.iter().find(|l| {
                        l.id == owned.lobby_id
                            && l.status == LobbyStatus::Open
                            && client.compare(&l.contract).is_ok()
                            && l.network_endpoints().contains(&owned.endpoint)
                    }) else {
                        return Decision::Unavailable;
                    };
                    lobby
                } else {
                    selected
                };
                let endpoints = lobby.network_endpoints();
                let endpoint = party.map_or_else(
                    || endpoints[(entropy as usize) % endpoints.len()],
                    |owned| owned.0.endpoint,
                );
                Decision::Route(Route {
                    region: directory.region,
                    contour: directory.contour.clone(),
                    lobby_id: lobby.id.clone(),
                    display: lobby.display_endpoint(),
                    endpoint,
                    sequence,
                    origin,
                })
            }
        }
    }
}

fn lobby_endpoint(lobby: &Lobby, hint: &str) -> Option<SocketAddr> {
    let endpoints = lobby.network_endpoints();
    if let Ok(endpoint) = hint.parse::<SocketAddr>() {
        let endpoint = canonical_socket(endpoint);
        endpoints.contains(&endpoint).then_some(endpoint)
    } else {
        (hint.to_ascii_lowercase() == lobby.display_endpoint()).then(|| endpoints[0])
    }
}

fn canonical_socket(endpoint: SocketAddr) -> SocketAddr {
    let ip = match endpoint.ip() {
        std::net::IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or(std::net::IpAddr::V6(ip), std::net::IpAddr::V4),
        ip => ip,
    };
    SocketAddr::new(ip, endpoint.port())
}

/// One bounded anonymous fetch in flight. Polling never blocks the render
/// thread on HTTPS or filesystem locks. No runtime URL/token override is used.
pub struct Resolver {
    admission: Admission,
    transport: omoba_passport::game_directory::DirectoryTransport,
    pending: Option<Mutex<mpsc::Receiver<Admission>>>,
}
impl Resolver {
    pub fn new(admission: Admission) -> Result<Self, &'static str> {
        let trust = &admission.cache.trust;
        let contour = (trust.contour != "production").then_some(trust.contour.as_str());
        Ok(Self {
            transport: omoba_passport::game_directory::DirectoryTransport::for_region(
                trust.region,
                contour,
            )?,
            admission,
            pending: None,
        })
    }
    pub fn begin_refresh(&mut self) -> bool {
        let transport = self.transport.clone();
        self.start(
            move || transport.fetch(),
            || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|now| now.as_millis() as u64)
                    .map_err(|_| "directory clock unavailable")
            },
        )
    }
    fn start(
        &mut self,
        fetch: impl FnOnce() -> Result<Vec<u8>, &'static str> + Send + 'static,
        now: impl FnOnce() -> Result<u64, &'static str> + Send + 'static,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let mut next = self.admission.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending = Some(Mutex::new(receiver));
        let worker = std::thread::Builder::new()
            .name("game-directory".into())
            .spawn(move || {
                let bytes = fetch();
                if let Ok(now) = now() {
                    let _ = next.refresh_at(bytes.as_deref().map_err(|error| *error), now);
                } else {
                    next.attempted = true;
                    next.error = Some(DirectoryError::Invalid("directory clock unavailable"));
                }
                let _ = sender.send(next);
            });
        if worker.is_err() {
            self.pending = None;
            self.admission.attempted = true;
            self.admission.error = Some(DirectoryError::Invalid("directory worker unavailable"));
            return false;
        }
        true
    }
    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = pending.lock().ok().map(|r| r.try_recv());
        match result {
            Some(Ok(admission)) => self.admission = admission,
            Some(Err(mpsc::TryRecvError::Empty)) => return,
            _ => {
                self.admission.attempted = true;
                self.admission.error = Some(DirectoryError::Invalid("directory fetch stopped"));
            }
        }
        self.pending = None;
    }
    pub fn admission(&self) -> &Admission {
        &self.admission
    }
}

#[cfg(test)]
mod tests;
