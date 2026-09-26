//! Enum evolution policy for the wire (report O10).
//!
//! A strict enum rejects a value it does not know, and the client drops the
//! whole datagram when any field fails to decode. So a new variant on any enum
//! the UDP protocol carries is a compatibility decision, not a local edit:
//!
//! - either bump [`PROTOCOL_VERSION`] (old peers are then refused up front
//!   with `JoinRejection::ProtocolMismatch` instead of silently dropping
//!   packets),
//! - or make that enum tolerant first (`#[serde(other)]` fallback or a
//!   lenient `Deserialize`) in a release that ships before the variant is
//!   ever sent.
//!
//! This module makes the decision unavoidable. [`wire_enums!`] lists every
//! enum reachable from `ClientPacket`, `ServerPacket` and the public transport
//! datagrams with its current variants, and matches each one exhaustively
//! with no `_` arm, so a new variant fails to compile here. The list is
//! pinned to [`POLICY_PROTOCOL_VERSION`]; bumping the protocol fails
//! `policy_matches_the_protocol_version` until this file is reviewed. A new
//! serde enum anywhere in `shared` fails `every_serde_enum_in_shared_is_classified`
//! until it is listed here or in [`NOT_UDP_WIRE`].
//!
//! `CharacterChoice` (`Join.character`, `PlayerState.character`) is owned by
//! this crate since step 13 (it was the SDK's `EkzaCharacter`) and is listed.
use std::path::Path;

use crate::protocol::PROTOCOL_VERSION;

/// The protocol version this variant list was last reviewed for.
const POLICY_PROTOCOL_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decoding {
    /// Unknown values fail to decode; a new variant needs a protocol bump.
    Strict,
    /// Unknown values decode to a fallback; a new variant is additive.
    Tolerant,
}

macro_rules! wire_enums {
    ($($ty:path => $decoding:ident [$($variant:ident),* $(,)?]),* $(,)?) => {
        /// (type path, decoding, variants) for every UDP wire enum.
        const WIRE_ENUMS: &[(&str, Decoding, &[&str])] = &[
            $((stringify!($ty), Decoding::$decoding, &[$(stringify!($variant)),*])),*
        ];

        /// Never called: it exists so a new variant stops the build here.
        #[allow(dead_code)]
        fn exhaustive_variant_lists() {
            $({
                type E = $ty;
                // No `_` arm on purpose: see the module docs before adding one.
                let name = |value: &E| -> &'static str {
                    match value {
                        $(E::$variant { .. } => stringify!($variant),)*
                    }
                };
                let _ = name;
            })*
        }
    };
}

