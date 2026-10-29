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
//!   Hero Supporter cosmetics (Apple, Solana). Sign-in is game ↔ website pairing.
//! - [`Market::Ru`]: Russia, on Russian servers: no Ekza services, Hero Supporter
//!   auras sold only through YooKassa donations on the market's player portal
//!   (LR-67, ADR-024), Russian by default, sign-in with VK ID (VK, OK, Mail.ru)
//!   through the same portal.
//! - [`Market::Cn`]: mainland China. A free game, no Ekza services, no payments,
//!   Simplified Chinese by default. Its endpoints and sign-in providers are not
//!   decided yet; the release tooling refuses to package it until they are.
//!
//! The market is independent of the build *contour* (`OMOBA_DEFAULT_CONFIG_DIR_SUFFIX`,
//! production or the `dev` tester app): an RU dev build is the OMOBA Dev app with
//! RU features. See `docs/markets.md`.
/// A market one build serves. Compile-time only: never serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Market {
    /// Europe and the rest of the world: the full current product.
    Global,
    /// Russia: no Ekza, YooKassa supporter donations, VK ID sign-in.
    Ru,
    /// Mainland China: free, no Ekza, no payments.
    Cn,
}

/// What a market (or a per-build override) switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Features {
    /// The Ekza ecosystem: the registry catalogue and store avatars, the Ekza
    /// passport (wallet pairing, protected avatars), the Ekza Library and its
    /// skins, market and Studio links, and registry-served map placements
    /// (community sculptures). Off: only the game's packaged content exists and
    /// no Ekza host is ever contacted.
    pub ekza: bool,
    /// Paid cosmetics: Hero Supporter checkout through the market's
    /// [`PaymentProvider`]s, the Supporter panel and other players' auras. Off:
    /// the game is free and shows no purchase.
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

/// A payment provider that sells Hero Supporter access (`docs/supporter.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaymentProvider {
    /// StoreKit in the iOS app, verified by the Account API.
    Apple,
    /// USDC on the website (`/supporter`).
    Solana,
    /// One-time ruble donations on the website (`/supporter`), «Чеки от ЮKassa».
    YooKassa,
}

impl PaymentProvider {
    pub const ALL: [Self; 3] = [Self::Apple, Self::Solana, Self::YooKassa];

    /// The provider id in grants, events, `scripts/markets.py` and the portal.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Solana => "solana",
            Self::YooKassa => "yookassa",
        }
    }

    /// Whether this provider's grants carry Hero tier points today: YooKassa
    /// (1 ₽ = 1 point). Apple tier products follow in LR-69; Solana carries
    /// none unless O-27 decides otherwise.
    pub const fn carries_tier_points(self) -> bool {
        matches!(self, Self::YooKassa)
    }
}

/// Whether any of `providers` can reach a Hero tier (shows the tier styles).
pub const fn sells_tiers(providers: &[PaymentProvider]) -> bool {
    let mut i = 0;
    while i < providers.len() {
        if providers[i].carries_tier_points() {
            return true;
        }
        i += 1;
    }
    false
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
                supporter: true,
                vk_sign_in: true,
            },
            Self::Cn => Features {
                ekza: false,
                supporter: false,
                vk_sign_in: false,
            },
        }
    }

    /// Who sells Hero Supporter access in this market when its `supporter`
    /// feature is on (ADR-024). LR-68/LR-69 make this per region × channel.
    pub const fn payment_providers(self) -> &'static [PaymentProvider] {
        match self {
            Self::Global => &[PaymentProvider::Apple, PaymentProvider::Solana],
            Self::Ru => &[PaymentProvider::YooKassa],
            Self::Cn => &[],
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

    /// The market's legal documents, source link, player channels and age sign
    /// (LR-06, LR-07, LR-21). `scripts/markets.py` holds the same values for
    /// the packages' README; the portal has its own copy
    /// (omoba-web `market-profile.mjs`). The global market has none, so its
    /// game shows no acceptance screen and no About page.
    pub const fn legal(self) -> Legal {
        match self {
            Self::Global => Legal::NONE,
            Self::Ru => Legal {
                // The Russian drafts (LR-04, plan section 7) until the
                // counsel-approved texts replace them with a new version.
                version: Some("0-draft"),
                privacy: Some("https://omoba.ru/privacy"),
                terms: Some("https://omoba.ru/terms"),
                source: Some("https://github.com/o-moba/omoba-bevy/tree/v{version}-ru"),
                // The owner provides these (LR-21); empty until then.
                support_email: None,
                community_vk: None,
                community_telegram: None,
                // Placeholder the owner confirms with counsel (LR-07).
                age_rating: Some("12+"),
            },
            Self::Cn => Legal::NONE,
        }
    }
}

