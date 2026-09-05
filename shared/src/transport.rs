//! Opt-in UDP snapshot framing. Datagrams fit a conservative 1200-byte
//! application budget; incomplete snapshots expire and never reach gameplay.

pub use crate::protocol::PROTOCOL_VERSION;
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    time::{Duration, Instant},
};

pub const MAX_DATAGRAM_BYTES: usize = 1200;
pub const MAX_SNAPSHOT_BYTES: usize = 65_507;
pub const MAX_ASSEMBLIES: usize = 4;
pub const ASSEMBLY_TTL: Duration = Duration::from_secs(2);
const MAGIC: &[u8; 4] = b"OMB1";
const HEADER: usize = 30;
const CHUNK: usize = MAX_DATAGRAM_BYTES - HEADER;
const MAX_FRAGMENTS: usize = MAX_SNAPSHOT_BYTES.div_ceil(CHUNK);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Size,
    Header,
    Version,
    Conflict,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Size => "snapshot or fragment exceeds the transport budget",
            Self::Header => "invalid snapshot fragment header",
            Self::Version => "unsupported snapshot framing version",
            Self::Conflict => "conflicting snapshot fragments",
        })
    }
}
impl std::error::Error for TransportError {}

pub fn encode_snapshot(
    payload: &[u8],
    server_epoch: u64,
    snapshot_tick: u64,
) -> Result<Vec<Vec<u8>>, TransportError> {
    if payload.is_empty() || payload.len() > MAX_SNAPSHOT_BYTES {
        return Err(TransportError::Size);
    }
    if server_epoch == 0 {
        return Err(TransportError::Header);
    }
    let count = payload.len().div_ceil(CHUNK);
    Ok(payload
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, data)| {
            let mut packet = Vec::with_capacity(HEADER + data.len());
            packet.extend_from_slice(MAGIC);
            packet.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
            packet.extend_from_slice(&server_epoch.to_le_bytes());
            packet.extend_from_slice(&snapshot_tick.to_le_bytes());
            packet.extend_from_slice(&(index as u16).to_le_bytes());
            packet.extend_from_slice(&(count as u16).to_le_bytes());
            packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            packet.extend_from_slice(data);
            packet
        })
        .collect())
}

struct Assembly {
    started: Instant,
    total: usize,
    received: usize,
    parts: Vec<Option<Vec<u8>>>,
}

#[derive(Default)]
pub struct SnapshotAssembler {
    pending: BTreeMap<(u64, u64), Assembly>,
    completed: VecDeque<(u64, u64)>,
}

impl SnapshotAssembler {
    pub fn expire(&mut self, now: Instant) {
        self.pending
            .retain(|_, value| now.saturating_duration_since(value.started) < ASSEMBLY_TTL);
    }

