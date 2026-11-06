//! Native D1 trust and durable last-good cache. No key or destination is learned
//! from the document itself. Runtime routing is integrated separately.
use ed25519_dalek::{Signature, VerifyingKey};
use shared::{
    game_directory::{Directory, MAX_WIRE_BYTES, SignedDirectory},
    region::RegionId,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct PublicPin {
    pub key_id: String,
    pub public_key: [u8; 32],
}
#[derive(Clone)]
pub struct DirectoryTrust {
    region: RegionId,
    contour: String,
    pins: Vec<(String, VerifyingKey)>,
}
#[derive(Clone, Debug)]
pub struct VerifiedDirectory {
    envelope: SignedDirectory,
    directory: Directory,
}
impl VerifiedDirectory {
    pub fn directory(&self) -> &Directory {
        &self.directory
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum DirectoryError {
    Invalid(&'static str),
    Rollback,
    Equivocation,
    CacheInvalid,
    CacheIo,
    CacheBusy,
}
impl DirectoryTrust {
    /// Callers supply separately trusted build pins, never envelope-provided keys.
    /// Overlap supports key rotation; retain the previous pin while its cached
    /// sequence fence is needed, and advance sequence on a key publication.
    pub fn new(
        region: RegionId,
        contour: &str,
        pins: Vec<PublicPin>,
    ) -> Result<Self, DirectoryError> {
        if contour.is_empty()
            || contour.len() > 32
            || !contour
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || pins.is_empty()
            || pins.len() > 4
        {
            return Err(DirectoryError::Invalid(
                "invalid directory trust configuration",
            ));
        }
        let mut verified = Vec::new();
        for pin in pins {
            SignedDirectory {
                key_id: pin.key_id.clone(),
                payload: String::new(),
                signature: String::new(),
            }
            .signing_bytes()
            .map_err(DirectoryError::Invalid)?;
            if verified.iter().any(|(id, _)| id == &pin.key_id) {
                return Err(DirectoryError::Invalid("duplicate directory pin"));
            }
            let key = VerifyingKey::from_bytes(&pin.public_key)
                .map_err(|_| DirectoryError::Invalid("invalid directory public key"))?;
            verified.push((pin.key_id, key));
        }
        Ok(Self {
            region,
            contour: contour.into(),
            pins: verified,
        })
    }
    fn authenticate(&self, bytes: &[u8]) -> Result<VerifiedDirectory, DirectoryError> {
        let envelope = SignedDirectory::parse(bytes).map_err(DirectoryError::Invalid)?;
        let key = self
            .pins
            .iter()
            .find(|(id, _)| id == &envelope.key_id)
            .map(|(_, key)| key)
            .ok_or(DirectoryError::Invalid("untrusted directory key"))?;
        // Parsing above enforces exactly128 lowercase ASCII hexadecimal digits.
        let mut signature = [0; 64];
        for (out, pair) in signature
            .iter_mut()
            .zip(envelope.signature.as_bytes().chunks_exact(2))
        {
            let digit = |b: u8| {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b - b'a' + 10
                }
            };
            *out = digit(pair[0]) * 16 + digit(pair[1]);
        }
        key.verify_strict(
            &envelope.signing_bytes().map_err(DirectoryError::Invalid)?,
            &Signature::from_bytes(&signature),
        )
        .map_err(|_| DirectoryError::Invalid("invalid directory signature"))?;
        let directory = Directory::parse(&envelope.payload).map_err(DirectoryError::Invalid)?;
        // Authenticate historical cache at its signed issue instant. Expiry
        // stops admission, but must not erase the durable sequence floor.
        directory
            .validate_at(self.region, &self.contour, directory.issued_at_ms, 0)
            .map_err(DirectoryError::Invalid)?;
        let contour = (self.contour != "production").then_some(self.contour.as_str());
        for lobby in &directory.lobbies {
            for host in std::iter::once(lobby.host.clone())
                .chain(lobby.addresses.iter().map(ToString::to_string))
            {
                if !crate::session_config::endpoint_allowed_in_region(
                    contour,
                    Some(self.region),
                    &host,
                ) || (contour.is_none() && known_dev_destination(&host))
                {
                    return Err(DirectoryError::Invalid(
                        "directory destination crosses authority",
                    ));
                }
            }
        }
        Ok(VerifiedDirectory {
            envelope,
            directory,
        })
    }
    pub fn verify_at(
        &self,
        bytes: &[u8],
        now_ms: u64,
        minimum_sequence: u64,
    ) -> Result<VerifiedDirectory, DirectoryError> {
        let verified = self.authenticate(bytes)?;
        if verified.directory.sequence < minimum_sequence {
            return Err(DirectoryError::Rollback);
        }
        verified
            .directory
            .validate_at(self.region, &self.contour, now_ms, minimum_sequence)
            .map_err(DirectoryError::Invalid)?;
        Ok(verified)
    }
}

fn known_dev_destination(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    // The shared dev VPS/namespace from ekza-registry origin/dev's
    // docs/dev-contour.md. Production directory trust cannot cross to it,
    // including when an otherwise production-looking label hides its IP.
    let dev_ip = std::net::Ipv4Addr::new(146, 103, 102, 18);
    let ip = host.parse::<std::net::IpAddr>().ok();
    if ip.is_some_and(|ip| match ip {
        std::net::IpAddr::V4(ip) => ip == dev_ip,
        std::net::IpAddr::V6(ip) => ip.to_ipv4_mapped() == Some(dev_ip),
    }) {
        return true;
    }
    ["dev.omoba.io", "dev.omoba.ru", "dev.ekza.io"]
        .iter()
        .any(|zone| host == *zone || host.ends_with(&format!(".{zone}")))
}

pub struct DirectoryCache {
    path: PathBuf,
    trust: DirectoryTrust,
}
impl DirectoryCache {
    /// The caller supplies the account-region/contour's own settings path.
    pub fn new(path: PathBuf, trust: DirectoryTrust) -> Self {
        Self { path, trust }
    }
    fn lock(&self) -> Result<File, DirectoryError> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(DirectoryError::CacheIo)?;
        fs::create_dir_all(parent).map_err(|_| DirectoryError::CacheIo)?;
        // A separate stable inode survives atomic publication replacement.
        // OS locks release on process death; do not remove/recreate this file.
        let lock = self.path.with_extension("directory-lock");
        match fs::symlink_metadata(&lock) {
            Ok(m) if !m.is_file() => return Err(DirectoryError::CacheInvalid),
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(DirectoryError::CacheIo),
            _ => (),
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock)
            .map_err(|_| DirectoryError::CacheIo)?;
        if !file
            .metadata()
            .map_err(|_| DirectoryError::CacheIo)?
            .is_file()
        {
            return Err(DirectoryError::CacheInvalid);
        }
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => DirectoryError::CacheBusy,
            _ => DirectoryError::CacheIo,
        })?;
        Ok(file)
    }
    fn read_locked(&self) -> Result<Option<VerifiedDirectory>, DirectoryError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DirectoryError::CacheIo),
        };
        if !metadata.is_file() || metadata.len() > MAX_WIRE_BYTES as u64 {
            return Err(DirectoryError::CacheInvalid);
        }
        let file = File::open(&self.path).map_err(|_| DirectoryError::CacheIo)?;
        let actual = file.metadata().map_err(|_| DirectoryError::CacheIo)?;
        if !actual.is_file() || actual.len() > MAX_WIRE_BYTES as u64 {
            return Err(DirectoryError::CacheInvalid);
        }
        let mut bytes = Vec::new();
        file.take(MAX_WIRE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DirectoryError::CacheIo)?;
        self.trust
            .authenticate(&bytes)
            .map(Some)
            .map_err(|_| DirectoryError::CacheInvalid)
    }
    /// An expired authenticated file yields no admission data, but stays on
    /// disk as a rollback fence. Corruption is an error, never a reset to zero.
    pub fn load_at(&self, now_ms: u64) -> Result<Option<VerifiedDirectory>, DirectoryError> {
        let _lock = self.lock()?;
        let saved = self.read_locked()?;
        let Some(saved) = saved else {
            return Ok(None);
        };
        if saved.directory.issued_at_ms > now_ms.saturating_add(30_000) {
            return Err(DirectoryError::Invalid(
                "directory cache clock moved backwards",
            ));
        }
        if saved.directory.expires_at_ms <= now_ms {
            return Ok(None);
        }
        saved
            .directory
            .validate_at(self.trust.region, &self.trust.contour, now_ms, 0)
            .map_err(DirectoryError::Invalid)?;
        Ok(Some(saved))
    }
    /// Authenticate, fence against even expired history, then persist before
    /// exposing the document. Do not use an older open cache after this returns
    /// a persistence error for a valid newer closed/maintenance policy.
    pub fn accept_at(
        &self,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<VerifiedDirectory, DirectoryError> {
        let _lock = self.lock()?;
        let saved = self.read_locked()?;
        let minimum = saved.as_ref().map_or(0, |s| s.directory.sequence);
        let new = self.trust.verify_at(bytes, now_ms, minimum)?;
        if let Some(old) = saved
            && old.directory.sequence == new.directory.sequence
        {
            if old.envelope.payload != new.envelope.payload {
                return Err(DirectoryError::Equivocation);
            }
            return Ok(new);
        }
        persist(&self.path, bytes).map_err(|_| DirectoryError::CacheIo)?;
        Ok(new)
    }
}
fn persist(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::other("random cache name unavailable"))?;
    let name = nonce.iter().map(|n| format!("{n:02x}")).collect::<String>();
    let temporary = path.with_extension(format!("pending-{name}"));
    let mut file = File::create_new(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(
            path.parent()
                .ok_or_else(|| io::Error::other("missing cache parent"))?,
        )?
        .sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests;
