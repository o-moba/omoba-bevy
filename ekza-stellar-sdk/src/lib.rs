//! Ekza-Stellar SDK surface for character identity and 3D model integration.
//!
//! The crate is intentionally small in this first extraction slice: it owns stable
//! character ids, model manifest metadata, GLB validation, and an optional Bevy
//! integration layer for resolving model assets into handles.

use serde::{Deserialize, Serialize};

/// Stable character ids shared by the game, protocol, and future SDK consumers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EkzaCharacter {
    #[default]
    Ipfs,
    Toka,
    Wang,
    Cube,
}

impl EkzaCharacter {
    pub const ALL: [Self; 4] = [Self::Ipfs, Self::Toka, Self::Wang, Self::Cube];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipfs => "IPFS",
            Self::Toka => "Toka",
            Self::Wang => "Wang",
            Self::Cube => "Cube",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Ipfs => "ipfs",
            Self::Toka => "toka",
            Self::Wang => "wang",
            Self::Cube => "cube",
        }
    }
}

/// Static source metadata for one SDK model entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// A GLB file already present under the consumer's Bevy asset root.
    LocalGlb {
        path: &'static str,
        scene_label: &'static str,
    },
    /// A remote GLB file to cache under the consumer's Bevy asset root.
    RemoteGlb {
        url: &'static str,
        cache_path: &'static str,
        scene_label: &'static str,
    },
    /// Consumer should render its own primitive fallback.
    PrimitiveFallback,
}

/// Built-in Ekza-Stellar character model metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    pub character: EkzaCharacter,
    pub display_name: &'static str,
    pub source: ModelSource,
    pub locomotion_animations: bool,
}

pub const IPFS_CHARACTER_URL: &str =
    "https://ipfs.io/ipfs/QmWMYVUF2pa4GkoMgquyY8nmYjQJDP9yxnSBvjVqH7EJQr";

pub const BUILTIN_MODEL_MANIFEST: [ModelEntry; 4] = [
    ModelEntry {
        character: EkzaCharacter::Ipfs,
        display_name: "IPFS",
        source: ModelSource::RemoteGlb {
            url: IPFS_CHARACTER_URL,
            cache_path: "downloaded/ipfs.glb",
            scene_label: "Scene0",
        },
        locomotion_animations: false,
    },
    ModelEntry {
        character: EkzaCharacter::Toka,
        display_name: "Toka",
        source: ModelSource::LocalGlb {
            path: "downloaded/toka.glb",
            scene_label: "Scene0",
        },
        locomotion_animations: true,
    },
    ModelEntry {
        character: EkzaCharacter::Wang,
        display_name: "Wang",
        source: ModelSource::LocalGlb {
            path: "downloaded/wang.glb",
            scene_label: "Scene0",
        },
        locomotion_animations: true,
    },
    ModelEntry {
        character: EkzaCharacter::Cube,
        display_name: "Cube",
        source: ModelSource::PrimitiveFallback,
        locomotion_animations: false,
    },
];

pub fn builtin_model_entry(character: EkzaCharacter) -> &'static ModelEntry {
    BUILTIN_MODEL_MANIFEST
        .iter()
        .find(|entry| entry.character == character)
        .expect("every EkzaCharacter must have a built-in model entry")
}

pub fn is_valid_glb_bytes(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[0..4] != b"glTF" {
        return false;
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let length = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    version == 2 && length <= bytes.len()
}

#[cfg(feature = "bevy")]
pub mod bevy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_ids_keep_protocol_snake_case() {
        assert_eq!(
            serde_json::to_string(&EkzaCharacter::Ipfs).unwrap(),
            "\"ipfs\""
        );
        assert_eq!(
            serde_json::to_string(&EkzaCharacter::Toka).unwrap(),
            "\"toka\""
        );
        assert_eq!(
            serde_json::to_string(&EkzaCharacter::Wang).unwrap(),
            "\"wang\""
        );
        assert_eq!(
            serde_json::to_string(&EkzaCharacter::Cube).unwrap(),
            "\"cube\""
        );
    }

    #[test]
    fn manifest_covers_every_character() {
        for character in EkzaCharacter::ALL {
            assert_eq!(builtin_model_entry(character).character, character);
        }
    }

    #[test]
    fn glb_header_validation_checks_magic_version_and_declared_length() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&12_u32.to_le_bytes());
        assert!(is_valid_glb_bytes(&bytes));

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(!is_valid_glb_bytes(&bad_magic));

        let mut bad_length = bytes;
        bad_length[8..12].copy_from_slice(&64_u32.to_le_bytes());
        assert!(!is_valid_glb_bytes(&bad_length));
    }
}
