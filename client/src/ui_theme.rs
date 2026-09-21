//! Shared Verdant overlay palette and packaged, licensed typography.
use bevy::prelude::*;

pub(crate) const PANEL: Color = Color::srgba(0.025, 0.060, 0.065, 0.96);
pub(crate) const TILE: Color = Color::srgb(0.050, 0.115, 0.125);
pub(crate) const HOVER: Color = Color::srgb(0.085, 0.205, 0.200);
pub(crate) const EDGE: Color = Color::srgb(0.19, 0.32, 0.30);
pub(crate) const GOLD: Color = Color::srgb(0.86, 0.74, 0.49);
pub(crate) const IVORY: Color = Color::srgb(0.92, 0.94, 0.89);
pub(crate) const MUTED: Color = Color::srgb(0.61, 0.72, 0.70);
pub(crate) const JADE: Color = Color::srgb(0.24, 0.79, 0.58);

#[derive(Resource)]
pub(crate) struct UiTheme {
    pub font: Handle<Font>,
    pub cjk_font: Handle<Font>,
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
        cjk_font: assets.load("ui/NotoSansCJKsc-Regular.otf"),
    });
}

fn needs_cjk_font(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character as u32,
        0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f |
        0xac00..=0xd7ff | 0xf900..=0xfaff | 0xfe30..=0xfe4f |
        0xff00..=0xffef | 0x20000..=0x323af)
    })
}

fn apply_theme_font(
    theme: Res<UiTheme>,
    mut text: Query<
        (&mut TextFont, Option<&Text>, Option<&TextSpan>),
        Or<(Added<TextFont>, Changed<Text>, Changed<TextSpan>)>,
    >,
) {
    for (mut font, root, span) in &mut text {
        let cjk = root.is_some_and(|text| needs_cjk_font(&text.0))
            || span.is_some_and(|text| needs_cjk_font(&text.0));
        font.font = if cjk {
            theme.cjk_font.clone()
        } else {
            theme.font.clone()
        };
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
        border_radius: BorderRadius::all(Val::Px(8.0)),
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packaged_font(name: &str) -> Font {
        Font::try_from_bytes(
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/ui")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn latin_cyrillic_keep_inter_and_cjk_names_use_the_packaged_fallback() {
        for value in ["Victory", "QA Дмитрий", "Δημήτρης"] {
            assert!(!needs_cjk_font(value));
        }
        for value in ["QA 小明", "かなカナ", "한글", "中文漢字"] {
            assert!(needs_cjk_font(value));
        }
        let mut fonts = Assets::<Font>::default();
        let font = fonts.add(packaged_font("Inter.ttf"));
        let cjk_font = fonts.add(packaged_font("NotoSansCJKsc-Regular.otf"));
        let mut app = App::new();
        app.insert_resource(UiTheme {
            font: font.clone(),
            cjk_font: cjk_font.clone(),
        })
        .add_systems(Update, apply_theme_font);
        let entity = app
            .world_mut()
            .spawn((Text::new("QA Дмитрий"), TextFont::default()))
            .id();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, font);
        app.world_mut().get_mut::<Text>(entity).unwrap().0 = "QA 小明".into();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, cjk_font);
        app.world_mut().get_mut::<Text>(entity).unwrap().0 = "QA Дмитрий".into();
        app.update();
        assert_eq!(app.world().get::<TextFont>(entity).unwrap().font, font);
        let span = app
            .world_mut()
            .spawn((TextSpan::new("한글"), TextFont::default()))
            .id();
        app.update();
        assert_eq!(app.world().get::<TextFont>(span).unwrap().font, cjk_font);
    }
}
