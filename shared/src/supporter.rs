//! Cosmetic-only Supporter contract. Status is issued by the authoritative server.
//!
//! Hero tiers (LR-67, ADR-024/ADR-025): every grant carries provider-neutral
//! tier points (YooKassa: one point per whole ruble paid; Apple tier products
//! their threshold, LR-69; Solana none until O-27). A player's tier is the sum
//! of the points of the grants covering now ([`SupporterTier::for_points`]; the
//! Account API sums them in SQL with `portal.supporter_points(profile, at)`).
//! The base styles (Solar, Lunar, Verdant) need any active grant; each tier
//! style needs its tier's points. One rule decides what a player may wear
//! ([`AuraStyle::unlocked`]), what is shown when the stored choice is no longer
//! allowed ([`AuraStyle::effective`]) and what the server replicates
//! ([`SupporterStatus::authorized_aura`]).
use serde::{Deserialize, Serialize};

/// Both readers of `portal.supporter_grants` (the Account API dashboard and the
/// game's career store) return at most this many grant rows, active ones first.
pub const STATUS_GRANT_LIMIT: i64 = 64;

/// An aura style. A Strict wire enum (`protocol/wire_enums.rs`): a new variant
/// needs a protocol bump (the seven tier styles came with protocol 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuraStyle {
    Solar,
    Lunar,
    Verdant,
    Rose,
    Jade,
    Azure,
    Crimson,
    #[serde(rename = "prism_i")]
    PrismI,
    #[serde(rename = "prism_ii")]
    PrismII,
    #[serde(rename = "prism_iii")]
    PrismIII,
}
impl AuraStyle {
    /// Base styles first, then the tier styles from Rose to Prism III.
    pub const ALL: [Self; 10] = [
        Self::Solar,
        Self::Lunar,
        Self::Verdant,
        Self::Rose,
        Self::Jade,
        Self::Azure,
        Self::Crimson,
        Self::PrismI,
        Self::PrismII,
        Self::PrismIII,
    ];
    /// The styles any active grant unlocks.
    pub const BASE: [Self; 3] = [Self::Solar, Self::Lunar, Self::Verdant];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Solar => "solar",
            Self::Lunar => "lunar",
            Self::Verdant => "verdant",
            Self::Rose => "rose",
            Self::Jade => "jade",
            Self::Azure => "azure",
            Self::Crimson => "crimson",
            Self::PrismI => "prism_i",
            Self::PrismII => "prism_ii",
            Self::PrismIII => "prism_iii",
        }
    }
    /// English working name (the client translates `aura.<id>`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solar => "Solar",
            Self::Lunar => "Lunar",
            Self::Verdant => "Verdant",
            Self::Rose => "Rose",
            Self::Jade => "Jade",
            Self::Azure => "Azure",
            Self::Crimson => "Crimson",
            Self::PrismI => "Prism I",
            Self::PrismII => "Prism II",
            Self::PrismIII => "Prism III",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.id() == id)
    }
    /// The tier a tier style belongs to; `None` for a base style.
    pub const fn tier(self) -> Option<SupporterTier> {
        match self {
            Self::Solar | Self::Lunar | Self::Verdant => None,
            Self::Rose => Some(SupporterTier::Rose),
            Self::Jade => Some(SupporterTier::Jade),
            Self::Azure => Some(SupporterTier::Azure),
            Self::Crimson => Some(SupporterTier::Crimson),
            Self::PrismI => Some(SupporterTier::PrismI),
            Self::PrismII => Some(SupporterTier::PrismII),
            Self::PrismIII => Some(SupporterTier::PrismIII),
        }
    }
    /// Points this style needs on top of an active grant (0 for a base style).
    pub const fn required_points(self) -> u32 {
        match self.tier() {
            Some(tier) => tier.points(),
            None => 0,
        }
    }
    /// The entitlement rule: a base style needs any active grant, a tier style
    /// also needs its tier's points.
    pub const fn unlocked(self, active: bool, points: u32) -> bool {
        active && points >= self.required_points()
    }
    /// The best style `points` unlock for an active supporter: the tier's own
    /// style, or Solar below the first tier. The default after a first grant
    /// (Account API `apply_event_tx` and the career store both use it).
    pub fn highest_unlocked(points: u32) -> Self {
        SupporterTier::for_points(points).map_or(Self::Solar, SupporterTier::style)
    }
    /// What a player wears given the stored preference (`None`: no row yet,
    /// `Some(None)`: hidden). A choice the tier no longer allows steps down to
    /// [`Self::highest_unlocked`]; nothing without an active grant.
    pub fn effective(preference: Option<Option<Self>>, active: bool, points: u32) -> Option<Self> {
        if !active {
            return None;
        }
        match preference {
            Some(None) => None,
            Some(Some(style)) if style.unlocked(true, points) => Some(style),
            _ => Some(Self::highest_unlocked(points)),
        }
    }
}