wire_enums! {
    crate::protocol::wire::ClientPacket => Strict [Sandbox, Leave, Social, Party, Career, Hello, Transform, Cast, Utility, BasicAttack, Join, Prematch, Ping, RequestRematch, SetGodMode, SetSpeedBoost, Practice, UpgradeSkill, BuyItem],
    crate::protocol::wire::ServerPacket => Strict [Party, Social, Career, Snapshot],
    crate::party::PartyCommand => Strict [Presence, Invite, Accept, Decline, Leave, Kick, Launch],
    crate::public_transport::PublicClientDatagram => Strict [TransportProbe, TransportProof, TransportBootstrap, SignedCommand],
    crate::public_transport::PublicServerDatagram => Strict [TransportChallenge],
    crate::career::CareerRequest => Strict [FindMatch, SupporterStatus, EquipSupporterAura, Social, Challenge, Authenticate, History, Detail, Friends, Friend, Profile, LookupPlayer, Rename, Authorized, CancelQueue],
    crate::utility::UtilityAction => Strict [Dash, Haste],
    crate::map::Team => Strict [Green, Blue],
    crate::HeroClass => Tolerant [Warrior, Mage, Ranger, Cleric, Warden],
    crate::practice::PracticeCommand => Tolerant [Roster, ClearBots, SpawnDummy, StartDuel, Unsupported],
    crate::protocol::JoinRejection => Strict [MatchFull, SessionActive, ProtocolMismatch, MapGeometryMismatch, AvatarNotAuthorized],
    crate::protocol::wire::GameState => Strict [Lobby, Forming, Starting, Running, Victory],
    crate::sandbox::SandboxCommand => Strict [ApplyConfig, Refill, ResetCooldowns, Teleport, ResetActor, AddXp, GrantItem, ResetDuel, ResetAnalytics, SpawnWave, FrameStep, ForceCast],
    crate::social::SocialCommand => Strict [Subscribe, Chat, Reaction],
    crate::match_service::MatchPreference => Strict [Quick, HumansOnly, BotPractice],
    crate::supporter::AuraStyle => Strict [Solar, Lunar, Verdant],
    crate::career::FriendAction => Strict [Request, Accept, Reject, Cancel, Remove],
    crate::career::CareerAction => Strict [FindMatch, SupporterStatus, EquipSupporterAura, Social, History, Detail, Friends, Friend, Profile, LookupPlayer, Rename, CancelQueue],
    crate::protocol::wire::TargetKind => Strict [Player, Minion, Structure, Neutral],
    crate::prematch::PrematchAction => Strict [Select, Lock, Loaded],
    crate::match_service::MatchServiceView => Strict [Idle, Waiting, Allocating, Assigned, Failed],
    crate::career::QueueView => Strict [Idle, Waiting, Selected, Playing, Full],
    crate::shop::ItemId => Strict [EmberBlade, SwiftGrip, TrailBoots, VitalityGem, FocusCharm, GuardianCrest],
    crate::PlayerActionKind => Tolerant [Attack, Cast, None],
    crate::prematch::PrematchPhase => Strict [Draft, Countdown, Loading],
    crate::combat::CombatEntityKind => Tolerant [Player, Minion, Structure, Neutral, Unknown],
    crate::combat::ProjectileStyle => Tolerant [Arrow, Arcane, Holy, Crescent, Claw, CasterBolt, TowerBolt, Standard],
    crate::map::Lane => Strict [Top, Mid, Bot],
    crate::protocol::wire::StructureKind => Strict [Tower, BaseTower],
    crate::combat::MinionKind => Tolerant [Caster, Melee],
    crate::protocol::wire::MinionBrainState => Strict [Marching, Chasing, Attacking, Dead],
    crate::protocol::wire::MinionTargetKind => Strict [Player, Minion, Structure],
    crate::protocol::wire::NeutralCampType => Strict [Skirmisher, Bruiser, Spitter, WendigoBoss, KingMutatioBoss],
    crate::protocol::wire::NeutralAiState => Strict [Idle, Aggro],
    crate::protocol::wire::TeamBuffKind => Strict [WendigoFavor, MutatioMight],
    crate::sandbox::SandboxActor => Strict [Player, Enemy, Dummy],
    crate::social::SocialChannel => Strict [Team, Match],
    crate::prematch::Role => Strict [Solo, Jungle, Mid, Carry, Support],
    crate::social::SocialTeam => Strict [Green, Blue],
    crate::social::SocialEventKind => Strict [Chat, Reaction],
    crate::career::MatchOutcome => Strict [Completed, Abandoned, Interrupted],
    crate::shop::PurchaseError => Strict [Unavailable, Dead, OutsideBase, InsufficientGold, AlreadyOwned, InventoryFull, UnknownItem],
    crate::career::FriendPresence => Strict [Offline, Online, Playing],
    crate::sandbox::BotBehavior => Strict [Stationary, Flee, Attack, Fight],
    crate::protocol::wire::CharacterChoice => Strict [Ipfs, Toka, Wang, Cube, Paco],
}

/// Serde enums in `shared` that the UDP protocol does not carry, and why.
/// They are versioned by their own contract (HTTP API, data file).
const NOT_UDP_WIRE: &[(&str, &str)] = &[
    ("DeviceAction", "account-api HTTP contract"),
    ("NativeSupporterAction", "account-api HTTP contract"),
    ("WebPairDecision", "account-api HTTP contract"),
    ("PackAccess", "reaction pack catalog data"),
    ("Placement", "map configuration file"),
    ("SkillSlot", "hero catalog data"),
    ("TargetingMode", "hero catalog data"),
];

fn short_name(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path).trim()
}

#[test]
fn policy_matches_the_protocol_version() {
    assert_eq!(
        PROTOCOL_VERSION, POLICY_PROTOCOL_VERSION,
        "PROTOCOL_VERSION changed: review the variant lists in this module, then update POLICY_PROTOCOL_VERSION"
    );
}

