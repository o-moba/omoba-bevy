//! Account-owned regions. Servers keep their compile-time market; a native client
//! explicitly binds one region before constructing any regional service.
use crate::market::{Features, Market, PaymentProvider};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionId {
    Global,
    Ru,
}
impl RegionId {
    pub const ALL: [Self; 2] = [Self::Global, Self::Ru];
    pub const fn key(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Ru => "ru",
        }
    }
    pub const fn market(self) -> Market {
        match self {
            Self::Global => Market::Global,
            Self::Ru => Market::Ru,
        }
    }
    pub const fn from_market(market: Market) -> Option<Self> {
        match market {
            Market::Global => Some(Self::Global),
            Market::Ru => Some(Self::Ru),
            Market::Cn => None,
        }
    }
    pub fn profile(self) -> &'static RegionProfile {
        match self {
            Self::Global => &REGIONS[0],
            Self::Ru => &REGIONS[1],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionProfile {
    pub id: RegionId,
    pub lobby: &'static str,
    pub account_api: &'static str,
    pub portal: &'static str,
}
pub const REGIONS: [RegionProfile; 2] = [
    RegionProfile {
        id: RegionId::Global,
        lobby: "77.246.105.57:4000",
        account_api: "https://account.omoba.io",
        portal: "https://app.omoba.io",
    },
    RegionProfile {
        id: RegionId::Ru,
        lobby: "lobby.omoba.ru:4000",
        account_api: "https://account.omoba.ru",
        portal: "https://omoba.ru",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub lobby: String,
    pub account_api: String,
    pub portal: String,
}
impl RegionProfile {
    /// A contour has a separate regional authority. Never fall back to production
    /// or send an RU identity to the international contour when it is unavailable.
    pub fn endpoints(&self, contour: Option<&str>) -> Endpoints {
        if let Some(contour) = contour.filter(|s| !s.is_empty()) {
            let zone = if self.id == RegionId::Ru {
                "omoba.ru"
            } else {
                "omoba.io"
            };
            let portal = if self.id == RegionId::Ru {
                format!("https://{contour}.{zone}")
            } else {
                format!("https://app.{contour}.{zone}")
            };
            Endpoints {
                lobby: format!("lobby.{contour}.{zone}:4100"),
                account_api: format!("https://account.{contour}.{zone}"),
                portal,
            }
        } else {
            Endpoints {
                lobby: self.lobby.into(),
                account_api: self.account_api.into(),
                portal: self.portal.into(),
            }
        }
    }
}

static ACTIVE: OnceLock<RegionId> = OnceLock::new();
/// Binding is explicit and immutable. A different account region needs a cold start.
pub fn bind(region: RegionId) -> Result<(), &'static str> {
    match ACTIVE.set(region) {
        Ok(()) => Ok(()),
        Err(_) if ACTIVE.get() == Some(&region) => Ok(()),
        Err(_) => Err("Changing account region requires a restart."),
    }
}
pub fn active() -> Option<&'static RegionProfile> {
    ACTIVE.get().map(|r| r.profile())
}
/// Compatibility for regional server processes and callers outside the client.
pub fn market() -> Market {
    active().map_or(crate::market::MARKET, |r| r.id.market())
}
pub fn features() -> Features {
    features_for(market())
}
fn features_for(market: Market) -> Features {
    // Build overrides describe the compiled market only. They cannot enable
    // Ekza or a different region's payment integration after runtime binding.
    if market != crate::market::MARKET {
        return market.default_features();
    }
    Features::resolve(
        market,
        option_env!("OMOBA_FEATURE_EKZA"),
        option_env!("OMOBA_FEATURE_SUPPORTER"),
        option_env!("OMOBA_FEATURE_VK_SIGN_IN"),
    )
    .expect("compile-time feature switches were validated by market::FEATURES")
}
pub fn payment_providers() -> &'static [PaymentProvider] {
    crate::market::providers_of(market(), features())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_process_cannot_rebind_the_account_to_another_region() {
        assert!(bind(RegionId::Global).is_ok());
        assert!(bind(RegionId::Global).is_ok());
        assert!(bind(RegionId::Ru).is_err());
        assert_eq!(active().unwrap().id, RegionId::Global);
    }
    #[test]
    fn each_account_region_has_one_authority_and_its_own_contour() {
        assert!(!features_for(Market::Ru).ekza);
        for region in RegionId::ALL {
            let profile = region.profile();
            let live = profile.endpoints(None);
            let dev = profile.endpoints(Some("dev"));
            assert_ne!(live, dev);
            assert!(dev.account_api.contains(".dev."));
            assert!(dev.lobby.contains(".dev."));
            assert_eq!(
                serde_json::from_str::<RegionId>(&serde_json::to_string(&region).unwrap()).unwrap(),
                region
            );
        }
        assert_ne!(
            REGIONS[0].endpoints(Some("dev")),
            REGIONS[1].endpoints(Some("dev"))
        );
        assert!(!RegionId::Ru.market().default_features().ekza);
        assert!(
            !RegionId::Ru
                .market()
                .payment_providers()
                .contains(&PaymentProvider::Apple)
        );
        assert!(serde_json::from_str::<RegionId>("\"cn\"").is_err());
    }
}