/// A monthly Hero tier: the sum of the tier points of the grants covering now.
/// Names are working names; the owner may rename them (LR-67).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SupporterTier {
    Rose,
    Jade,
    Azure,
    Crimson,
    PrismI,
    PrismII,
    PrismIII,
}
impl SupporterTier {
    /// Lowest first.
    pub const ALL: [Self; 7] = [
        Self::Rose,
        Self::Jade,
        Self::Azure,
        Self::Crimson,
        Self::PrismI,
        Self::PrismII,
        Self::PrismIII,
    ];
    /// The points a tier needs (YooKassa: rubles inside one 30-day window).
    pub const fn points(self) -> u32 {
        match self {
            Self::Rose => 100,
            Self::Jade => 250,
            Self::Azure => 500,
            Self::Crimson => 1000,
            Self::PrismI => 2500,
            Self::PrismII => 5000,
            Self::PrismIII => 10000,
        }
    }
    /// The highest tier `points` reach; `None` below 100.
    pub fn for_points(points: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .rev()
            .find(|tier| points >= tier.points())
    }
    /// The aura style this tier unlocks.
    pub const fn style(self) -> AuraStyle {
        match self {
            Self::Rose => AuraStyle::Rose,
            Self::Jade => AuraStyle::Jade,
            Self::Azure => AuraStyle::Azure,
            Self::Crimson => AuraStyle::Crimson,
            Self::PrismI => AuraStyle::PrismI,
            Self::PrismII => AuraStyle::PrismII,
            Self::PrismIII => AuraStyle::PrismIII,
        }
    }
    pub const fn id(self) -> &'static str {
        self.style().id()
    }
}

