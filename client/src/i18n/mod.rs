//! Player-facing text by key, from dictionaries embedded at build time.
//!
//! `client/i18n/<locale>/<namespace>.json` holds flat `"namespace.key":
//! "text"` objects and `<locale>/_meta.json` the locale's code and native
//! name. `client/build.rs` scans the folders and generates the registry, so a
//! language is added by adding a folder; nothing here names a locale except
//! the English fallback. `docs/i18n.md` is the guide (format, patterns,
//! glossary).
//!
//! - [`tr`] / [`trf`] look a key up in the active locale, then in English,
//!   then return the key itself. `tr` is one hash lookup and never allocates.
//! - The active locale is process-wide (an atomic), so spawn helpers without
//!   resource access can call `tr`. Only [`Locale::set`] on the resource the
//!   [`I18nPlugin`] inserted changes it.
//! - [`Localized`] labels are rewritten by one system when the [`Locale`]
//!   resource changes; per-frame writers call `tr`/`trf` and re-run on a
//!   `Locale` change; render-key caches include [`Locale::generation`].
//! - [`data`] turns game data (heroes, items, reactions, errors, …) into
//!   display text by id.
use std::collections::HashMap;
use std::fmt::{Display, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, OnceLock};

use bevy::prelude::*;
use serde_json::{Map, Value};

pub(crate) mod data;
mod plugin;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

pub(crate) use plugin::{I18nPlugin, I18nSystems, Locale, configure_text_sets, locale_changed};
#[cfg(test)]
pub(crate) use plugin::{LANGUAGE_ENV, relabel_localized};

/// One locale folder as `build.rs` embedded it.
struct RawLocale {
    code: &'static str,
    meta: &'static str,
    /// `(namespace, file contents)`, sorted by namespace.
    files: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/i18n_bundles.rs"));

/// A locale's `_meta.json`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    code: String,
    native_name: String,
}

/// Parsed `_meta.json` of every locale, in registry order (English first).
static METAS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    LOCALES
        .iter()
        .map(|raw| {
            let meta: Meta = serde_json::from_str(raw.meta)
                .unwrap_or_else(|error| panic!("client/i18n/{}/_meta.json: {error}", raw.code));
            assert_eq!(
                meta.code, raw.code,
                "client/i18n/{}/_meta.json names another code",
                raw.code
            );
            assert!(
                !meta.native_name.trim().is_empty(),
                "client/i18n/{}/_meta.json has no native_name",
                raw.code
            );
            &*Box::leak(meta.native_name.into_boxed_str())
        })
        .collect()
});

/// One locale's dictionary, parsed on first use.
struct Table {
    entries: HashMap<&'static str, &'static str>,
}

static TABLES: LazyLock<Vec<OnceLock<Table>>> =
    LazyLock::new(|| LOCALES.iter().map(|_| OnceLock::new()).collect());

/// Parses one namespace file: a flat object of string values whose keys all
/// start with `"{namespace}."`. Strings are leaked once per process, like the
/// shared catalog, so lookups hand out `&'static str`.
fn parse_namespace(
    code: &str,
    namespace: &str,
    source: &str,
) -> Result<Vec<(&'static str, &'static str)>, String> {
    let object: Map<String, Value> = serde_json::from_str(source)
        .map_err(|error| format!("client/i18n/{code}/{namespace}.json: {error}"))?;
    let prefix = format!("{namespace}.");
    object
        .into_iter()
        .map(|(key, value)| {
            if !key.starts_with(&prefix) || key.len() == prefix.len() {
                return Err(format!(
                    "client/i18n/{code}/{namespace}.json: key {key:?} must start with {prefix:?}"
                ));
            }
            let Value::String(text) = value else {
                return Err(format!(
                    "client/i18n/{code}/{namespace}.json: {key:?} is not a string"
                ));
            };
            Ok((
                &*Box::leak(key.into_boxed_str()),
                &*Box::leak(text.into_boxed_str()),
            ))
        })
        .collect()
}

fn parse_table(raw: &RawLocale) -> Table {
    let mut entries = HashMap::new();
    for (namespace, source) in raw.files {
        for (key, text) in parse_namespace(raw.code, namespace, source)
            .unwrap_or_else(|error| panic!("invalid dictionary {error}"))
        {
            entries.insert(key, text);
        }
    }
    Table { entries }
}

fn table(locale: LocaleId) -> &'static Table {
    TABLES[locale.0].get_or_init(|| parse_table(&LOCALES[locale.0]))
}

