//! Real glTF/PBR asset loading without a GPU, window, audio device or game systems.
use bevy::prelude::*;

struct HeadlessShaderAssets;
impl Plugin for HeadlessShaderAssets {
    fn build(&self, app: &mut App) {
        app.init_asset::<bevy::shader::Shader>()
            .register_asset_loader(bevy::shader::ShaderLoader);
    }
}

pub(crate) fn asset_app() -> App {
    let mut app = App::new();
    // In 0.19 PbrPlugin installs the glTF material extension. Keep the real
    // default asset plugins, including that extension, without creating a GPU.
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::RenderPlugin>()
            .disable::<bevy::audio::AudioPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .add_after::<AssetPlugin>(HeadlessShaderAssets)
            .set(AssetPlugin {
                file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                ..default()
            }),
    );
    app
}