/// A database sum (bigint) as tier points: negative is 0, huge saturates.
pub fn tier_points(sum: i64) -> u32 {
    u32::try_from(sum.max(0)).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupporterGrantSummary {
    pub provider: String,
    pub valid_from: i64,
    pub valid_until: i64,
    pub revoked: bool,
    /// False means no future renewal, not immediate loss of paid access.
    pub renewal_enabled: Option<bool>,
    /// Provider-neutral tier points this grant adds while it covers a moment
    /// (protocol 12; older payloads decode as 0).
    #[serde(default)]
    pub tier_points: u32,
}
impl SupporterGrantSummary {
    /// Whether this grant counts at `at` (`[valid_from, valid_until)`, not revoked).
    pub fn covers(&self, at: i64) -> bool {
        !self.revoked && self.valid_from <= at && self.valid_until > at
    }
}

/// Tier points of the grants covering `at`.
pub fn points_at(grants: &[SupporterGrantSummary], at: i64) -> u32 {
    grants
        .iter()
        .filter(|grant| grant.covers(at))
        .fold(0u32, |sum, grant| sum.saturating_add(grant.tier_points))
}

/// The end of the tier that `points` reach at `now`: a sweep over the grant
/// ends after `now`, returning the first one where the covering points fall
/// below the tier's threshold. `None` without a tier.
pub fn tier_until(grants: &[SupporterGrantSummary], now: i64, points: u32) -> Option<i64> {
    let tier = SupporterTier::for_points(points)?;
    let mut ends: Vec<i64> = grants
        .iter()
        .filter(|grant| !grant.revoked && grant.valid_until > now && grant.tier_points > 0)
        .map(|grant| grant.valid_until)
        .collect();
    ends.sort_unstable();
    ends.dedup();
    ends.into_iter()
        .find(|end| points_at(grants, *end) < tier.points())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupporterStatus {
    pub active: bool,
    pub equipped_aura: Option<AuraStyle>,
    /// UTC epoch seconds, never a client entitlement assertion.
    pub active_until: Option<i64>,
    pub grants: Vec<SupporterGrantSummary>,
    /// Points of the grants covering the moment this status was read (protocol 12).
    #[serde(default)]
    pub tier_points: u32,
    /// When the current tier ends (UTC epoch seconds); `None` without a tier.
    #[serde(default)]
    pub tier_until: Option<i64>,
}
impl SupporterStatus {
    /// The status both grant readers return: `grants` as read (active first,
    /// at most [`STATUS_GRANT_LIMIT`]), `points` from `portal.supporter_points`
    /// at `now`, and the stored aura preference (`None`: no row).
    pub fn from_grants(
        grants: Vec<SupporterGrantSummary>,
        now: i64,
        points: u32,
        preference: Option<Option<AuraStyle>>,
    ) -> Self {
        let active_until = grants
            .iter()
            .filter(|grant| grant.covers(now))
            .map(|grant| grant.valid_until)
            .max();
        let active = active_until.is_some();
        let tier_points = if active { points } else { 0 };
        Self {
            active,
            equipped_aura: AuraStyle::effective(preference, active, tier_points),
            active_until,
            tier_until: tier_until(&grants, now, tier_points),
            grants,
            tier_points,
        }
    }
    /// The current tier.
    pub fn tier(&self) -> Option<SupporterTier> {
        if self.active {
            SupporterTier::for_points(self.tier_points)
        } else {
            None
        }
    }
    /// The tier after [`Self::tier_until`] (the step-down), from the grant list.
    pub fn next_tier(&self) -> Option<SupporterTier> {
        SupporterTier::for_points(points_at(&self.grants, self.tier_until?))
    }
    /// Points that still hold at `now`: past `tier_until` only what the grant
    /// list still covers, never more than was read.
    pub fn points_at(&self, now: i64) -> u32 {
        if self.tier_until.is_some_and(|until| until > now) {
            self.tier_points
        } else {
            self.tier_points.min(points_at(&self.grants, now))
        }
    }
    /// The aura the server may replicate at `now`: the equipped style while a
    /// grant is active and, for a tier style, while its points still hold.
    pub fn authorized_aura(&self, now: i64) -> Option<AuraStyle> {
        if !self.active || self.active_until.is_none_or(|until| until <= now) {
            return None;
        }
        let style = self.equipped_aura?;
        style.unlocked(true, self.points_at(now)).then_some(style)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeSupporterAction {
    Status,
    Equip { aura: Option<AuraStyle> },
    ApplePrepare,
    AppleVerify { signed_payload: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeSupporterRequest {
    pub origin: String,
    pub public_key: String,
    pub nonce: String,
    pub expires_at: u64,
    pub action: NativeSupporterAction,
}
impl NativeSupporterRequest {
    pub fn validate(&self, origin: &str, key: &str, now: u64) -> Result<(), &'static str> {
        if self.origin != origin
            || self.public_key != key
            || !crate::web_account::lowercase_hex(key, 32)
            || !crate::web_account::lowercase_hex(&self.nonce, 32)
            || self.expires_at <= now
            || self.expires_at > now.saturating_add(60)
        {
            return Err("Invalid or expired supporter request");
        }
        if let NativeSupporterAction::AppleVerify { signed_payload } = &self.action {
            if signed_payload.len() > 32768 || signed_payload.is_empty() {
                return Err("Invalid Apple payload");
            }
        }
        Ok(())
    }
    pub fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "omoba.supporter.native.v1",
            &self.origin,
            &self.public_key,
            &self.nonce,
            self.expires_at,
            &self.action,
        ))
        .expect("typed supporter request")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedNativeSupporterRequest {
    pub request: NativeSupporterRequest,
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proof_binds_every_operation_and_never_accepts_unknown_auras() {
        let mut r = NativeSupporterRequest {
            origin: "https://portal.example".into(),
            public_key: "a".repeat(64),
            nonce: "b".repeat(64),
            expires_at: 160,
            action: NativeSupporterAction::Status,
        };
        assert!(r.validate(&r.origin, &r.public_key, 100).is_ok());
        assert!(r.validate(&r.origin, &r.public_key, 99).is_err());
        assert!(r.validate(&r.origin, &r.public_key, 160).is_err());
        let bytes = r.signing_bytes();
        r.action = NativeSupporterAction::ApplePrepare;
        assert_ne!(bytes, r.signing_bytes());
        assert!(serde_json::from_str::<AuraStyle>("\"admin\"").is_err());
    }

    fn grant(from: i64, until: i64, points: u32) -> SupporterGrantSummary {
        SupporterGrantSummary {
            provider: "yookassa".into(),
            valid_from: from,
            valid_until: until,
            revoked: false,
            renewal_enabled: Some(false),
            tier_points: points,
        }
    }

    #[test]
    fn tier_thresholds_are_the_monthly_ruble_ladder() {
        assert_eq!(SupporterTier::for_points(0), None);
        assert_eq!(SupporterTier::for_points(99), None);
        for (points, tier) in [
            (100, SupporterTier::Rose),
            (249, SupporterTier::Rose),
            (250, SupporterTier::Jade),
            (499, SupporterTier::Jade),
            (500, SupporterTier::Azure),
            (999, SupporterTier::Azure),
            (1000, SupporterTier::Crimson),
            (2499, SupporterTier::Crimson),
            (2500, SupporterTier::PrismI),
            (5000, SupporterTier::PrismII),
            (9999, SupporterTier::PrismII),
            (10000, SupporterTier::PrismIII),
            (u32::MAX, SupporterTier::PrismIII),
        ] {
            assert_eq!(SupporterTier::for_points(points), Some(tier), "{points}");
        }
        assert_eq!(
            SupporterTier::ALL.map(SupporterTier::points),
            [100, 250, 500, 1000, 2500, 5000, 10000]
        );
        for tier in SupporterTier::ALL {
            assert_eq!(tier.style().tier(), Some(tier));
            assert_eq!(SupporterTier::for_points(tier.points()), Some(tier));
            assert_eq!(AuraStyle::from_id(tier.id()), Some(tier.style()));
        }
        assert_eq!(tier_points(-5), 0);
        assert_eq!(tier_points(1000), 1000);
        assert_eq!(tier_points(i64::MAX), u32::MAX);
    }

    #[test]
    fn wire_ids_are_stable_and_round_trip() {
        let ids: Vec<String> = AuraStyle::ALL
            .iter()
            .map(|style| serde_json::to_string(style).unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "\"solar\"",
                "\"lunar\"",
                "\"verdant\"",
                "\"rose\"",
                "\"jade\"",
                "\"azure\"",
                "\"crimson\"",
                "\"prism_i\"",
                "\"prism_ii\"",
                "\"prism_iii\"",
            ]
        );
        for style in AuraStyle::ALL {
            assert_eq!(
                serde_json::to_string(&style).unwrap(),
                format!("\"{}\"", style.id())
            );
            assert_eq!(AuraStyle::from_id(style.id()), Some(style));
        }
        assert!(AuraStyle::from_id("prism_i_i").is_none());
    }

    #[test]
    fn base_styles_need_any_active_grant_and_tier_styles_need_points() {
        for style in AuraStyle::BASE {
            assert!(style.unlocked(true, 0), "{style:?}");
            assert!(!style.unlocked(false, 0));
            assert!(!style.unlocked(false, 99_999));
        }
        assert!(!AuraStyle::Rose.unlocked(true, 99));
        assert!(AuraStyle::Rose.unlocked(true, 100));
        assert!(AuraStyle::Crimson.unlocked(true, 1000));
        assert!(!AuraStyle::PrismI.unlocked(true, 1000));
        assert!(!AuraStyle::Crimson.unlocked(false, 1000));
        // Wearing any style up to the tier: a Crimson supporter keeps Rose.
        for style in [AuraStyle::Rose, AuraStyle::Jade, AuraStyle::Azure] {
            assert!(style.unlocked(true, 1000));
        }
        assert_eq!(AuraStyle::highest_unlocked(0), AuraStyle::Solar);
        assert_eq!(AuraStyle::highest_unlocked(99), AuraStyle::Solar);
        assert_eq!(AuraStyle::highest_unlocked(1000), AuraStyle::Crimson);
        assert_eq!(AuraStyle::highest_unlocked(12_000), AuraStyle::PrismIII);
    }

    #[test]
    fn stored_choices_step_down_and_hidden_stays_hidden() {
        use AuraStyle::*;
        assert_eq!(AuraStyle::effective(None, true, 1000), Some(Crimson));
        assert_eq!(AuraStyle::effective(None, true, 0), Some(Solar));
        assert_eq!(AuraStyle::effective(Some(None), true, 1000), None);
        assert_eq!(
            AuraStyle::effective(Some(Some(Lunar)), true, 0),
            Some(Lunar)
        );
        assert_eq!(
            AuraStyle::effective(Some(Some(Rose)), true, 1000),
            Some(Rose)
        );
        // A Prism choice after the tier fell to Rose shows Rose.
        assert_eq!(
            AuraStyle::effective(Some(Some(PrismI)), true, 120),
            Some(Rose)
        );
        assert_eq!(
            AuraStyle::effective(Some(Some(Crimson)), true, 0),
            Some(Solar)
        );
        assert_eq!(AuraStyle::effective(Some(Some(Crimson)), false, 1000), None);
        assert_eq!(AuraStyle::effective(None, false, 0), None);
    }

    #[test]
    fn the_window_sums_covering_grants_and_steps_down_as_they_expire() {
        let now = 1_000_000;
        let day = 86_400;
        // 900 ₽ on day 0 and 300 ₽ on day 10: 1200 points = Crimson until the
        // first grant ends (day 30), then 300 = Jade until day 40.
        let grants = vec![
            grant(now - 20 * day, now + 10 * day, 900),
            grant(now - 10 * day, now + 20 * day, 300),
        ];
        assert_eq!(points_at(&grants, now), 1200);
        assert_eq!(tier_until(&grants, now, 1200), Some(now + 10 * day));
        let status = SupporterStatus::from_grants(grants.clone(), now, 1200, None);
        assert!(status.active);
        assert_eq!(status.tier(), Some(SupporterTier::Crimson));
        assert_eq!(status.tier_until, Some(now + 10 * day));
        assert_eq!(status.next_tier(), Some(SupporterTier::Jade));
        assert_eq!(status.active_until, Some(now + 20 * day));
        assert_eq!(status.equipped_aura, Some(AuraStyle::Crimson));
        // Expiry: one second after the first grant ends only 300 points remain.
        assert_eq!(points_at(&grants, now + 10 * day), 300);
        let later = SupporterStatus::from_grants(grants.clone(), now + 10 * day, 300, None);
        assert_eq!(later.tier(), Some(SupporterTier::Jade));
        assert_eq!(later.tier_until, Some(now + 20 * day));
        assert_eq!(later.next_tier(), None);
        // 100 ₽ every month stays Rose: the next donation (not yet covering
        // now) starts before the current one ends, so the tier runs to its end.
        let monthly = vec![
            grant(now - 25 * day, now + 5 * day, 100),
            grant(now + 4 * day, now + 34 * day, 100),
        ];
        assert_eq!(points_at(&monthly, now), 100);
        assert_eq!(points_at(&monthly, now + 5 * day), 100);
        assert_eq!(tier_until(&monthly, now, 100), Some(now + 34 * day));
        // While both cover (200 points) the tier is still Rose.
        assert_eq!(
            tier_until(&monthly, now + 4 * day, 200),
            Some(now + 34 * day)
        );
        assert_eq!(tier_until(&[], now, 0), None);
        assert_eq!(tier_until(&grants, now, 50), None);
    }

    #[test]
    fn partial_refunds_and_revocations_leave_the_sum() {
        let now = 50_000;
        let mut grants = vec![
            grant(now - 10, now + 100, 1000),
            grant(now - 5, now + 200, 0),
        ];
        // A partial refund recomputes the grant's points (1000 ₽ - 600 ₽).
        grants[0].tier_points = 400;
        assert_eq!(points_at(&grants, now), 400);
        assert_eq!(SupporterTier::for_points(400), Some(SupporterTier::Jade));
        // A full refund revokes the grant: the zero-point grant keeps base access.
        grants[0].revoked = true;
        let status = SupporterStatus::from_grants(grants, now, 0, Some(Some(AuraStyle::Crimson)));
        assert!(status.active);
        assert_eq!(status.tier(), None);
        assert_eq!(status.tier_until, None);
        assert_eq!(status.equipped_aura, Some(AuraStyle::Solar));
    }

    #[test]
    fn the_server_replicates_a_tier_style_only_while_its_points_hold() {
        let now = 10_000;
        let grants = vec![
            grant(now - 10, now + 100, 900),
            grant(now - 10, now + 300, 300),
        ];
        let mut status =
            SupporterStatus::from_grants(grants, now, 1200, Some(Some(AuraStyle::Crimson)));
        assert_eq!(status.authorized_aura(now), Some(AuraStyle::Crimson));
        // Past `tier_until` the cached Crimson is refused until the next refresh;
        // Jade is still within the remaining 300 points.
        assert_eq!(status.authorized_aura(now + 100), None);
        status.equipped_aura = Some(AuraStyle::Jade);
        assert_eq!(status.authorized_aura(now + 100), Some(AuraStyle::Jade));
        // A tampered status is re-checked: no points, no tier style.
        status.equipped_aura = Some(AuraStyle::PrismI);
        assert_eq!(status.authorized_aura(now), None);
        status.equipped_aura = Some(AuraStyle::Lunar);
        assert_eq!(status.authorized_aura(now + 299), Some(AuraStyle::Lunar));
        assert_eq!(status.authorized_aura(now + 300), None);
        status.active = false;
        assert_eq!(status.authorized_aura(now), None);
    }

    #[test]
    fn statuses_without_tier_fields_still_decode() {
        let old = r#"{"active":true,"equipped_aura":"lunar","active_until":200,"grants":[{"provider":"apple","valid_from":100,"valid_until":200,"revoked":false,"renewal_enabled":true}]}"#;
        let status: SupporterStatus = serde_json::from_str(old).unwrap();
        assert_eq!(status.tier_points, 0);
        assert_eq!(status.tier_until, None);
        assert_eq!(status.grants[0].tier_points, 0);
        assert_eq!(status.equipped_aura, Some(AuraStyle::Lunar));
        let new = serde_json::to_string(&SupporterStatus {
            tier_points: 1000,
            tier_until: Some(300),
            ..status
        })
        .unwrap();
        assert!(
            new.ends_with(r#""tier_points":1000,"tier_until":300}"#),
            "{new}"
        );
    }
}
