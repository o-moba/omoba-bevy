//! Build-time market: the storefront rules, services and sign-in one build of
//! OMOBA ships with.
//!
//! One binary serves one market. `scripts/release.py --market <id>`
//! (`scripts/markets.py`) bakes `OMOBA_MARKET` into every crate of the
//! workspace, together with the market's endpoints. Each market is a preset of
//! [`Features`]; a build may flip a single feature with
//! `OMOBA_FEATURE_<NAME>=on|off` (`release.py --feature ekza=on`) without
//! defining a new market. An unknown market or switch value fails the build
//! (a `const` panic), so a typo never ships the wrong storefront.
//!
//! - [`Market::Global`] (no `OMOBA_MARKET`, the default): Europe and the rest of
//!   the world. Everything the game has today: the Ekza ecosystem and paid
//!   Hero Supporter cosmetics. Sign-in is game ↔ website pairing.
//! - [`Market::Ru`]: Russia. A free game on Russian servers: no Ekza services, no
//!   payments, Russian by default, sign-in with VK ID (VK, OK, Mail.ru) through
//!   the market's player portal.
//! - [`Market::Cn`]: mainland China. A free game, no Ekza services, no payments,
//!   Simplified Chinese by default. Its endpoints and sign-in providers are not
//!   decided yet; the release tooling refuses to package it until they are.
//!
//! The market is independent of the build *contour* (`OMOBA_DEFAULT_CONFIG_DIR_SUFFIX`,
//! production or the `dev` tester app): an RU dev build is the OMOBA Dev app with
//! RU features. See `docs/markets.md`.
use serde::{Deserialize, Serialize};

/// A market one build serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    /// Europe and the rest of the world: the full current product.
    Global,
    /// Russia: free, no Ekza, no payments, VK ID sign-in.
    Ru,
    /// Mainland China: free, no Ekza, no payments.
    Cn,
}

/// What a market (or a per-build override) switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Features {
    /// The Ekza ecosystem: the registry catalogue and store avatars, the Ekza
    /// passport (wallet pairing, protected avatars), the Ekza Library and its
    /// skins, market and Studio links, and registry-served map placements
    /// (community sculptures). Off: only the game's packaged content exists and
    /// no Ekza host is ever contacted.
    pub ekza: bool,
    /// Paid cosmetics: Hero Supporter checkout (StoreKit in the game, the
    /// website's Solana checkout). Off: the game is free and shows no purchase.
    pub supporter: bool,
    /// Sign in with VK ID (VK, OK, Mail.ru) through the player portal.
    pub vk_sign_in: bool,
}

/// One switchable feature, as named by `OMOBA_FEATURE_<NAME>` and `release.py --feature`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Ekza,
    Supporter,
    VkSignIn,
}

impl Feature {
    pub const ALL: [Self; 3] = [Self::Ekza, Self::Supporter, Self::VkSignIn];

    /// `release.py --feature <key>=on|off`.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ekza => "ekza",
            Self::Supporter => "supporter",
            Self::VkSignIn => "vk_sign_in",
        }
    }

    /// The compile-time environment variable that overrides the market's value.
    pub const fn env(self) -> &'static str {
        match self {
            Self::Ekza => "OMOBA_FEATURE_EKZA",
            Self::Supporter => "OMOBA_FEATURE_SUPPORTER",
            Self::VkSignIn => "OMOBA_FEATURE_VK_SIGN_IN",
        }
    }
}

impl Market {
    pub const ALL: [Self; 3] = [Self::Global, Self::Ru, Self::Cn];

    /// The `OMOBA_MARKET` value and the name in package and folder names.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Ru => "ru",
            Self::Cn => "cn",
        }
    }

    /// `OMOBA_MARKET`: unset or empty is [`Market::Global`]; an unknown value is `None`.
    pub const fn parse(value: Option<&str>) -> Option<Self> {
        let Some(value) = value else {
            return Some(Self::Global);
        };
        if value.is_empty() || same(value, "global") {
            Some(Self::Global)
        } else if same(value, "ru") {
            Some(Self::Ru)
        } else if same(value, "cn") {
            Some(Self::Cn)
        } else {
            None
        }
    }

    /// The market's preset.
    pub const fn default_features(self) -> Features {
        match self {
            Self::Global => Features {
                ekza: true,
                supporter: true,
                vk_sign_in: false,
            },
            Self::Ru => Features {
                ekza: false,
                supporter: false,
                vk_sign_in: true,
            },
            Self::Cn => Features {
                ekza: false,
                supporter: false,
                vk_sign_in: false,
            },
        }
    }

    /// The UI language on first launch (`client/i18n/<locale>/`); players can
    /// still change it in Settings.
    pub const fn default_locale(self) -> &'static str {
        match self {
            Self::Global => "en",
            Self::Ru => "ru",
            Self::Cn => "zh-Hans",
        }
    }

    /// The market's part of the settings-folder name (`omoba-bevy-ru`): each
    /// market has its own account service, so its device keys and caches never
    /// meet another market's. The global market keeps the historical folder.
    pub const fn storage_suffix(self) -> Option<&'static str> {
        match self {
            Self::Global => None,
            Self::Ru => Some("ru"),
            Self::Cn => Some("cn"),
        }
    }
}

