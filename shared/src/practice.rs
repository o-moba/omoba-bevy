//! Practice-only sandbox commands. The local bot practice server accepts
//! them from a joined human; release and dev matches ignore them entirely.
//! Part of the debug command family (`crate::debug`), which re-exports them.
use serde::{Deserialize, Serialize};

/// Highest hero level the duel opponent may be configured to (mirrors the
/// server's `MAX_LEVEL`).
pub const DUEL_MAX_LEVEL: u32 = 10;
pub const DUEL_MIN_LEVEL: u32 = 1;
/// Gold step and cap for the duel opponent's shopping budget. The cap buys a
/// full inventory of any items (pinned by a test against the catalog).
pub const DUEL_GOLD_STEP: u32 = 100;
pub const DUEL_MAX_GOLD: u32 = 1_000;
/// Stationary dummies alive at once; older ones are recycled first.
pub const MAX_DUMMIES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PracticeCommand {
    /// Standard practice: both teams refill with lane bots.
    Roster,
    /// Remove every bot; only humans remain (minions still spawn).
    ClearBots,
    /// A stationary enemy target in front of the requester. It never moves
    /// or attacks and returns to its spot after each respawn.
    SpawnDummy,
    /// Clear the bots and send one enemy lane bot down mid at `level`, with
    /// its skills ranked for that level and `gold` spent on items at base.
    StartDuel { level: u32, gold: u32 },
    /// A kind this build does not know, such as a newer client's command.
    /// It decodes instead of failing the whole packet, so new kinds are
    /// additive; every host ignores it. Never sent.
    #[serde(other)]
    Unsupported,
}

impl PracticeCommand {
    /// Clamp menu values into the supported range before they go on the wire.
    pub fn duel(level: u32, gold: u32) -> Self {
        Self::StartDuel {
            level: level.clamp(DUEL_MIN_LEVEL, DUEL_MAX_LEVEL),
            gold: gold.min(DUEL_MAX_GOLD),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop::{INVENTORY_CAPACITY, items};

    #[test]
    fn duel_gold_cap_buys_a_full_inventory() {
        // The dearest items that fit in the inventory, so any shopping order
        // fills every slot within the cap.
        let mut costs: Vec<u32> = items().iter().map(|item| item.cost).collect();
        costs.sort_unstable_by(|a, b| b.cmp(a));
        assert!(costs.len() >= INVENTORY_CAPACITY);
        let dearest_full_inventory: u32 = costs.iter().take(INVENTORY_CAPACITY).sum();
        assert!(
            dearest_full_inventory <= DUEL_MAX_GOLD,
            "{dearest_full_inventory} gold for a full inventory exceeds the duel cap"
        );
    }

    #[test]
    fn duel_values_are_clamped_and_round_trip_on_the_wire() {
        assert_eq!(
            PracticeCommand::duel(0, 5_000),
            PracticeCommand::StartDuel {
                level: DUEL_MIN_LEVEL,
                gold: DUEL_MAX_GOLD
            }
        );
        let json = serde_json::to_string(&PracticeCommand::duel(7, 300)).unwrap();
        assert_eq!(json, r#"{"kind":"start_duel","level":7,"gold":300}"#);
        assert_eq!(
            serde_json::from_str::<PracticeCommand>(r#"{"kind":"spawn_dummy"}"#).unwrap(),
            PracticeCommand::SpawnDummy
        );
    }
}
