//! How the game shows the registry's `license.attribution` line for Ekza
//! Studio items: the credit a CC-BY item needs wherever the game names it.
//!
//! The SDK keeps the line on its store templates (`StoreAsset::attribution`,
//! `StoreAvatar::attribution`), filled from the `/v2/assets` and
//! `/v2/avatars` items and from the account library, and saves it with each
//! store's offline catalogue. It is display text only: it never decides what
//! is listed, installed or admitted.

/// Longest line shown, in characters; longer text ends with an ellipsis.
pub const MAX_CHARS: usize = 300;

/// Creator text as shown: whitespace and control characters collapse to single
/// spaces and the line is bounded. Blank text is `None`.
pub fn clean(text: &str) -> Option<String> {
    let line = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= MAX_CHARS {
        return Some(line);
    }
    let mut cut: String = line.chars().take(MAX_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creator_text_is_one_bounded_line() {
        assert_eq!(clean("  \n\t "), None);
        assert_eq!(
            clean(" Pyre Sorcerer by\nPolygonal Mind,\u{7}  CC0 1.0 ").as_deref(),
            Some("Pyre Sorcerer by Polygonal Mind, CC0 1.0")
        );
        let long = clean(&"word ".repeat(200)).unwrap();
        assert_eq!(long.chars().count(), MAX_CHARS);
        assert!(long.ends_with("word…"), "{long}");
        assert!(!long.contains("  "));
    }
}