    pub fn retained_bytes(&self) -> usize {
        self.pending
            .values()
            .map(|a| a.parts.iter().flatten().map(Vec::len).sum::<usize>())
            .sum()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Legacy JSON is forwarded whole for compatibility; its schema and version
    /// are validated by the recipient. Framed data is emitted only when complete.
    pub fn push(&mut self, packet: &[u8], now: Instant) -> Result<Option<Vec<u8>>, TransportError> {
        self.expire(now);
        if !packet.starts_with(MAGIC) {
            return if packet.is_empty() || packet.len() > MAX_SNAPSHOT_BYTES {
                Err(TransportError::Size)
            } else {
                Ok(Some(packet.to_vec()))
            };
        }
        if packet.len() < HEADER || packet.len() > MAX_DATAGRAM_BYTES {
            return Err(TransportError::Size);
        }
        let version = u16::from_le_bytes(packet[4..6].try_into().unwrap());
        if version != PROTOCOL_VERSION {
            return Err(TransportError::Version);
        }
        let epoch = u64::from_le_bytes(packet[6..14].try_into().unwrap());
        let tick = u64::from_le_bytes(packet[14..22].try_into().unwrap());
        let index = u16::from_le_bytes(packet[22..24].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(packet[24..26].try_into().unwrap()) as usize;
        let total = u32::from_le_bytes(packet[26..30].try_into().unwrap()) as usize;
        if epoch == 0
            || total == 0
            || total > MAX_SNAPSHOT_BYTES
            || count == 0
            || count > MAX_FRAGMENTS
            || count != total.div_ceil(CHUNK)
            || index >= count
        {
            return Err(TransportError::Header);
        }
        let expected = if index + 1 == count {
            total - index * CHUNK
        } else {
            CHUNK
        };
        if packet.len() - HEADER != expected {
            return Err(TransportError::Header);
        }
        let key = (epoch, tick);
        if self.completed.contains(&key) {
            return Ok(None);
        }
        if !self.pending.contains_key(&key) && self.pending.len() >= MAX_ASSEMBLIES {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(_, a)| a.started)
                .map(|(key, _)| *key)
                .unwrap();
            self.pending.remove(&oldest);
        }
        let assembly = self.pending.entry(key).or_insert_with(|| Assembly {
            started: now,
            total,
            received: 0,
            parts: vec![None; count],
        });
        if assembly.total != total || assembly.parts.len() != count {
            self.pending.remove(&key);
            return Err(TransportError::Conflict);
        }
        let data = &packet[HEADER..];
        if let Some(existing) = &assembly.parts[index] {
            if existing != data {
                self.pending.remove(&key);
                return Err(TransportError::Conflict);
            }
        } else {
            assembly.parts[index] = Some(data.to_vec());
            assembly.received += 1;
        }
        if assembly.received != count {
            return Ok(None);
        }
        let assembly = self.pending.remove(&key).unwrap();
        let mut payload = Vec::with_capacity(total);
        for part in assembly.parts {
            payload.extend_from_slice(&part.unwrap());
        }
        self.completed.push_back(key);
        if self.completed.len() > 16 {
            self.completed.pop_front();
        }
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_snapshots_survive_reordering_and_duplicates_without_partial_output() {
        for size in [1, 1170, 1171, 8193, 9217, MAX_SNAPSHOT_BYTES] {
            let payload = (0..size).map(|i| (i % 251) as u8).collect::<Vec<_>>();
            let frames = encode_snapshot(&payload, 9, 42).unwrap();
            assert!(frames.iter().all(|frame| frame.len() <= MAX_DATAGRAM_BYTES));
            let mut assembler = SnapshotAssembler::default();
            let now = Instant::now();
            let mut outputs = Vec::new();
            for frame in frames.iter().rev() {
                if let Some(bytes) = assembler.push(frame, now).unwrap() {
                    outputs.push(bytes);
                }
                assert!(assembler.push(frame, now).unwrap().is_none());
            }
            assert_eq!(outputs, vec![payload]);
            assert_eq!(assembler.retained_bytes(), 0);
        }
    }

    #[test]
    fn dropped_fragments_expire_and_unfinished_work_is_bounded() {
        let now = Instant::now();
        let mut assembler = SnapshotAssembler::default();
        for tick in 0..100 {
            let frames = encode_snapshot(&vec![b'x'; MAX_SNAPSHOT_BYTES], 1, tick).unwrap();
            assert!(assembler.push(&frames[0], now).unwrap().is_none());
            assert!(assembler.pending_count() <= MAX_ASSEMBLIES);
            assert!(assembler.retained_bytes() <= MAX_ASSEMBLIES * MAX_SNAPSHOT_BYTES);
        }
        assembler.expire(now + ASSEMBLY_TTL);
        assert_eq!(assembler.pending_count(), 0);
        assert_eq!(assembler.retained_bytes(), 0);
    }

    #[test]
    fn malformed_conflicting_and_oversized_frames_are_rejected() {
        assert!(encode_snapshot(&vec![0; MAX_SNAPSHOT_BYTES + 1], 1, 1).is_err());
        let now = Instant::now();
        let mut assembler = SnapshotAssembler::default();
        let frames = encode_snapshot(&vec![1; 2400], 1, 1).unwrap();
        assert!(assembler.push(&frames[0], now).unwrap().is_none());
        let mut conflicting = frames[0].clone();
        conflicting[HEADER] = 2;
        assert_eq!(
            assembler.push(&conflicting, now),
            Err(TransportError::Conflict)
        );
        let mut malformed = frames[0].clone();
        malformed[24..26].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(assembler.push(&malformed, now).is_err());
        malformed = frames[0].clone();
        malformed[4] = 99;
        assert_eq!(
            assembler.push(&malformed, now),
            Err(TransportError::Version)
        );
        assert!(assembler.push(b"OMB1", now).is_err());
    }
}
