//! Shared Verdant overlay palette and packaged, licensed typography.
use bevy::prelude::*;

pub(crate) const PANEL: Color = Color::srgba(0.025, 0.075, 0.078, 0.97);
pub(crate) const TILE: Color = Color::srgb(0.055, 0.14, 0.145);
pub(crate) const HOVER: Color = Color::srgb(0.10, 0.24, 0.23);
pub(crate) const EDGE: Color = Color::srgb(0.23, 0.40, 0.36);
pub(crate) const GOLD: Color = Color::srgb(0.94, 0.77, 0.43);
pub(crate) const IVORY: Color = Color::srgb(0.91, 0.94, 0.86);
pub(crate) const MUTED: Color = Color::srgb(0.55, 0.70, 0.66);
pub(crate) const JADE: Color = Color::srgb(0.24, 0.79, 0.58);

#[derive(Resource)]
pub(crate) struct UiTheme {
    pub font: Handle<Font>,
}

pub(crate) struct UiThemePlugin;
impl Plugin for UiThemePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_theme)
            .add_systems(Update, apply_theme_font);
    }
}

fn load_theme(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiTheme {
        font: assets.load("ui/Inter.ttf"),
    });
}

fn apply_theme_font(theme: Res<UiTheme>, mut text: Query<&mut TextFont, Added<TextFont>>) {
    for mut font in &mut text {
        font.font = theme.font.clone();
    }
}

pub(crate) fn text(size: f32) -> TextFont {
    TextFont {
        font_size: size,
        ..default()
    }
}

pub(crate) fn panel_node() -> Node {
    Node {
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    }
}