/// Parses English and the active locale now, so a malformed dictionary stops
/// the process at startup instead of on the first lookup (like
/// `shared::catalog::ensure_loaded`).
pub(crate) fn ensure_loaded() {
    LazyLock::force(&METAS);
    table(LocaleId::ENGLISH);
    table(active());
}

/// A locale that exists in the registry. Copy, compares by registry index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct LocaleId(usize);

impl LocaleId {
    /// The fallback locale; `build.rs` puts it first.
    pub(crate) const ENGLISH: Self = Self(0);

    /// The locale with this code. Case and `_`/`-` are ignored, so
    /// `zh_hans` finds `zh-Hans`; an unknown code is `None`.
    pub(crate) fn parse(code: &str) -> Option<Self> {
        let wanted = code.trim();
        LOCALES
            .iter()
            .position(|raw| {
                raw.code.len() == wanted.len()
                    && raw.code.bytes().zip(wanted.bytes()).all(|(a, b)| {
                        let fold = |byte: u8| {
                            if byte == b'_' {
                                b'-'
                            } else {
                                byte.to_ascii_lowercase()
                            }
                        };
                        fold(a) == fold(b)
                    })
            })
            .map(Self)
    }

    /// The code as the folder names it (`en`, `zh-Hans`).
    pub(crate) fn code(self) -> &'static str {
        LOCALES[self.0].code
    }

    /// The name of the language in itself (`English`, `简体中文`).
    pub(crate) fn native_name(self) -> &'static str {
        METAS[self.0]
    }

    /// The next locale in [`available_locales`] order, wrapping around (the
    /// settings row cycles with it).
    pub(crate) fn next(self) -> Self {
        available_locales()
            .cycle()
            .skip_while(|locale| *locale != self)
            .nth(1)
            .unwrap_or(Self::ENGLISH)
    }
}

impl Default for LocaleId {
    fn default() -> Self {
        Self::ENGLISH
    }
}

/// Every shipped locale, English first, then by code.
pub(crate) fn available_locales() -> impl ExactSizeIterator<Item = LocaleId> + Clone {
    (0..LOCALES.len()).map(LocaleId)
}

/// The process-wide active locale (a registry index).
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// The locale `tr` and `trf` read.
pub(crate) fn active() -> LocaleId {
    LocaleId(ACTIVE.load(Ordering::Relaxed))
}

/// Only [`Locale`] calls this, and only for the resource the plugin owns.
fn store_active(locale: LocaleId) {
    ACTIVE.store(locale.0, Ordering::Relaxed);
}

/// `key` in `primary`, else in `fallback`.
fn lookup_chain(primary: &Table, fallback: &Table, key: &str) -> Option<&'static str> {
    primary
        .entries
        .get(key)
        .or_else(|| fallback.entries.get(key))
        .copied()
}

/// `key` in `locale`, else in English, else `None`.
pub(crate) fn lookup_in(locale: LocaleId, key: &str) -> Option<&'static str> {
    lookup_chain(table(locale), table(LocaleId::ENGLISH), key)
}

/// `key` in the active locale, else English, else `None`. For keys built at
/// runtime (game data by id); a literal key uses [`tr`].
pub(crate) fn lookup(key: &str) -> Option<&'static str> {
    lookup_in(active(), key)
}

/// The text of `key` in the active locale, falling back to English, then to
/// the key itself (so a missing entry is visible, never blank).
pub(crate) fn tr(key: &'static str) -> &'static str {
    tr_in(key, active())
}

/// [`tr`] in an explicit locale.
pub(crate) fn tr_in(key: &'static str, locale: LocaleId) -> &'static str {
    lookup_in(locale, key).unwrap_or(key)
}

/// [`tr`] with named `{name}` placeholders filled from `args`
/// (`{{` and `}}` are literal braces; an unknown placeholder stays as
/// written).
///
/// ```text
/// trf("pause.settings.server_hint", &[("addr", &addr)])
///   => "Server: 127.0.0.1:4000\nSettings are saved automatically."
/// ```
pub(crate) fn trf(key: &'static str, args: &[(&str, &dyn Display)]) -> String {
    trf_in(key, active(), args)
}

/// [`trf`] in an explicit locale.
pub(crate) fn trf_in(key: &'static str, locale: LocaleId, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::new();
    render(tr_in(key, locale), &mut out, |name, out| {
        args.iter()
            .find(|(arg, _)| *arg == name)
            .map(|(_, value)| {
                let _ = write!(out, "{value}");
            })
            .is_some()
    });
    out
}

/// One piece of a template.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Literal(&'a str),
    Placeholder(&'a str),
}

/// Splits a template into literal runs and `{name}` placeholders. `{{` and
/// `}}` are literal braces; a `{` without a valid name and closing `}` is
/// literal text.
fn pieces(template: &str) -> impl Iterator<Item = Piece<'_>> {
    let mut rest = template;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        if let Some(after) = rest.strip_prefix("{{") {
            rest = after;
            return Some(Piece::Literal("{"));
        }
        if let Some(after) = rest.strip_prefix("}}") {
            rest = after;
            return Some(Piece::Literal("}"));
        }
        if let Some(after) = rest.strip_prefix('{') {
            if let Some(end) = after.find('}') {
                let name = &after[..end];
                if is_placeholder_name(name) {
                    rest = &after[end + 1..];
                    return Some(Piece::Placeholder(name));
                }
            }
            rest = after;
            return Some(Piece::Literal("{"));
        }
        let end = rest
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '{' | '}'))
            .map_or(rest.len(), |(index, _)| index);
        let (literal, after) = rest.split_at(end);
        rest = after;
        Some(Piece::Literal(literal))
    })
}