#[test]
fn every_listed_enum_is_named_once() {
    let mut names: Vec<&str> = WIRE_ENUMS
        .iter()
        .map(|(path, _, _)| short_name(path))
        .collect();
    names.extend(NOT_UDP_WIRE.iter().map(|(name, _)| *name));
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "an enum is listed twice");
    for (path, _, variants) in WIRE_ENUMS {
        assert!(!variants.is_empty(), "{path} lists no variants");
    }
}

/// Every `pub enum` in `shared/src` that derives or implements `Deserialize`.
fn serde_enums_in_shared() -> Vec<String> {
    fn visit(dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                let lines: Vec<&str> = source.lines().collect();
                for (index, line) in lines.iter().enumerate() {
                    let Some(rest) = line.trim_start().strip_prefix("pub enum ") else {
                        continue;
                    };
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    // The attributes directly above the item.
                    let derives = lines[..index]
                        .iter()
                        .rev()
                        .take_while(|above| {
                            let above = above.trim_start();
                            above.starts_with("#[")
                                || above.starts_with("///")
                                || above.starts_with(')')
                                || above.starts_with('"')
                                || above.ends_with(',')
                        })
                        .any(|above| above.contains("Deserialize"));
                    let manual = source.contains(&format!("Deserialize<'de> for {name} "));
                    if derives || manual {
                        found.push(name);
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    visit(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found.sort();
    found
}

#[test]
fn every_serde_enum_in_shared_is_classified() {
    let found = serde_enums_in_shared();
    assert!(
        found.iter().any(|name| name == "ClientPacket"),
        "scan found {found:?}"
    );
    let unclassified: Vec<&String> = found
        .iter()
        .filter(|name| {
            !WIRE_ENUMS
                .iter()
                .any(|(path, _, _)| short_name(path) == name.as_str())
                && !NOT_UDP_WIRE.iter().any(|(other, _)| other == name)
        })
        .collect();
    assert!(
        unclassified.is_empty(),
        "new serde enums {unclassified:?}: add each to wire_enums! (with its decoding) or to NOT_UDP_WIRE"
    );
}

#[test]
fn tolerant_enums_decode_an_unknown_value() {
    const FUTURE: &str = "\"a_future_variant\"";
    assert_eq!(
        serde_json::from_str::<crate::HeroClass>(FUTURE).unwrap(),
        crate::HeroClass::default()
    );
    assert_eq!(
        serde_json::from_str::<crate::PlayerActionKind>(FUTURE).unwrap(),
        crate::PlayerActionKind::None
    );
    assert_eq!(
        serde_json::from_str::<crate::combat::MinionKind>(FUTURE).unwrap(),
        crate::combat::MinionKind::Melee
    );
    assert_eq!(
        serde_json::from_str::<crate::combat::ProjectileStyle>(FUTURE).unwrap(),
        crate::combat::ProjectileStyle::Standard
    );
    assert_eq!(
        serde_json::from_str::<crate::combat::CombatEntityKind>(FUTURE).unwrap(),
        crate::combat::CombatEntityKind::Unknown
    );
    assert_eq!(
        serde_json::from_str::<crate::practice::PracticeCommand>(r#"{"kind":"a_future_variant"}"#)
            .unwrap(),
        crate::practice::PracticeCommand::Unsupported
    );
    let tolerant: Vec<&str> = WIRE_ENUMS
        .iter()
        .filter(|(_, decoding, _)| *decoding == Decoding::Tolerant)
        .map(|(path, _, _)| short_name(path))
        .collect();
    assert_eq!(
        tolerant,
        [
            "HeroClass",
            "PracticeCommand",
            "PlayerActionKind",
            "CombatEntityKind",
            "ProjectileStyle",
            "MinionKind"
        ],
        "a tolerant enum needs an unknown-value case above"
    );
}

#[test]
fn strict_enums_reject_an_unknown_value() {
    assert!(serde_json::from_str::<crate::map::Team>("\"a_future_variant\"").is_err());
    assert!(
        serde_json::from_str::<crate::protocol::wire::ClientPacket>(
            r#"{"type":"a_future_variant"}"#
        )
        .is_err()
    );
}