/// A market's legal documents, source link, player channels and age sign
/// ([`Market::legal`]). `None` is "not provided": nothing is shown for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Legal {
    /// The version of the user agreement and privacy policy a player accepts
    /// on first launch. A new version asks again; `None` asks nothing.
    pub version: Option<&'static str>,
    /// The privacy policy page.
    pub privacy: Option<&'static str>,
    /// The user agreement (terms of use) page.
    pub terms: Option<&'static str>,
    /// The public source tree of a release; `{version}` stands for the release
    /// (`0.45.0`, see [`Legal::source_for`]). The tag is `release.py draft`'s.
    pub source: Option<&'static str>,
    /// The support e-mail address.
    pub support_email: Option<&'static str>,
    /// The market's VK community page.
    pub community_vk: Option<&'static str>,
    /// The market's Telegram channel.
    pub community_telegram: Option<&'static str>,
    /// The age category sign (436-FZ in Russia), for example `12+`.
    pub age_rating: Option<&'static str>,
}

impl Legal {
    /// Nothing to show and nothing to accept.
    pub const NONE: Self = Self {
        version: None,
        privacy: None,
        terms: None,
        source: None,
        support_email: None,
        community_vk: None,
        community_telegram: None,
        age_rating: None,
    };

    /// The source link of `release` (`0.45.0` →
    /// `https://github.com/o-moba/omoba-bevy/tree/v0.45.0-ru`).
    pub fn source_for(&self, release: &str) -> Option<String> {
        self.source
            .map(|template| template.replace("{version}", release))
    }

    /// Whether a player has anything to read: a document, a link or the sign.
    pub const fn any(&self) -> bool {
        self.version.is_some()
            || self.privacy.is_some()
            || self.terms.is_some()
            || self.source.is_some()
            || self.support_email.is_some()
            || self.community_vk.is_some()
            || self.community_telegram.is_some()
            || self.age_rating.is_some()
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
        // VK is an RU account service. A generic feature override must never
        // expose it in the international or CN region.
        if vk_sign_in && !matches!(market, Market::Ru) {
            return Err(Feature::VkSignIn);
        }
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
    Err(Feature::VkSignIn) => {
        panic!("OMOBA_FEATURE_VK_SIGN_IN must be empty, on or off; on requires OMOBA_MARKET=ru")
    }
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

/// The payment providers this build sells through: [`MARKET`]'s list while
/// `supporter` is on, none otherwise.
pub const fn payment_providers() -> &'static [PaymentProvider] {
    providers_of(MARKET, FEATURES)
}

/// A market's providers under a feature set (`supporter` off sells nothing).
pub const fn providers_of(market: Market, features: Features) -> &'static [PaymentProvider] {
    if features.supporter {
        market.payment_providers()
    } else {
        &[]
    }
}

/// Whether `providers` lists `provider`.
pub const fn sells(providers: &[PaymentProvider], provider: PaymentProvider) -> bool {
    let mut i = 0;
    while i < providers.len() {
        if providers[i] as u8 == provider as u8 {
            return true;
        }
        i += 1;
    }
    false
}

