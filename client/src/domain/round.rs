//! Identity of one round: a server process epoch plus a match on it.

use shared::protocol::SnapshotMeta;

/// One round as the server numbers it. A new server process gets a new
/// `server_epoch`; each match on it gets a new `match_id`. Snapshots taken
/// outside a round (lobby, teardown) carry zero in either part and have no
/// round identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoundId {
    pub server_epoch: u64,
    pub match_id: u64,
}

impl RoundId {
    /// The round a snapshot belongs to, or `None` when either part is zero.
    pub fn from_meta(meta: &SnapshotMeta) -> Option<Self> {
        let round = Self {
            server_epoch: meta.server_epoch,
            match_id: meta.match_id,
        };
        (round.server_epoch != 0 && round.match_id != 0).then_some(round)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_in_either_part_has_no_round() {
        assert_eq!(RoundId::from_meta(&SnapshotMeta::default()), None);
        assert_eq!(RoundId::from_meta(&SnapshotMeta::new(0, 3, 9)), None);
        assert_eq!(RoundId::from_meta(&SnapshotMeta::new(7, 0, 9)), None);
        assert_eq!(
            RoundId::from_meta(&SnapshotMeta::new(7, 3, 9)),
            Some(RoundId {
                server_epoch: 7,
                match_id: 3,
            })
        );
    }

    #[test]
    fn tick_does_not_change_the_round_but_epoch_and_match_do() {
        let round = |epoch, id, tick| RoundId::from_meta(&SnapshotMeta::new(epoch, id, tick));
        assert_eq!(round(7, 3, 1), round(7, 3, 900));
        assert_ne!(round(7, 3, 1), round(8, 3, 1));
        assert_ne!(round(7, 3, 1), round(7, 4, 1));
    }
}
