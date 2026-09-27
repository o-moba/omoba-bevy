//! Guard tests of the dictionaries (AC2) and the core API (AC1).
//!
//! The source scan (`every_literal_key_in_client_src_exists_in_english`)
//! accepts these call forms, the key being the first argument, written as a
//! string literal:
//!
//! - `tr("ns.key")`, `trf("ns.key", …)`, `tr_in("ns.key", …)`,
//!   `trf_in("ns.key", …)` (plain or path-qualified, e.g. `i18n::tr(`);
//! - `Localized::new("ns.key")`, `Localized::with_args("ns.key", …)`.
//!
//! A key passed through a variable or built at runtime is not seen by that
//! scan; every other string literal in `client/src` that looks like a key of
//! a shipped namespace (`ns.segment[.segment…]`, lowercase) must exist too
//! (`every_key_shaped_literal_in_client_src_exists_in_english`), which
//! covers key tables such as `data::TITLE_KEYS`. Runtime-built keys (game
//! data by id) are covered by `data::tests`.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::*;

fn english_files() -> BTreeMap<&'static str, Vec<(&'static str, &'static str)>> {
    namespace_entries(&LOCALES[0])
}

fn namespace_entries(raw: &RawLocale) -> BTreeMap<&'static str, Vec<(&'static str, &'static str)>> {
    raw.files
        .iter()
        .map(|(namespace, source)| {
            let entries = parse_namespace(raw.code, namespace, source)
                .unwrap_or_else(|error| panic!("{error}"));
            (*namespace, entries)
        })
        .collect()
}

fn zh() -> LocaleId {
    LocaleId::parse("zh-Hans").expect("zh-Hans ships")
}

/// AC2 (a): every shipped locale has the English files and exactly the
/// English key set; nothing missing, nothing extra.
#[test]
fn every_locale_has_exactly_the_english_files_and_keys() {
    let english = english_files();
    assert!(LOCALES.len() >= 2, "English and at least one translation");
    for raw in &LOCALES[1..] {
        let files = namespace_entries(raw);
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            english.keys().collect::<Vec<_>>(),
            "client/i18n/{}/ must have the English namespace files",
            raw.code
        );
        for (namespace, entries) in &files {
            let keys: BTreeSet<_> = entries.iter().map(|(key, _)| *key).collect();
            let wanted: BTreeSet<_> = english[namespace].iter().map(|(key, _)| *key).collect();
            let missing: Vec<_> = wanted.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&wanted).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "client/i18n/{}/{namespace}.json: missing {missing:?}, extra {extra:?}",
                raw.code
            );
        }
    }
}

/// AC2 (b): a translation uses exactly the placeholders of its English text.
#[test]
fn every_translation_uses_the_english_placeholders() {
    let english: HashMap<_, _> = english_files().into_values().flatten().collect();
    for raw in &LOCALES[1..] {
        for (key, text) in namespace_entries(raw).into_values().flatten() {
            assert_eq!(
                placeholders(text),
                placeholders(english[key]),
                "client/i18n/{}: placeholders of {key}",
                raw.code
            );
        }
    }
}

