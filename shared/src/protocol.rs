//! Shared additive session envelope. Gameplay entity types remain unchanged.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotMeta {
    pub protocol_version: u16,
    pub server_epoch: u64,
    pub match_id: u64,
    pub snapshot_tick: u64,
}

impl SnapshotMeta {
    pub const fn new(server_epoch: u64, match_id: u64, snapshot_tick: u64) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            server_epoch,
            match_id,
            snapshot_tick,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinRejection {
    MatchFull,
    SessionActive,
    ProtocolMismatch,
}

impl JoinRejection {
    pub const fn message(self) -> &'static str {
        match self {
            Self::MatchFull => "The match is full. Retry when a seat becomes available.",
            Self::SessionActive => {
                "This session is active in another client. Close it, then retry."
            }
            Self::ProtocolMismatch => {
                "Client and server versions differ. Use the same release build."
            }
        }
    }
}

/// Applied on complete snapshots, never on incomplete fragments. Reset only
/// when the transport is replaced; previous epochs cannot reenter that stream.
#[derive(Debug, Default)]
pub struct SnapshotOrder {
    latest: Option<SnapshotMeta>,
}

impl SnapshotOrder {
    pub fn accept(&mut self, next: SnapshotMeta) -> bool {
        if next.protocol_version != PROTOCOL_VERSION || next.server_epoch == 0 || next.match_id == 0
        {
            return false;
        }
        if let Some(last) = self.latest {
            if last.server_epoch == next.server_epoch {
                if next.match_id < last.match_id
                    || (next.match_id == last.match_id && next.snapshot_tick <= last.snapshot_tick)
                {
                    return false;
                }
            } else if next.server_epoch < last.server_epoch {
                // Server epochs use process-start UTC nanoseconds. Keep a
                // high-water mark without retaining an unbounded epoch set.
                // A server clock rollback needs an explicit transport reset.
                return false;
            }
        }
        self.latest = Some(next);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_duplicates_and_previous_rounds_cannot_roll_state_back() {
        let mut order = SnapshotOrder::default();
        assert!(order.accept(SnapshotMeta::new(10, 1, 42)));
        assert!(!order.accept(SnapshotMeta::new(10, 1, 42)));
        assert!(!order.accept(SnapshotMeta::new(10, 1, 41)));
        assert!(order.accept(SnapshotMeta::new(10, 2, 0)));
        assert!(!order.accept(SnapshotMeta::new(10, 1, 999)));
        assert!(order.accept(SnapshotMeta::new(20, 1, 0)));
        assert!(!order.accept(SnapshotMeta::new(10, 3, 999)));
    }

    #[test]
    fn incompatible_or_unidentified_data_is_not_applied() {
        let mut order = SnapshotOrder::default();
        assert!(!order.accept(SnapshotMeta::default()));
        let mut next = SnapshotMeta::new(1, 1, 1);
        next.protocol_version += 1;
        assert!(!order.accept(next));
        assert!(order.accept(SnapshotMeta::new(1, 1, 1)));
    }
}
