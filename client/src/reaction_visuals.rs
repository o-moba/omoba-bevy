//! Local reaction images, independent from the server's pack entitlement policy.
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;
use std::collections::HashMap;

pub(crate) const FREE_IDS: [&str; 4] = ["thumbs_up", "thumbs_down", "heart", "laugh"];
const BUILT_IN: &str = r#"{"schema_version":1,"wheel":["thumbs_up","thumbs_down","heart","laugh"],"images":{"thumbs_up":{"path":"reactions/base-atlas.png","grid":[2,2],"index":0},"thumbs_down":{"path":"reactions/base-atlas.png","grid":[2,2],"index":1},"heart":{"path":"reactions/base-atlas.png","grid":[2,2],"index":2},"laugh":{"path":"reactions/base-atlas.png","grid":[2,2],"index":3}}}"#;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageSpec {
    path: String,
    grid: [u32; 2],
    index: u32,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    wheel: [String; 4],
    images: HashMap<String, ImageSpec>,
}
impl Manifest {
    fn parse(json: &str) -> Result<Self, &'static str> {
        if json.len() > 64 * 1024 {
            return Err("Reaction image manifest is too large.");
        }
        let config: Self =
            serde_json::from_str(json).map_err(|_| "Invalid reaction image manifest.")?;
        if config.schema_version != 1 || config.images.len() > 256 || config.images.is_empty() {
            return Err("Unsupported reaction image manifest.");
        }
        for (id, image) in &config.images {
            if !shared::social::valid_reaction_id(id)
                || !crate::combat_visuals::safe_asset_path(&image.path, ".png")
                || image.grid.iter().any(|side| *side == 0 || *side > 16)
                || image.index >= image.grid[0] * image.grid[1]
            {
                return Err("Invalid reaction image entry.");
            }
        }
        if config
            .wheel
            .iter()
            .any(|id| !config.images.contains_key(id))
        {
            return Err("Missing wheel image.");
        }
        let unique: std::collections::HashSet<_> = config.wheel.iter().collect();
        if unique.len() != 4 {
            return Err("Wheel entries must be distinct.");
        }
        Ok(config)
    }
}
#[derive(Resource)]
pub(crate) struct ReactionVisuals {
    pub(crate) wheel: [String; 4],
    fallback: Handle<Image>,
    images: HashMap<String, (ImageSpec, Handle<Image>)>,
}
#[derive(Asset, TypePath)]
struct LoadedManifest(Manifest);
#[derive(Default, TypePath)]
struct ManifestLoader;
impl AssetLoader for ManifestLoader {
    type Asset = LoadedManifest;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let json = std::str::from_utf8(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Manifest::parse(json)
            .map(LoadedManifest)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
#[derive(Resource, Default)]
struct Pending(Option<Handle<LoadedManifest>>);
pub(crate) struct ReactionVisualsPlugin;
impl Plugin for ReactionVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<LoadedManifest>()
            .init_asset_loader::<ManifestLoader>()
            .init_resource::<Pending>()
            .add_systems(Startup, load)
            .add_systems(Update, apply);
    }
}
fn visuals(config: Manifest, assets: &AssetServer) -> ReactionVisuals {
    let images = config
        .images
        .into_iter()
        .map(|(id, spec)| {
            let handle = assets.load(spec.path.clone());
            (id, (spec, handle))
        })
        .collect();
    ReactionVisuals {
        fallback: assets.load("reactions/base-atlas.png"),
        wheel: config.wheel,
        images,
    }
}
fn load(mut commands: Commands, assets: Res<AssetServer>, mut pending: ResMut<Pending>) {
    commands.insert_resource(visuals(
        Manifest::parse(BUILT_IN).expect("packaged fallback valid"),
        &assets,
    ));
    pending.0 = Some(assets.load("reactions/manifest.json"));
}
fn apply(
    assets: Res<AssetServer>,
    loaded: Res<Assets<LoadedManifest>>,
    mut pending: ResMut<Pending>,
    mut registry: ResMut<ReactionVisuals>,
) {
    let Some(handle) = pending.0.as_ref() else {
        return;
    };
    if let Some(manifest) = loaded.get(handle) {
        *registry = visuals(manifest.0.clone(), &assets);
        pending.0 = None;
    } else if matches!(
        assets.get_load_state(handle.id()),
        Some(bevy::asset::LoadState::Failed(_))
    ) {
        warn!("Reaction image configuration unavailable; keeping built-in images");
        pending.0 = None;
    }
}
impl ReactionVisuals {
    pub(crate) fn image(&self, id: &str, images: &Assets<Image>) -> Option<ImageNode> {
        let custom = self
            .images
            .get(id)
            .and_then(|(spec, handle)| crop(spec, handle, images));
        custom.or_else(|| {
            let index = FREE_IDS.iter().position(|candidate| *candidate == id)? as u32;
            crop(
                &ImageSpec {
                    path: "reactions/base-atlas.png".into(),
                    grid: [2, 2],
                    index,
                },
                &self.fallback,
                images,
            )
        })
    }
}
fn crop(spec: &ImageSpec, handle: &Handle<Image>, images: &Assets<Image>) -> Option<ImageNode> {
    let image = images.get(handle)?;
    if image.width() == 0
        || image.height() == 0
        || image.width() > 8192
        || image.height() > 8192
        || image.width() % spec.grid[0] != 0
        || image.height() % spec.grid[1] != 0
    {
        return None;
    }
    let cell = Vec2::new(
        image.width() as f32 / spec.grid[0] as f32,
        image.height() as f32 / spec.grid[1] as f32,
    );
    let min = Vec2::new(
        (spec.index % spec.grid[0]) as f32,
        (spec.index / spec.grid[0]) as f32,
    ) * cell;
    Some(ImageNode {
        rect: Some(Rect::from_corners(min, min + cell)),
        ..ImageNode::new(handle.clone())
    })
}

pub(crate) fn label(id: &str) -> String {
    shared::social::reaction(id)
        .map_or_else(|| "Reaction".into(), |reaction| reaction.label.clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_manifest_is_bounded_safe_and_independent_of_access() {
        let config = Manifest::parse(BUILT_IN).unwrap();
        assert_eq!(config.wheel, FREE_IDS.map(str::to_owned));
        for path in [
            "https://example.test/a.png",
            "../a.png",
            "/tmp/a.png",
            "a.glb",
        ] {
            assert!(Manifest::parse(&BUILT_IN.replace("reactions/base-atlas.png", path)).is_err());
        }
        assert!(Manifest::parse(&BUILT_IN.replace("\"grid\":[2,2]", "\"grid\":[0,2]")).is_err());
        assert!(Manifest::parse(&BUILT_IN.replace("\"index\":3", "\"index\":4")).is_err());
        // Rendering a local image is not an ownership assertion.
        assert!(Manifest::parse(&BUILT_IN.replace("thumbs_up", "paid_sticker")).is_ok());
    }
}
