//! The Verdant UI textures: icons, 9-slice frames and slabs, sprites and
//! menu backgrounds installed under `client/assets/ui/verdant/` by
//! `scripts/sync_ui_assets.py`. The typed table below is generated from the
//! installed manifest by `client/build.rs` (`build/ui_assets.rs`).
//!
//! Widgets put a [`KitImage`] on a node; [`resolve_kit_images`] turns it into
//! the node's `ImageNode` at the right density: `@2x` when a UI pixel covers
//! 1.5 or more physical pixels ([`UiDensity`]), with 9-slice insets scaled so
//! a frame's corners keep their logical size at any `UiScale`.
#![allow(dead_code)] // Every installed asset is typed; screens adopt them step by step.

use std::collections::HashMap;

use bevy::{
    prelude::*,
    sprite::{BorderRect, SliceScaleMode, TextureSlicer},
};

/// A sprite atlas grid (`TextureAtlasLayout::from_grid`), in texture px.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AtlasGrid {
    pub frame: u32,
    pub columns: u32,
    pub rows: u32,
    pub frames: u32,
    pub padding: u32,
}

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// Physical pixels per logical UI pixel (`Window::scale_factor × UiScale`).
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub(crate) struct UiDensity {
    pub scale: f32,
}

impl Default for UiDensity {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

impl UiDensity {
    /// `@2x` textures from 1.5 physical px per UI px (assets/README.md).
    pub(crate) fn hi(self) -> bool {
        self.scale >= 1.5
    }
}

pub(super) fn update_ui_density(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    mut density: ResMut<UiDensity>,
) {
    let window = windows.single().map_or(1.0, Window::scale_factor);
    let scale = window * ui_scale.map_or(1.0, |scale| scale.0);
    if (density.scale - scale).abs() > f32::EPSILON {
        density.scale = scale;
    }
}

/// Which installed texture a [`KitImage`] shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum KitSource {
    Icon(Icon),
    Frame(Frame),
    Sprite(Sprite),
    Background(Background),
}

impl KitSource {
    fn path(self, hi: bool) -> &'static str {
        match self {
            KitSource::Icon(icon) => icon.path(hi),
            KitSource::Frame(frame) => frame.path(hi),
            KitSource::Sprite(sprite) => sprite.path(hi),
            KitSource::Background(background) => background.path(hi),
        }
    }

    fn insets(self) -> Option<[f32; 4]> {
        match self {
            KitSource::Frame(frame) => frame.insets(),
            KitSource::Sprite(sprite) => sprite.insets(),
            _ => None,
        }
    }
}

/// A kit texture on this node, tinted with `tint` (white-on-alpha icons and
/// sprites take the token colour; frames use `Color::WHITE`). `frame` picks
/// an atlas cell for atlas sprites.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) struct KitImage {
    pub source: KitSource,
    pub tint: Color,
    pub frame: Option<usize>,
}

impl KitImage {
    pub(crate) fn icon(icon: Icon, tint: Color) -> Self {
        Self {
            source: KitSource::Icon(icon),
            tint,
            frame: None,
        }
    }

    pub(crate) fn frame(frame: Frame) -> Self {
        Self {
            source: KitSource::Frame(frame),
            tint: Color::WHITE,
            frame: None,
        }
    }

    pub(crate) fn sprite(sprite: Sprite, tint: Color) -> Self {
        Self {
            source: KitSource::Sprite(sprite),
            tint,
            frame: None,
        }
    }

    pub(crate) fn atlas(sprite: Sprite, tint: Color, frame: usize) -> Self {
        Self {
            source: KitSource::Sprite(sprite),
            tint,
            frame: Some(frame),
        }
    }

    pub(crate) fn background(background: Background) -> Self {
        Self {
            source: KitSource::Background(background),
            tint: Color::WHITE,
            frame: None,
        }
    }
}

/// Texture density override: the timer-ring atlas at 2x is ~25 MB of GPU
/// memory, so phones use 1x (assets/README.md).
#[derive(Component, Clone, Copy)]
pub(crate) struct LowDensity;