impl Features {
    /// The market's preset with the `OMOBA_FEATURE_*` overrides applied
    /// (`on` / `off`; unset or empty keeps the preset). `None` names the first
    /// invalid override.
    pub const fn resolve(
        market: Market,
        ekza: Option<&str>,
        supporter: Option<&str>,
        vk_sign_in: Option<&str>,
    ) -> Result<Self, Feature> {
        let preset = market.default_features();
        let Some(ekza) = switch(ekza, preset.ekza) else {
            return Err(Feature::Ekza);
        };
        let Some(supporter) = switch(supporter, preset.supporter) else {
            return Err(Feature::Supporter);
        };
        let Some(vk_sign_in) = switch(vk_sign_in, preset.vk_sign_in) else {
            return Err(Feature::VkSignIn);
        };
        Ok(Self {
            ekza,
            supporter,
            vk_sign_in,
        })
    }

    /// One feature's value.
    pub const fn get(self, feature: Feature) -> bool {
        match feature {
            Feature::Ekza => self.ekza,
            Feature::Supporter => self.supporter,
            Feature::VkSignIn => self.vk_sign_in,
        }
    }
}

/// This build's market (`OMOBA_MARKET`).
pub const MARKET: Market = match Market::parse(option_env!("OMOBA_MARKET")) {
    Some(market) => market,
    None => panic!("OMOBA_MARKET must be empty, global, ru or cn"),
};

/// This build's features: [`MARKET`]'s preset with its `OMOBA_FEATURE_*` overrides.
pub const FEATURES: Features = match Features::resolve(
    MARKET,
    option_env!("OMOBA_FEATURE_EKZA"),
    option_env!("OMOBA_FEATURE_SUPPORTER"),
    option_env!("OMOBA_FEATURE_VK_SIGN_IN"),
) {
    Ok(features) => features,
    Err(_) => panic!("OMOBA_FEATURE_* must be empty, on or off"),
};

/// Whether this build talks to the Ekza ecosystem at all.
pub const fn ekza() -> bool {
    FEATURES.ekza
}

/// Whether this build sells anything.
pub const fn supporter() -> bool {
    FEATURES.supporter
}

/// One line for logs and the build-info overlay: `ru (ekza off, supporter off, vk_sign_in on)`.
pub fn describe(market: Market, features: Features) -> String {
    let flags = Feature::ALL
        .iter()
        .map(|f| {
            format!(
                "{} {}",
                f.key(),
                if features.get(*f) { "on" } else { "off" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{} ({flags})", market.key())
}

const fn switch(value: Option<&str>, preset: bool) -> Option<bool> {
    match value {
        None => Some(preset),
        Some(value) if value.is_empty() => Some(preset),
        Some(value) if same(value, "on") => Some(true),
        Some(value) if same(value, "off") => Some(false),
        Some(_) => None,
    }
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_market_is_global_and_unknown_is_refused() {
        assert_eq!(Market::parse(None), Some(Market::Global));
        assert_eq!(Market::parse(Some("")), Some(Market::Global));
        for market in Market::ALL {
            assert_eq!(Market::parse(Some(market.key())), Some(market));
        }
        for bad in ["RU", "eu", "ru ", "russia", "china"] {
            assert_eq!(Market::parse(Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn presets_keep_ekza_and_payments_out_of_ru_and_cn() {
        let global = Market::Global.default_features();
        assert!(global.ekza && global.supporter && !global.vk_sign_in);
        let ru = Market::Ru.default_features();
        assert!(!ru.ekza && !ru.supporter && ru.vk_sign_in);
        let cn = Market::Cn.default_features();
        assert!(!cn.ekza && !cn.supporter && !cn.vk_sign_in);
    }

    #[test]
    fn overrides_flip_one_feature_and_reject_typos() {
        let ru = Features::resolve(Market::Ru, Some("on"), None, Some("")).unwrap();
        assert_eq!(
            ru,
            Features {
                ekza: true,
                supporter: false,
                vk_sign_in: true
            }
        );
        let global = Features::resolve(Market::Global, None, Some("off"), None).unwrap();
        assert!(global.ekza && !global.supporter);
        assert_eq!(
            Features::resolve(Market::Ru, Some("yes"), None, None),
            Err(Feature::Ekza)
        );
        assert_eq!(
            Features::resolve(Market::Ru, None, Some("OFF"), None),
            Err(Feature::Supporter)
        );
        assert_eq!(
            Features::resolve(Market::Ru, None, None, Some("1")),
            Err(Feature::VkSignIn)
        );
    }

    #[test]
    fn locales_and_storage_names_match_the_client_and_release_tooling() {
        assert_eq!(Market::Global.default_locale(), "en");
        assert_eq!(Market::Ru.default_locale(), "ru");
        assert_eq!(Market::Cn.default_locale(), "zh-Hans");
        assert_eq!(Market::Global.storage_suffix(), None);
        assert_eq!(Market::Ru.storage_suffix(), Some("ru"));
        assert_eq!(Market::Cn.storage_suffix(), Some("cn"));
        assert_eq!(
            describe(Market::Ru, Market::Ru.default_features()),
            "ru (ekza off, supporter off, vk_sign_in on)"
        );
    }

    #[test]
    fn feature_env_names_follow_one_pattern() {
        for feature in Feature::ALL {
            assert_eq!(
                feature.env(),
                format!("OMOBA_FEATURE_{}", feature.key().to_ascii_uppercase())
            );
        }
    }
}