fn is_placeholder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Renders `template` into `out`; `arg(name, out)` appends a placeholder's
/// value and returns whether it knew the name.
fn render(template: &str, out: &mut String, mut arg: impl FnMut(&str, &mut String) -> bool) {
    for piece in pieces(template) {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Placeholder(name) => {
                if !arg(name, out) {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
        }
    }
}

/// The placeholder names a template uses (the guard tests compare them
/// across locales).
#[cfg(test)]
pub(crate) fn placeholders(template: &str) -> std::collections::BTreeSet<&str> {
    pieces(template)
        .filter_map(|piece| match piece {
            Piece::Placeholder(name) => Some(name),
            Piece::Literal(_) => None,
        })
        .collect()
}

/// A text entity that follows the language: [`I18nPlugin`] rewrites its
/// `Text`, `Text2d` or `TextSpan` whenever the [`Locale`] changes and when
/// this component is added or changed. Spawn it next to the text it owns,
/// already filled in the active locale (the [`UiLabel`] impl does both), so
/// the first frame is right without the plugin.
///
/// Use it only for text whose key never depends on state; a label that
/// switches between keys (mute/unmute) is written by its owner with `tr`.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Localized {
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl Localized {
    pub(crate) fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    /// A template key with its placeholder values.
    pub(crate) fn with_args<const N: usize>(
        key: &'static str,
        args: [(&'static str, &dyn Display); N],
    ) -> Self {
        Self {
            key,
            args: args
                .into_iter()
                .map(|(name, value)| (name, value.to_string()))
                .collect(),
        }
    }

    /// The text in the active locale.
    pub(crate) fn text(&self) -> String {
        self.text_in(active())
    }

    /// The text in `locale`.
    pub(crate) fn text_in(&self, locale: LocaleId) -> String {
        let template = tr_in(self.key, locale);
        if self.args.is_empty() {
            return template.to_owned();
        }
        let mut out = String::with_capacity(template.len() + 16);
        render(template, &mut out, |name, out| {
            self.args
                .iter()
                .find(|(arg, _)| *arg == name)
                .map(|(_, value)| out.push_str(value))
                .is_some()
        });
        out
    }

    /// `(Text, Localized)` for a UI label, filled in the active language.
    pub(crate) fn into_text(self) -> (Text, Self) {
        (Text::new(self.text()), self)
    }

    /// `(Text2d, Localized)` for a world-space label.
    pub(crate) fn text2d(self) -> (Text2d, Self) {
        (Text2d::new(self.text()), self)
    }
}

/// Text a widget can show: a literal (`&str`, `String`) becomes a plain
/// `Text`; a [`Localized`] becomes `(Text, Localized)`, filled in the active
/// locale and relabelled on a language change. The kit widgets take
/// `impl UiLabel`, so `widgets::button(parent, Localized::new("pause.title"), …)`
/// and `widgets::button(parent, "×", …)` both work.
pub(crate) trait UiLabel {
    type Bundle: Bundle;
    fn into_text(self) -> Self::Bundle;
}

impl UiLabel for &str {
    type Bundle = Text;
    fn into_text(self) -> Text {
        Text::new(self)
    }
}

impl UiLabel for &String {
    type Bundle = Text;
    fn into_text(self) -> Text {
        Text::new(self.as_str())
    }
}

impl UiLabel for String {
    type Bundle = Text;
    fn into_text(self) -> Text {
        Text::new(self)
    }
}

impl UiLabel for Localized {
    type Bundle = (Text, Localized);
    fn into_text(self) -> (Text, Localized) {
        Localized::into_text(self)
    }
}