/// Atlas layouts per sprite and density, created once.
#[derive(Resource, Default)]
pub(crate) struct KitAtlasLayouts(HashMap<(Sprite, bool), Handle<TextureAtlasLayout>>);

/// The 9-slice scaler for insets given in 1x px, drawn at `density` physical
/// px per UI px from a texture with `texel` px per 1x px: corners keep their
/// logical size (Bevy draws corners at texture size × `max_corner_scale`).
pub(crate) fn slicer(insets: [f32; 4], texel: f32, density: f32) -> TextureSlicer {
    let [left, top, right, bottom] = insets;
    TextureSlicer {
        border: BorderRect {
            min_inset: Vec2::new(left, top) * texel,
            max_inset: Vec2::new(right, bottom) * texel,
        },
        center_scale_mode: SliceScaleMode::Stretch,
        sides_scale_mode: SliceScaleMode::Stretch,
        max_corner_scale: density / texel,
    }
}

/// Keeps every [`KitImage`]'s `ImageNode` in step with its source, tint,
/// atlas cell and the display density. Runs in `UiSet::Paint`.
#[allow(clippy::type_complexity)]
pub(crate) fn resolve_kit_images(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    density: Res<UiDensity>,
    mut layouts: ResMut<KitAtlasLayouts>,
    mut layout_assets: Option<ResMut<Assets<TextureAtlasLayout>>>,
    mut images: Query<(
        Entity,
        Ref<KitImage>,
        Option<&mut ImageNode>,
        Has<LowDensity>,
    )>,
) {
    let Some(assets) = assets else { return };
    let all = density.is_changed();
    for (entity, kit, image, low) in &mut images {
        if !all && !kit.is_changed() && image.is_some() {
            continue;
        }
        let hi = density.hi() && !low;
        let texel = if hi { 2.0 } else { 1.0 };
        let handle: Handle<Image> = assets.load(kit.source.path(hi));
        let mode = match kit.source.insets() {
            Some(insets) if kit.frame.is_none() => {
                NodeImageMode::Sliced(slicer(insets, texel, density.scale))
            }
            _ => NodeImageMode::Stretch,
        };
        let atlas = match (kit.source, kit.frame) {
            (KitSource::Sprite(sprite), Some(index)) => sprite.atlas(hi).map(|grid| {
                let layout = layouts.0.entry((sprite, hi)).or_insert_with(|| {
                    layout_assets
                        .as_mut()
                        .map_or_else(Handle::default, |store| {
                            store.add(TextureAtlasLayout::from_grid(
                                UVec2::splat(grid.frame),
                                grid.columns,
                                grid.rows,
                                Some(UVec2::splat(grid.padding)),
                                None,
                            ))
                        })
                });
                TextureAtlas {
                    layout: layout.clone(),
                    index: index.min(grid.frames as usize - 1),
                }
            }),
            _ => None,
        };
        match image {
            Some(mut node) => {
                if node.image != handle {
                    node.image = handle;
                }
                if node.color != kit.tint {
                    node.color = kit.tint;
                }
                if node.image_mode != mode {
                    node.image_mode = mode;
                }
                if node.texture_atlas != atlas {
                    node.texture_atlas = atlas;
                }
            }
            None => {
                let mut node = ImageNode::new(handle).with_mode(mode);
                node.color = kit.tint;
                node.texture_atlas = atlas;
                commands.entity(entity).insert(node);
            }
        }
    }
}

/// Crops an image to cover its node like CSS `object-fit: cover` (menu
/// backgrounds, hero tile art): sets `ImageNode::rect` once the texture and
/// the node size are known, and again when either changes.
#[derive(Component, Clone, Copy, Default)]
pub(crate) struct CoverImage {
    /// Vertical anchor of the crop, 0 = top, 0.5 = centre.
    pub anchor_y: f32,
}

