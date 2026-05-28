use std::{collections::HashMap, fs, path::Path};

use ::bevy::{gltf::Gltf, prelude::*};

use crate::{BUILTIN_MODEL_MANIFEST, EkzaCharacter, ModelEntry, ModelSource, is_valid_glb_bytes};

#[derive(Clone)]
pub struct EkzaModelHandles {
    pub scene: Option<Handle<Scene>>,
    pub gltf: Option<Handle<Gltf>>,
    pub label: String,
}

#[derive(Resource, Clone, Default)]
pub struct EkzaModelCatalog {
    entries: HashMap<EkzaCharacter, EkzaModelHandles>,
}

impl EkzaModelCatalog {
    pub fn insert(&mut self, character: EkzaCharacter, handles: EkzaModelHandles) {
        self.entries.insert(character, handles);
    }

    pub fn handles_for(
        &self,
        character: EkzaCharacter,
    ) -> (Option<Handle<Scene>>, Option<Handle<Gltf>>) {
        self.entries
            .get(&character)
            .map(|entry| (entry.scene.clone(), entry.gltf.clone()))
            .unwrap_or((None, None))
    }

    pub fn label_for(&self, character: EkzaCharacter) -> String {
        self.entries
            .get(&character)
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| character.as_str().to_string())
    }
}

pub fn load_builtin_model_catalog(
    asset_server: &AssetServer,
    asset_root: &Path,
) -> EkzaModelCatalog {
    let mut catalog = EkzaModelCatalog::default();
    for entry in BUILTIN_MODEL_MANIFEST {
        let handles = load_model_entry(entry, asset_server, asset_root);
        catalog.insert(entry.character, handles);
    }
    catalog
}

pub fn load_model_entry(
    entry: ModelEntry,
    asset_server: &AssetServer,
    asset_root: &Path,
) -> EkzaModelHandles {
    match entry.source {
        ModelSource::LocalGlb { path, scene_label } => {
            if asset_root.join(path).exists() {
                glb_handles(asset_server, path, scene_label, path.to_string())
            } else {
                EkzaModelHandles {
                    scene: None,
                    gltf: None,
                    label: entry.display_name.to_string(),
                }
            }
        }
        ModelSource::RemoteGlb {
            url,
            cache_path,
            scene_label,
        } => {
            if cache_remote_glb(url, asset_root, cache_path).is_some() {
                glb_handles(
                    asset_server,
                    cache_path,
                    scene_label,
                    cache_path.to_string(),
                )
            } else {
                EkzaModelHandles {
                    scene: None,
                    gltf: None,
                    label: entry.display_name.to_string(),
                }
            }
        }
        ModelSource::PrimitiveFallback => EkzaModelHandles {
            scene: None,
            gltf: None,
            label: entry.display_name.to_string(),
        },
    }
}

pub fn cache_remote_glb(url: &str, asset_root: &Path, cache_path: &str) -> Option<String> {
    use reqwest::blocking as req_blocking;

    let final_path = asset_root.join(cache_path);
    if let Ok(existing) = fs::read(&final_path)
        && is_valid_glb_bytes(&existing)
    {
        return Some(cache_path.to_string());
    }

    if let Some(parent) = final_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        warn!("Failed to create SDK model cache directory {parent:?}: {error}");
        return None;
    }

    let response = match req_blocking::get(url) {
        Ok(response) => response,
        Err(error) => {
            warn!("Failed to download Ekza model {url}: {error}");
            return None;
        }
    };
    let bytes = response
        .bytes()
        .map_err(|error| warn!("Failed to read bytes from Ekza model {url}: {error}"))
        .ok()?;
    if !is_valid_glb_bytes(&bytes) {
        warn!("Downloaded Ekza model from {url} is not a valid GLB.");
        return None;
    }
    if let Err(error) = fs::write(&final_path, &bytes) {
        warn!("Failed to write Ekza model cache file {final_path:?}: {error}");
        return None;
    }
    Some(cache_path.to_string())
}

fn glb_handles(
    asset_server: &AssetServer,
    path: &str,
    scene_label: &str,
    label: String,
) -> EkzaModelHandles {
    EkzaModelHandles {
        scene: Some(asset_server.load(format!("{path}#{scene_label}"))),
        gltf: Some(asset_server.load(path.to_string())),
        label,
    }
}
