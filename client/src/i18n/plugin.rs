//! The Bevy side of i18n: the [`Locale`] resource, the startup language and
//! the one relabel system for [`Localized`] text.
use bevy::prelude::*;

use super::{LocaleId, Localized, active, store_active};

/// Environment override of the startup language for QA and captures
/// (`OMOBA_LANGUAGE=zh-Hans`). It is not written to the preferences file.
pub(crate) const LANGUAGE_ENV: &str = "OMOBA_LANGUAGE";

/// The active language as Bevy state. Systems that show text re-run on its
/// change (`locale_changed`); render-key caches add [`Self::generation`].
///
/// The resource [`I18nPlugin`] inserts drives the process-wide locale that
/// `tr` reads: [`Self::set`] updates both at once, so every system later in
/// the same frame already reads the new language. Any other instance
/// (`Locale::default()`, [`Locale::detached`], used by tests) only changes
/// itself, so a test never switches the language of other tests running in
/// the same process (see `i18n::testing::isolated`).
#[derive(Resource, Debug)]
pub(crate) struct Locale {
    id: LocaleId,
    generation: u32,
    drives_process: bool,
}

impl Default for Locale {
    fn default() -> Self {
        Self::detached(LocaleId::ENGLISH)
    }
}

impl Locale {
    /// A locale resource that leaves the process-wide locale alone.
    pub(crate) fn detached(id: LocaleId) -> Self {
        Self {
            id,
            generation: 0,
            drives_process: false,
        }
    }

    fn process_wide(id: LocaleId) -> Self {
        store_active(id);
        Self {
            id,
            generation: 0,
            drives_process: true,
        }
    }

    pub(crate) fn id(&self) -> LocaleId {
        self.id
    }

    /// Bumped by every language change since startup; 0 means the startup
    /// language (from the environment, the preferences or the default).
    pub(crate) fn generation(&self) -> u32 {
        self.generation
    }

    /// Switches the language. Callers check `id() != next` first, so an
    /// unchanged language does not mark the resource changed.
    pub(crate) fn set(&mut self, id: LocaleId) {
        if self.drives_process {
            store_active(id);
        }
        self.id = id;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// `true` when the language changed since the system last ran (and on the
/// first run). Per-frame text writers `||` it into their own change checks;
/// it takes an `Option` so systems also run in apps without the plugin.
pub(crate) fn locale_changed(locale: &Option<Res<Locale>>) -> bool {
    locale.as_ref().is_some_and(|locale| locale.is_changed())
}

/// Where the startup language comes from.
#[derive(Clone, Copy, Debug, Default)]
enum Startup {
    /// English, without reading anything (the default, for tests).
    #[default]
    English,
    /// `OMOBA_LANGUAGE`, else the saved preference, else English.
    Environment,
    #[cfg(test)]
    Fixed(LocaleId),
}

/// Inserts [`Locale`], sets the process-wide language before any `Startup`
/// system spawns text, and relabels [`Localized`] text. The client adds
/// [`I18nPlugin::from_environment`] first, before every UI plugin.
#[derive(Default)]
pub(crate) struct I18nPlugin {
    startup: Startup,
}

/// PostUpdate order of text finishing: `Relabel` rewrites `Localized` text,
/// then `Font` (`ui::theme::apply_theme_font`) picks the Latin or CJK font,
/// both before UI and `Text2d` layout, so a changed text never renders a
/// frame in the old language or the wrong font.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum I18nSystems {
    Relabel,
    Font,
}

impl I18nPlugin {
    /// The shipped client: `OMOBA_LANGUAGE`, else `language` from
    /// `client_preferences.json`, else English.
    pub(crate) fn from_environment() -> Self {
        Self {
            startup: Startup::Environment,
        }
    }

