//! Public trust/fallback configuration supplied at compile time. The running
//! app never reads its pins from a response, preferences or environment override.
use super::{
    DirectoryError, DirectoryTrust, PublicPin,
    admission::{Admission, PinnedFallback},
};
use serde::Deserialize;
use shared::{
    compatibility::ReleaseContract,
    game_directory::{Lobby, LobbyStatus},
    region::RegionId,
};
use std::{
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
};

const MAX_BUILD_BYTES: usize = 16_384;
const COMPILED: Option<&str> = option_env!("OMOBA_DEFAULT_GAME_DIRECTORY_TRUST");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleSpec {
    version: u32,
    contour: String,
    regions: Vec<RegionSpec>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionSpec {
    region: RegionId,
    pins: Vec<PinSpec>,
    fallback: FallbackSpec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinSpec {
    key_id: String,
    public_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FallbackSpec {
    id: String,
    location: String,
    host: String,
    addresses: Vec<IpAddr>,
    port: u16,
}

#[derive(Clone)]
pub struct RegionTrust {
    trust: DirectoryTrust,
    fallback: PinnedFallback,
    fallback_endpoints: Vec<std::net::SocketAddr>,
    fallback_display: String,
}
impl RegionTrust {
    pub fn region(&self) -> RegionId {
        self.trust.region
    }
    pub fn contour(&self) -> &str {
        &self.trust.contour
    }
    /// Explicitly include both scope dimensions even when the caller already
    /// supplies an account's own settings directory. Labels cannot escape it.
    pub fn cache_path(&self, settings_root: &Path) -> PathBuf {
        settings_root
            .join("game-directory")
            .join(&self.trust.contour)
            .join(self.trust.region.key())
            .join("last-good.json")
    }
    pub fn admission(&self, settings_root: &Path) -> Result<Admission, DirectoryError> {
        Admission::new(
            self.cache_path(settings_root),
            self.trust.clone(),
            Some(self.fallback.clone()),
        )
    }
}

#[derive(Clone)]
pub struct BuildTrust {
    contour: String,
    regions: Vec<RegionTrust>,
}
impl BuildTrust {
    /// Unconfigured legacy builds remain distinguishable from malformed or
    /// partially configured ones. A missing requested region never uses legacy.
    pub fn compiled() -> Result<Option<Self>, DirectoryError> {
        Self::parse(
            COMPILED,
            crate::session_config::BUILD_CONTOUR.unwrap_or("production"),
        )
    }
    pub fn parse(
        raw: Option<&str>,
        expected_contour: &str,
    ) -> Result<Option<Self>, DirectoryError> {
        let Some(raw) = raw else { return Ok(None) };
        if raw.is_empty() || raw.len() > MAX_BUILD_BYTES {
            return Err(DirectoryError::Invalid(
                "invalid directory build configuration size",
            ));
        }
        let spec: BundleSpec = serde_json::from_str(raw)
            .map_err(|_| DirectoryError::Invalid("invalid directory build configuration"))?;
        if spec.version != 1
            || spec.contour != expected_contour
            || spec.regions.is_empty()
            || spec.regions.len() > RegionId::ALL.len()
        {
            return Err(DirectoryError::Invalid(
                "directory build authority mismatch",
            ));
        }
        let mut regions: Vec<RegionTrust> = Vec::new();
        for region in spec.regions {
            if regions.iter().any(|r| r.region() == region.region)
                || region.pins.is_empty()
                || region.pins.len() > 4
            {
                return Err(DirectoryError::Invalid(
                    "invalid directory build region or pins",
                ));
            }
            let pins = region
                .pins
                .into_iter()
                .map(|pin| {
                    Ok(PublicPin {
                        key_id: pin.key_id,
                        public_key: public_key(&pin.public_key)?,
                    })
                })
                .collect::<Result<Vec<_>, DirectoryError>>()?;
            let trust = DirectoryTrust::new(region.region, &spec.contour, pins)?;
            let fallback = Lobby {
                id: region.fallback.id,
                location: region.fallback.location,
                host: region.fallback.host,
                addresses: region.fallback.addresses,
                port: region.fallback.port,
                // Only the concrete destination is statically authorized. The
                // actual server contract is still checked by UDP preflight.
                contract: ReleaseContract::current(),
                status: LobbyStatus::Open,
                weight: 1,
            };
            let fallback_endpoints = fallback.network_endpoints();
            let fallback_display = fallback.display_endpoint();
            let fallback = PinnedFallback::new(&trust, fallback)?;
            regions.push(RegionTrust {
                trust,
                fallback,
                fallback_endpoints,
                fallback_display,
            });
        }
        Ok(Some(Self {
            contour: spec.contour,
            regions,
        }))
    }
    pub fn region(&self, region: RegionId) -> Result<&RegionTrust, DirectoryError> {
        self.regions
            .iter()
            .find(|r| r.region() == region)
            .ok_or(DirectoryError::Invalid(
                "account region has no directory trust",
            ))
    }
    /// Operator-only audit: no account/service initialization, HTTP or cache I/O.
    pub fn audit(&self) -> serde_json::Value {
        serde_json::json!({"configured":true,"version":1,"contour":self.contour,"regions":self.regions.iter().map(|r|serde_json::json!({
            "region":r.region(),"pins":r.trust.pins.iter().map(|(id,key)|serde_json::json!({"key_id":id,"public_key":key.to_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()})).collect::<Vec<_>>(),
            "fallback":r.fallback_display,"concrete_endpoints":r.fallback_endpoints.iter().map(ToString::to_string).collect::<Vec<_>>()
        })).collect::<Vec<_>>()})
    }
}
fn public_key(hex: &str) -> Result<[u8; 32], DirectoryError> {
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(DirectoryError::Invalid(
            "invalid directory build public key encoding",
        ));
    }
    let mut out = [0; 32];
    let digit = |b: u8| {
        if b.is_ascii_digit() {
            b - b'0'
        } else {
            b - b'a' + 10
        }
    };
    for (out, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        *out = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(out)
}
/// CLI callers must not certify an unconfigured build as directory-ready.
pub fn audit_compiled() -> Result<serde_json::Value, DirectoryError> {
    BuildTrust::compiled()?
        .map(|bundle| bundle.audit())
        .ok_or(DirectoryError::Invalid(
            "build has no directory trust configuration",
        ))
}

/// Operator input check for release preparation. It reuses the runtime parser,
/// including curve-strength and destination checks; it never changes compiled
/// trust or initializes accounts, networking, settings, caches, or the UI.
pub fn audit_input(
    input: impl Read,
    expected_contour: &str,
) -> Result<serde_json::Value, DirectoryError> {
    if !matches!(expected_contour, "production" | "dev") {
        return Err(DirectoryError::Invalid(
            "input audit requires an explicit production or dev contour",
        ));
    }
    let mut raw = Vec::new();
    input
        .take(MAX_BUILD_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| DirectoryError::Invalid("cannot read directory build input"))?;
    let raw = std::str::from_utf8(&raw)
        .map_err(|_| DirectoryError::Invalid("directory build input must be UTF-8"))?;
    let bundle = BuildTrust::parse(Some(raw), expected_contour)?.ok_or(DirectoryError::Invalid(
        "directory build input must be configured",
    ))?;
    Ok(serde_json::json!({
        "input_valid": true,
        "compiled_configuration_checked": false,
        "audit": bundle.audit(),
    }))
}

/// A recognized operator command always returns a result (including bad usage),
/// so invalid audit arguments cannot accidentally launch the game. Unrelated
/// game arguments retain the ordinary entry path and do not consume input.
pub fn audit_cli(
    args: &[String],
    input: impl Read,
) -> Option<Result<serde_json::Value, DirectoryError>> {
    match args.first().map(String::as_str) {
        Some("--check-directory-trust") if args.len() == 1 => Some(audit_compiled()),
        Some("--check-directory-trust-input") if args.len() == 2 => {
            Some(audit_input(input, &args[1]))
        }
        Some("--check-directory-trust" | "--check-directory-trust-input") => {
            Some(Err(DirectoryError::Invalid(
                "use --check-directory-trust, or --check-directory-trust-input production|dev with public JSON on stdin",
            )))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