pub(crate) fn fit_cover_images(
    textures: Res<Assets<Image>>,
    mut images: Query<(&CoverImage, &ComputedNode, &mut ImageNode)>,
) {
    for (cover, node, mut image) in &mut images {
        let Some(texture) = textures.get(&image.image) else {
            continue;
        };
        let rect = cover_rect(texture.size().as_vec2(), node.size(), cover.anchor_y);
        if rect.is_some() && image.rect != rect {
            image.rect = rect;
        }
    }
}

/// The part of a `texture`-sized image that covers a `node`-sized box.
pub(crate) fn cover_rect(texture: Vec2, node: Vec2, anchor_y: f32) -> Option<Rect> {
    if texture.min_element() <= 0.0 || node.min_element() <= 0.0 {
        return None;
    }
    let scale = (node.x / texture.x).max(node.y / texture.y);
    let size = node / scale;
    let min = Vec2::new(
        (texture.x - size.x) * 0.5,
        (texture.y - size.y) * anchor_y.clamp(0.0, 1.0),
    );
    Some(Rect::from_corners(min, min + size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_typed_asset_is_installed_at_both_densities() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut paths = Vec::new();
        for hi in [false, true] {
            paths.extend(Icon::ALL.iter().map(|icon| icon.path(hi)));
            paths.extend(Frame::ALL.iter().map(|frame| frame.path(hi)));
            paths.extend(Sprite::ALL.iter().map(|sprite| sprite.path(hi)));
            paths.extend(Background::ALL.iter().map(|background| background.path(hi)));
        }
        for path in paths {
            assert!(root.join(path).is_file(), "{path}");
        }
        assert_eq!(
            Frame::ButtonPrimary.insets(),
            Some([16.0, 14.0, 16.0, 14.0])
        );
        assert_eq!(Frame::Ornament.insets(), Some([56.0; 4]));
        assert_eq!(Frame::Panel.size(), (96.0, 96.0));
        assert_eq!(Icon::NavLock.path(true), "ui/verdant/icons/nav/lock@2x.png");
        assert_eq!(
            Sprite::CooldownSweepAtlas.atlas(false),
            Some(AtlasGrid {
                frame: 64,
                columns: 10,
                rows: 6,
                frames: 60,
                padding: 2
            })
        );
        assert_eq!(
            Background::MenuArena.path(true),
            "ui/verdant/backgrounds/menu-arena-1080.jpg"
        );
    }

    /// Corners keep their logical size: 16 px at 1x density with the 1x
    /// texture, and with the 2x texture at 1.5 and 2.0.
    #[test]
    fn slicer_keeps_corners_at_their_logical_size() {
        let low = slicer([16.0, 14.0, 16.0, 14.0], 1.0, 1.0);
        assert_eq!(low.border.min_inset, Vec2::new(16.0, 14.0));
        assert_eq!(low.max_corner_scale, 1.0);
        let high = slicer([16.0, 14.0, 16.0, 14.0], 2.0, 1.5);
        assert_eq!(high.border.max_inset, Vec2::new(32.0, 28.0));
        // 32 texture px × 0.75 = 24 physical px = 16 logical px at 1.5.
        assert_eq!(high.max_corner_scale, 0.75);
        assert!(UiDensity { scale: 1.5 }.hi());
        assert!(!UiDensity { scale: 1.25 }.hi());
    }

    #[test]
    fn cover_crops_the_long_axis_around_the_anchor() {
        // A 1920×1080 image in a 4:3 box keeps full height, crops the sides.
        let rect = cover_rect(Vec2::new(1920.0, 1080.0), Vec2::new(800.0, 600.0), 0.5).unwrap();
        assert!((rect.height() - 1080.0).abs() < 1e-2);
        assert!((rect.width() - 1440.0).abs() < 1e-2);
        assert!((rect.min.x - 240.0).abs() < 1e-2);
        // A square portrait in a wide tile keeps the top (anchor 0).
        let top = cover_rect(Vec2::new(256.0, 256.0), Vec2::new(96.0, 88.0), 0.0).unwrap();
        assert_eq!(top.min.y, 0.0);
        assert!((top.width() - 256.0).abs() < 1e-3);
        assert!(cover_rect(Vec2::ZERO, Vec2::ONE, 0.5).is_none());
    }
}