    /// A fixed startup language (tests that run isolated).
    #[cfg(test)]
    pub(crate) fn fixed(id: LocaleId) -> Self {
        Self {
            startup: Startup::Fixed(id),
        }
    }
}

/// `OMOBA_LANGUAGE` if it names a shipped locale, else the saved one.
fn startup_locale(env: Option<&str>, saved: impl FnOnce() -> Option<LocaleId>) -> LocaleId {
    if let Some(code) = env.map(str::trim).filter(|code| !code.is_empty()) {
        if let Some(id) = LocaleId::parse(code) {
            return id;
        }
        warn!("{LANGUAGE_ENV}={code:?} is not a shipped language; ignoring it.");
    }
    saved().unwrap_or(LocaleId::ENGLISH)
}

impl Plugin for I18nPlugin {
    fn build(&self, app: &mut App) {
        let id = match self.startup {
            Startup::English => LocaleId::ENGLISH,
            #[cfg(test)]
            Startup::Fixed(id) => id,
            Startup::Environment => startup_locale(
                std::env::var(LANGUAGE_ENV).ok().as_deref(),
                crate::persistence::saved_language,
            ),
        };
        app.insert_resource(Locale::process_wide(id));
        super::ensure_loaded();
        info!("Language: {} ({})", id.native_name(), id.code());
        configure_text_sets(app);
        app.add_systems(PostUpdate, relabel_localized.in_set(I18nSystems::Relabel));
    }
}

/// Orders [`I18nSystems`] before UI and `Text2d` layout. Both the i18n
/// plugin and the UI kit (which owns the font system) call it.
pub(crate) fn configure_text_sets(app: &mut App) {
    app.configure_sets(
        PostUpdate,
        (I18nSystems::Relabel, I18nSystems::Font)
            .chain()
            .before(bevy::ui::UiSystems::Prepare)
            .before(bevy::text::Text2dUpdateSystems),
    );
}

/// Rewrites every [`Localized`] text when the language changes, and one
/// that was just added or changed. Reads the [`Locale`] resource (not the
/// process-wide value) so a detached test locale relabels too.
pub(crate) fn relabel_localized(
    locale: Option<Res<Locale>>,
    mut labels: Query<(
        Ref<Localized>,
        Option<&mut Text>,
        Option<&mut Text2d>,
        Option<&mut TextSpan>,
    )>,
) {
    let all = locale_changed(&locale);
    let id = locale.as_ref().map_or_else(active, |locale| locale.id());
    for (localized, text, text2d, span) in &mut labels {
        if !all && !localized.is_changed() {
            continue;
        }
        let next = localized.text_in(id);
        let current = text
            .map(|text| text.map_unchanged(|text| &mut text.0))
            .or_else(|| text2d.map(|text| text.map_unchanged(|text| &mut text.0)))
            .or_else(|| span.map(|span| span.map_unchanged(|span| &mut span.0)));
        if let Some(mut current) = current {
            if *current != next {
                *current = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zh() -> LocaleId {
        LocaleId::parse("zh-Hans").expect("zh-Hans ships")
    }

    #[test]
    fn startup_language_prefers_the_environment_then_the_saved_choice() {
        assert_eq!(startup_locale(None, || None), LocaleId::ENGLISH);
        assert_eq!(startup_locale(None, || Some(zh())), zh());
        assert_eq!(startup_locale(Some("zh-hans"), || None), zh());
        assert_eq!(startup_locale(Some("en"), || Some(zh())), LocaleId::ENGLISH);
        // An unknown or blank override falls through to the saved choice.
        assert_eq!(startup_locale(Some("xx-Unknown"), || Some(zh())), zh());
        assert_eq!(startup_locale(Some("  "), || None), LocaleId::ENGLISH);
    }

    #[test]
    fn a_detached_locale_counts_generations_without_touching_the_process() {
        let mut locale = Locale::detached(LocaleId::ENGLISH);
        assert_eq!(locale.generation(), 0);
        locale.set(zh());
        assert_eq!(locale.id(), zh());
        assert_eq!(locale.generation(), 1);
        assert_eq!(active(), LocaleId::ENGLISH);
    }

    /// Startup-spawned `Localized` labels (UI and world text) follow a
    /// language change without a respawn; a literal next to them is left
    /// alone, and an unchanged frame writes nothing.
    #[test]
    fn switching_the_locale_relabels_startup_text_and_text2d() {
        let mut app = App::new();
        app.insert_resource(Locale::detached(LocaleId::ENGLISH))
            .add_systems(PostUpdate, relabel_localized);
        let ui = app
            .world_mut()
            .spawn(Localized::new("pause.title").into_text())
            .id();
        let world = app
            .world_mut()
            .spawn(Localized::new("lane.top").text2d())
            .id();
        let hint = app
            .world_mut()
            .spawn(
                Localized::with_args("pause.settings.server_hint", [("addr", &"game.local:4000")])
                    .into_text(),
            )
            .id();
        let literal = app.world_mut().spawn(Text::new("×")).id();
        app.update();
        assert_eq!(app.world().get::<Text>(ui).unwrap().0, "Game menu");
        assert_eq!(app.world().get::<Text2d>(world).unwrap().0, "TOP");
        app.world_mut().resource_mut::<Locale>().set(zh());
        app.update();
        assert_eq!(app.world().get::<Text>(ui).unwrap().0, "游戏菜单");
        assert_eq!(app.world().get::<Text2d>(world).unwrap().0, "上路");
        assert_eq!(
            app.world().get::<Text>(hint).unwrap().0,
            "服务器：game.local:4000\n设置会自动保存。"
        );
        assert_eq!(app.world().get::<Text>(literal).unwrap().0, "×");
        let tick = app.world().read_change_tick();
        app.update();
        assert!(
            !app.world()
                .entity(ui)
                .get_ref::<Text>()
                .unwrap()
                .last_changed()
                .is_newer_than(tick, app.world().read_change_tick()),
            "an unchanged language must not rewrite text"
        );
        // Changing the key of one label relabels just that label.
        app.world_mut().get_mut::<Localized>(world).unwrap().key = "lane.mid";
        app.update();
        assert_eq!(app.world().get::<Text2d>(world).unwrap().0, "中路");
    }
}