/// AC2 (d): keys start with their file's namespace (checked by the parser),
/// values are non-empty, and `_meta.json` names the folder's code and a
/// native name.
#[test]
fn namespaces_prefix_their_keys_and_meta_files_are_valid() {
    for raw in LOCALES {
        for (namespace, entries) in namespace_entries(raw) {
            assert!(
                !entries.is_empty(),
                "{}/{namespace}.json is empty",
                raw.code
            );
            for (key, text) in entries {
                assert!(key.starts_with(&format!("{namespace}.")));
                assert!(
                    key.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'.')),
                    "{}: key {key:?} must be lowercase ASCII, digits, `_` and `.`",
                    raw.code
                );
                assert!(!text.trim().is_empty(), "{}: {key} is blank", raw.code);
            }
        }
        let meta: serde_json::Value = serde_json::from_str(raw.meta).unwrap();
        assert_eq!(meta["code"], raw.code);
        assert!(
            meta["native_name"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
        );
    }
    assert_eq!(LocaleId::ENGLISH.code(), "en");
    assert_eq!(LocaleId::ENGLISH.native_name(), "English");
    assert_eq!(zh().native_name(), "简体中文");
    // A malformed file is refused with its path and key.
    assert!(parse_namespace("xx", "pause", r#"{"help.title": "x"}"#).is_err());
    assert!(parse_namespace("xx", "pause", r#"{"pause.title": 1}"#).is_err());
    assert!(parse_namespace("xx", "pause", r#"{"pause.": "x"}"#).is_err());
    assert!(parse_namespace("xx", "pause", r#"["pause.title"]"#).is_err());
}

/// AC1: the registry is generated from the folders, so adding a language is
/// adding `client/i18n/<code>/` (no Rust change): the registry lists exactly
/// the folders and files on disk, English first, then by code.
#[test]
fn the_registry_is_exactly_the_locale_folders_on_disk() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n");
    let mut on_disk: Vec<(String, Vec<String>)> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|folder| {
            let mut files: Vec<String> = std::fs::read_dir(&folder)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name != "_meta.json")
                .map(|name| name.trim_end_matches(".json").to_owned())
                .collect();
            files.sort();
            (
                folder.file_name().unwrap().to_string_lossy().into_owned(),
                files,
            )
        })
        .collect();
    on_disk.sort_by_key(|(code, _)| (code != "en", code.clone()));
    let registry: Vec<(String, Vec<String>)> = LOCALES
        .iter()
        .map(|raw| {
            (
                raw.code.to_owned(),
                raw.files.iter().map(|(ns, _)| (*ns).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(registry, on_disk);
    let codes: Vec<_> = available_locales().map(LocaleId::code).collect();
    assert_eq!(codes[0], "en");
    assert_eq!(
        codes,
        registry
            .iter()
            .map(|(code, _)| code.as_str())
            .collect::<Vec<_>>()
    );
    // A locale is found by its code in any case or separator style, and
    // `next` cycles through every locale.
    assert_eq!(LocaleId::parse("ZH_hans"), Some(zh()));
    assert_eq!(LocaleId::parse("xx"), None);
    let mut cycle = LocaleId::ENGLISH;
    for _ in 0..LOCALES.len() {
        cycle = cycle.next();
    }
    assert_eq!(cycle, LocaleId::ENGLISH);
    // Nothing about a locale is code: any folder's files parse the same way.
    static SYNTHETIC: RawLocale = RawLocale {
        code: "xx-Test",
        meta: r#"{"code":"xx-Test","native_name":"Test"}"#,
        files: &[("pause", r#"{"pause.title": "T {n}"}"#)],
    };
    assert_eq!(parse_table(&SYNTHETIC).entries["pause.title"], "T {n}");
}

#[test]
fn lookups_fall_back_to_english_then_to_the_key() {
    let primary = Table {
        entries: HashMap::from([("pause.title", "译")]),
    };
    let fallback = Table {
        entries: HashMap::from([("pause.title", "Game menu"), ("pause.hint.online", "Hint")]),
    };
    assert_eq!(lookup_chain(&primary, &fallback, "pause.title"), Some("译"));
    assert_eq!(
        lookup_chain(&primary, &fallback, "pause.hint.online"),
        Some("Hint")
    );
    assert_eq!(lookup_chain(&primary, &fallback, "zz_none.key"), None);
    const MISSING: &str = "zz_missing.key";
    assert_eq!(tr(MISSING), MISSING);
    assert_eq!(tr_in(MISSING, zh()), MISSING);
    assert_eq!(tr_in("pause.title", LocaleId::ENGLISH), "Game menu");
    assert_eq!(tr_in("pause.title", zh()), "游戏菜单");
    // The process-wide locale of an in-process test is English.
    assert_eq!(active(), LocaleId::ENGLISH);
    assert_eq!(tr("common.back"), "Back");
}

#[test]
fn templates_fill_named_arguments_and_keep_escaped_braces() {
    let mut out = String::new();
    render(
        "{a} and {b}, {{literal}} {missing} { x } }",
        &mut out,
        |name, out| {
            match name {
                "a" => out.push('1'),
                "b" => out.push_str("two"),
                _ => return false,
            }
            true
        },
    );
    assert_eq!(out, "1 and two, {literal} {missing} { x } }");
    assert_eq!(
        placeholders("{a}{{b}}{c}{a}{ d }"),
        BTreeSet::from(["a", "c"])
    );
    assert_eq!(
        trf("pause.settings.server_hint", &[("addr", &"h:1")]),
        "Server: h:1\nSettings are saved automatically."
    );
    assert_eq!(
        trf_in("pause.settings.server_hint", zh(), &[("addr", &42)]),
        "服务器：42\n设置会自动保存。"
    );
    let label = Localized::with_args("pause.settings.server_hint", [("addr", &"a:1")]);
    assert_eq!(label.text_in(zh()), "服务器：a:1\n设置会自动保存。");
}

// --- Source scans (AC2 c) ---

/// Every `.rs` file under `client/src`.
fn client_sources() -> Vec<(PathBuf, String)> {
    fn walk(directory: &Path, out: &mut Vec<(PathBuf, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                out.push((path, source));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// The literal key right after a call's opening parenthesis, if the first
/// argument is a plain string literal: whitespace is skipped in `code` (where
/// comments are blank), the literal is read from `source`.
fn literal_argument<'a>(code: &str, source: &'a str, after: usize) -> Option<&'a str> {
    let skipped = code[after..].len() - code[after..].trim_start().len();
    let body = source[after + skipped..].strip_prefix('"')?;
    let end = body.find('"')?;
    let key = &body[..end];
    (!key.contains('\\')).then_some(key)
}

/// What the lexer found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Span {
    /// A `"…"` literal; the range is its contents.
    Str,
    /// A comment or a raw string (never a key by convention).
    Skipped,
}

/// Comments, string literals and raw strings of a Rust source, as byte
/// ranges. Char literals and lifetimes are stepped over so `'"'` does not
/// open a string.
fn lex(source: &str) -> Vec<(Span, std::ops::Range<usize>)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    let ident = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let end = source[index..]
                    .find('\n')
                    .map_or(bytes.len(), |end| index + end);
                spans.push((Span::Skipped, index..end));
                index = end;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let end = source[index + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| index + 2 + end + 2);
                spans.push((Span::Skipped, index..end));
                index = end;
            }
            b'r' if matches!(bytes.get(index + 1), Some(b'"' | b'#'))
                && (index == 0 || !ident(bytes[index - 1])) =>
            {
                let hashes = bytes[index + 1..]
                    .iter()
                    .take_while(|byte| **byte == b'#')
                    .count();
                if bytes.get(index + 1 + hashes) != Some(&b'"') {
                    index += 1;
                    continue;
                }
                let closing = format!("\"{}", "#".repeat(hashes));
                let start = index + 2 + hashes;
                let end = source[start..]
                    .find(&closing)
                    .map_or(bytes.len(), |end| start + end + closing.len());
                spans.push((Span::Skipped, index..end));
                index = end;
            }
            b'\'' => {
                // A char literal ('x', '\'', '"') or a lifetime ('a).
                if bytes.get(index + 1) == Some(&b'\\') {
                    index = source[index + 2..]
                        .find('\'')
                        .map_or(bytes.len(), |end| index + 2 + end + 1);
                } else if let Some(character) = source[index + 1..].chars().next() {
                    let after = index + 1 + character.len_utf8();
                    index = if bytes.get(after) == Some(&b'\'') {
                        after + 1
                    } else {
                        after
                    };
                } else {
                    index += 1;
                }
            }
            b'"' => {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                let end = end.min(bytes.len());
                spans.push((Span::Str, start..end));
                index = end + 1;
            }
            _ => index += 1,
        }
    }
    spans
}

/// `source` with comments, raw strings and the contents of string literals
/// blanked (offsets and lines kept; a literal keeps its quotes), so the call
/// scan sees code only.
fn code_only(source: &str) -> String {
    let mut code = source.as_bytes().to_vec();
    for (_, range) in lex(source) {
        for byte in &mut code[range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    // Spans start and end at ASCII delimiters, so whole characters were
    // replaced and the offsets still match `source`.
    String::from_utf8(code).expect("blanked spans cover whole characters")
}

/// `(file, line, key)` for every key literal passed to an i18n call.
fn called_keys(sources: &[(PathBuf, String)]) -> Vec<(String, usize, String)> {
    const CALLS: [&str; 6] = [
        "tr(",
        "trf(",
        "tr_in(",
        "trf_in(",
        "Localized::new(",
        "Localized::with_args(",
    ];
    let mut found = Vec::new();
    for (path, source) in sources {
        let code = code_only(source);
        for call in CALLS {
            for (offset, _) in code.match_indices(call) {
                let preceding = code[..offset].chars().next_back();
                if preceding.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                    continue;
                }
                if let Some(key) = literal_argument(&code, source, offset + call.len()) {
                    found.push((
                        path.display().to_string(),
                        line_of(&code, offset),
                        key.to_owned(),
                    ));
                }
            }
        }
    }
    found
}

/// The contents of every plain `"…"` literal outside comments.
fn string_literals(source: &str) -> Vec<(usize, &str)> {
    lex(source)
        .into_iter()
        .filter(|(kind, _)| *kind == Span::Str)
        .map(|(_, range)| (range.start, &source[range]))
        .collect()
}

/// `ns.segment[.segment…]` in lowercase ASCII, `_` and digits.
fn looks_like_key(text: &str) -> bool {
    let mut segments = text.split('.');
    let first_ok = segments.next().is_some_and(|first| {
        first
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
            && first
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    });
    let rest: Vec<_> = segments.collect();
    first_ok
        && !rest.is_empty()
        && rest.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

/// AC2 (c): every key literal passed to `tr`/`trf`/`tr_in`/`trf_in`/
/// `Localized::{new, with_args}` in `client/src` exists in English.
#[test]
fn every_literal_key_in_client_src_exists_in_english() {
    let sources = client_sources();
    let keys = called_keys(&sources);
    assert!(
        keys.iter().any(|(_, _, key)| key == "pause.title"),
        "the scan must see the pause menu's keys"
    );
    let missing: Vec<_> = keys
        .iter()
        .filter(|(_, _, key)| lookup_in(LocaleId::ENGLISH, key).is_none())
        .collect();
    assert!(missing.is_empty(), "keys missing in English: {missing:#?}");
}

/// AC2 (c), second net: a key-shaped literal of a shipped namespace anywhere
/// in `client/src` (a key table, a `match` arm) must exist in English.
#[test]
fn every_key_shaped_literal_in_client_src_exists_in_english() {
    let namespaces: BTreeSet<&str> = LOCALES[0].files.iter().map(|(ns, _)| *ns).collect();
    let mut checked = 0;
    let mut missing = Vec::new();
    for (path, source) in client_sources() {
        for (offset, literal) in string_literals(&source) {
            if !looks_like_key(literal) {
                continue;
            }
            let namespace = literal.split('.').next().unwrap_or_default();
            if !namespaces.contains(namespace) {
                continue;
            }
            checked += 1;
            if lookup_in(LocaleId::ENGLISH, literal).is_none() {
                missing.push(format!(
                    "{}:{}: {literal}",
                    path.display(),
                    line_of(&source, offset)
                ));
            }
        }
    }
    assert!(checked > 50, "the scan must see the key tables ({checked})");
    assert!(
        missing.is_empty(),
        "key-shaped literals missing in English: {missing:#?}"
    );
}

#[test]
fn the_scanners_read_rust_literals_correctly() {
    let source = r##"
        // tr("in.comment") is a comment, not a call
        let a = tr("pause.title"); let b = attr("pause.none"); x.tr("pause.none");
        let c = '"'; let d = 'x'; fn f<'a>(v: &'a str) {}
        let e = Localized::with_args(
            "pause.settings.server_hint", [("addr", &x)]);
        let raw = r#"lane.top "quoted" "#; let s = "a \" lane.mid";
        /* "lane.bot" */ let g = "lane.base";
    "##;
    let sources = vec![(PathBuf::from("probe.rs"), source.to_owned())];
    let keys: Vec<_> = called_keys(&sources)
        .into_iter()
        .map(|(_, _, key)| key)
        .collect();
    assert_eq!(keys, ["pause.title", "pause.settings.server_hint"]);
    let literals: Vec<_> = string_literals(source)
        .into_iter()
        .map(|(_, literal)| literal)
        .collect();
    assert!(literals.contains(&"lane.base"));
    assert!(literals.contains(&"pause.title"));
    assert!(!literals.contains(&"lane.bot"), "comments are skipped");
    assert!(literals.contains(&"a \\\" lane.mid"));
    assert!(!literals.iter().any(|literal| literal.contains("quoted")));
    assert!(looks_like_key("hero.warrior.name"));
    assert!(!looks_like_key("hero."));
    assert!(!looks_like_key("ui/Inter.ttf"));
    assert!(!looks_like_key("Hero.name"));
}

// --- Hard-coded English (AC3) ---

/// A module whose player-facing text is fully migrated carries this line;
/// the strict scan then refuses any English literal left in it.
const STRICT_MARKER: &str = "// i18n-strict";
/// A trailing comment that allows one literal on its line (a proper name, a
/// glyph the scan misreads).
const ALLOW_MARKER: &str = "// i18n-allow";

/// Calls whose string arguments are identities or diagnostics, never shown
/// to players.
const NON_UI_CALLS: [&str; 17] = [
    "Name::new",
    "TestId::new",
    "child",
    "info!",
    "warn!",
    "error!",
    "debug!",
    "trace!",
    "panic!",
    "unreachable!",
    "expect",
    "assert!",
    "assert_eq!",
    "assert_ne!",
    "debug_assert!",
    "load",
    "eprintln!",
];

/// The callee of the innermost open parenthesis before `offset` in `code`
/// (comments and literals blanked), e.g. `Name::new` or `info!`.
fn enclosing_call(code: &str, offset: usize) -> &str {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut index = offset;
    while index > 0 {
        index -= 1;
        match bytes[index] {
            b')' | b']' | b'}' => depth += 1,
            b'(' if depth == 0 => {
                let end = index;
                let start = code[..end]
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '!'))
                    .map_or(0, |at| at + 1);
                return &code[start..end];
            }
            b'[' | b'{' if depth == 0 => return "",
            b'(' | b'[' | b'{' => depth -= 1,
            _ => {}
        }
    }
    ""
}

/// An identifier-like literal (`SettingsButton`, `pause_menu`, `CONST`):
/// no whitespace, and CamelCase, snake/lowercase or with `_`.
fn is_identifier_like(text: &str) -> bool {
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    let upper = text.chars().filter(char::is_ascii_uppercase).count();
    let lower = text.chars().filter(char::is_ascii_lowercase).count();
    (lower > 0 && (upper == 0 || upper >= 2)) || text.contains('_')
}

/// Text a player could read: at least two consecutive ASCII letters, not a
/// key, identifier, path or format-only string.
fn is_english_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    let has_word = bytes
        .windows(2)
        .any(|pair| pair[0].is_ascii_alphabetic() && pair[1].is_ascii_alphabetic());
    let path_like = !text.contains(' ')
        && (text.contains('/') || text.contains("::") || text.ends_with(".json"));
    has_word && !looks_like_key(text) && !is_identifier_like(text) && !path_like
}

/// `(line, literal)` of English UI literals outside the file's test module.
fn hardcoded_english(source: &str) -> Vec<(usize, String)> {
    let code = code_only(source);
    let end = code
        .match_indices("#[cfg(test)]")
        .map(|(at, attribute)| (at, &code[at + attribute.len()..]))
        .find(|(_, rest)| rest.trim_start().starts_with("mod tests"))
        .map_or(code.len(), |(at, _)| at);
    let lines: Vec<&str> = source.lines().collect();
    string_literals(source)
        .into_iter()
        .filter(|(offset, _)| *offset < end)
        .filter(|(_, literal)| is_english_text(literal))
        .filter(|(offset, _)| !NON_UI_CALLS.contains(&enclosing_call(&code, *offset)))
        .map(|(offset, literal)| (line_of(source, offset), literal.to_owned()))
        .filter(|(line, _)| !lines[line - 1].contains(ALLOW_MARKER))
        .collect()
}

/// AC3: a module marked `// i18n-strict` shows no hard-coded English; all its
/// player-facing text comes from `tr`/`trf`/`Localized`/`i18n::data`. Phase B
/// marks each module as it migrates it; the final pass makes the scan cover
/// every player-facing module (dev-only modules excluded).
#[test]
fn strict_modules_have_no_hardcoded_english() {
    let mut strict = 0;
    let mut found = Vec::new();
    for (path, source) in client_sources() {
        if !source.lines().any(|line| line.trim() == STRICT_MARKER) {
            continue;
        }
        strict += 1;
        for (line, literal) in hardcoded_english(&source) {
            found.push(format!("{}:{line}: {literal:?}", path.display()));
        }
    }
    assert!(strict >= 2, "pause_menu and help_overlay are strict");
    assert!(
        found.is_empty(),
        "hard-coded English in strict modules: {found:#?}"
    );
}

/// Progress report for the migration: every remaining English literal in
/// `client/src`, by file. `cargo test -p client report_hardcoded_english --
/// --ignored --nocapture`.
#[test]
#[ignore = "report for the i18n migration, not a gate"]
fn report_hardcoded_english() {
    let mut total = 0;
    for (path, source) in client_sources() {
        let found = hardcoded_english(&source);
        if found.is_empty() {
            continue;
        }
        total += found.len();
        println!("{} ({})", path.display(), found.len());
        for (line, literal) in found {
            println!("  {line}: {literal:?}");
        }
    }
    println!("total: {total}");
}

#[test]
fn the_english_scan_skips_identities_diagnostics_and_keys() {
    let source = r##"
        fn f() {
            parent.spawn((Text::new("Game menu"), Name::new("PauseMenuTitle")));
            widgets::button(p, "Reset graphics", Kind::A, Action::B, "PauseMenuResetButton");
            info!("Pause menu {}", if open { "opened" } else { "closed" });
            let a = tr("pause.title"); let b = format!("{:.0}%", 1.0); let c = "×";
            let d = "Proper Noun"; // i18n-allow
            let e = "OK"; let f = assets.load("ui/Inter.ttf");
            TestId::new(format!("QuickBuy-{slot}")).child("-Down");
        }
        #[cfg(test)]
        mod tests { fn t() { assert!(x == "Game menu") ; let y = "Hidden text"; } }
    "##;
    let found: Vec<_> = hardcoded_english(source)
        .into_iter()
        .map(|(_, literal)| literal)
        .collect();
    assert_eq!(found, ["Game menu", "Reset graphics", "OK"]);
}

/// A test that needs the process-wide language switched runs alone in a
/// child process; the parent's language stays English.
#[test]
fn isolated_tests_switch_the_process_language_in_a_child_process() {
    if testing::isolated(
        "i18n::tests::isolated_tests_switch_the_process_language_in_a_child_process",
    ) {
        assert_eq!(active(), LocaleId::ENGLISH);
        return;
    }
    let mut app = App::new();
    app.add_plugins(I18nPlugin::fixed(zh()));
    assert_eq!(active(), zh());
    assert_eq!(tr("pause.title"), "游戏菜单");
    app.world_mut()
        .resource_mut::<Locale>()
        .set(LocaleId::ENGLISH);
    assert_eq!(
        tr("pause.title"),
        "Game menu",
        "Locale::set drives tr at once"
    );
}