/// One line for logs and the build-info overlay: `ru (ekza off, supporter on, vk_sign_in on)`.
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
    fn presets_keep_ekza_out_of_ru_and_cn_and_payments_out_of_cn() {
        let global = Market::Global.default_features();
        assert!(global.ekza && global.supporter && !global.vk_sign_in);
        let ru = Market::Ru.default_features();
        assert!(!ru.ekza && ru.supporter && ru.vk_sign_in);
        let cn = Market::Cn.default_features();
        assert!(!cn.ekza && !cn.supporter && !cn.vk_sign_in);
    }

    #[test]
    fn ru_sells_only_through_yookassa_and_global_keeps_apple_and_solana() {
        use PaymentProvider::*;
        assert_eq!(Market::Global.payment_providers(), &[Apple, Solana]);
        assert_eq!(Market::Ru.payment_providers(), &[YooKassa]);
        assert!(Market::Cn.payment_providers().is_empty());
        for market in Market::ALL {
            let features = market.default_features();
            assert_eq!(
                providers_of(market, features),
                market.payment_providers(),
                "{market:?}"
            );
            // Turning `supporter` off sells nothing, whatever the market lists.
            let free = Features {
                supporter: false,
                ..features
            };
            assert!(providers_of(market, free).is_empty(), "{market:?}");
        }
        assert!(sells(&[Apple, Solana], Solana) && !sells(&[Apple, Solana], YooKassa));
        assert!(sells(&[YooKassa], YooKassa) && !sells(&[YooKassa], Apple));
        assert!(!sells(&[], Apple));
        assert!(sells_tiers(Market::Ru.payment_providers()));
        assert!(!sells_tiers(Market::Global.payment_providers()));
        assert!(!sells_tiers(&[]));
        assert_eq!(
            PaymentProvider::ALL.map(PaymentProvider::key),
            ["apple", "solana", "yookassa"]
        );
    }

    #[test]
    fn overrides_flip_one_feature_and_reject_typos() {
        let ru = Features::resolve(Market::Ru, Some("on"), Some("off"), Some("")).unwrap();
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
    fn vk_cannot_be_enabled_outside_the_ru_account_region() {
        for market in [Market::Global, Market::Cn] {
            assert_eq!(
                Features::resolve(market, None, None, Some("on")),
                Err(Feature::VkSignIn)
            );
            for value in [None, Some(""), Some("off")] {
                assert!(
                    !Features::resolve(market, None, None, value)
                        .unwrap()
                        .vk_sign_in
                );
            }
        }
        assert!(
            Features::resolve(Market::Ru, None, None, Some("on"))
                .unwrap()
                .vk_sign_in
        );
        assert!(
            !Features::resolve(Market::Ru, None, None, Some("off"))
                .unwrap()
                .vk_sign_in
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
        // Pinned: the RU line changed with LR-67 (supporter on); the portal's
        // `describeMarket` prints the same line.
        assert_eq!(
            describe(Market::Ru, Market::Ru.default_features()),
            "ru (ekza off, supporter on, vk_sign_in on)"
        );
        assert_eq!(
            describe(Market::Global, Market::Global.default_features()),
            "global (ekza on, supporter on, vk_sign_in off)"
        );
        assert_eq!(
            describe(Market::Cn, Market::Cn.default_features()),
            "cn (ekza off, supporter off, vk_sign_in off)"
        );
    }

    #[test]
    fn only_ru_has_legal_documents_and_its_links_stay_on_its_portal_or_source() {
        assert_eq!(Market::Global.legal(), Legal::NONE);
        assert_eq!(Market::Cn.legal(), Legal::NONE);
        assert!(!Legal::NONE.any());
        let ru = Market::Ru.legal();
        assert!(ru.any());
        assert_eq!(ru.version, Some("0-draft"));
        assert_eq!(ru.age_rating, Some("12+"));
        for link in [ru.privacy, ru.terms].into_iter().flatten() {
            assert!(link.starts_with("https://omoba.ru/"), "{link}");
        }
        assert_eq!(
            ru.source_for("0.45.0").as_deref(),
            Some("https://github.com/o-moba/omoba-bevy/tree/v0.45.0-ru")
        );
        assert_eq!(Legal::NONE.source_for("0.45.0"), None);
        // Hidden until the owner provides them (LR-21).
        assert_eq!(
            (ru.support_email, ru.community_vk, ru.community_telegram),
            (None, None, None)
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
