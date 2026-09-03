// Vendored from tq-univault crates/univault-core/src/text.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Localization tag table built from the text files inside
//! `Text_XX.arc` — the `tagSwordName01=Bronze Sword` side of item
//! naming. Ported from `TQVaultAE`'s `Database.ParseTextDB` (MIT).
//!
//! Each text file is line-oriented `tag=label`. Encoding is sniffed
//! from the BOM (the game ships UTF-16LE; Windows-1252 is the
//! no-BOM fallback, matching `StreamReader`'s behavior with
//! `Encoding.Default`). Labels for gendered languages carry metadata
//! like `[ms]Épée[fs]…`; only the first form is kept, like
//! `TQVaultAE`.

use std::collections::HashMap;

use crate::reader::decode_windows_1252;

/// Tag → localized label table. Tags match case-insensitively; files
/// added later override earlier entries (base game, then expansions).
#[derive(Debug, Default, Clone)]
pub struct TextDb {
    entries: HashMap<String, String>,
}

impl TextDb {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Merges one text file's `tag=label` lines into the table.
    pub fn add_file(&mut self, bytes: &[u8]) {
        let content = decode(bytes);
        for line in content.lines() {
            let line = line.trim();
            if line.len() < 2 || line.starts_with("//") {
                continue;
            }
            let mut fields = line.split('=');
            let (Some(tag), Some(label)) = (fields.next(), fields.next()) else {
                continue;
            };
            let tag = tag.trim().to_uppercase();
            if tag.is_empty() {
                continue;
            }
            let label = strip_color_tags(clean_label(label.trim()))
                .trim()
                .to_string();
            self.entries.insert(tag, label);
        }
    }

    /// Looks up a tag (case-insensitive).
    #[must_use]
    pub fn get(&self, tag: &str) -> Option<&str> {
        self.entries.get(&tag.to_uppercase()).map(String::as_str)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn decode(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => decode_windows_1252(bytes),
    }
}

fn decode_utf16(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> String {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| from_bytes(pair));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// `TQVaultAE`'s label cleanup regex
/// `^(?<Tag>\[\w+\])(?<Label>[^\[]+)|^\[(?<Label>[^\]]+)\]$`: a
/// leading `[tag]` selects the text after it up to the next `[`; a
/// label that is entirely `[text]` unwraps to the text.
fn clean_label(label: &str) -> &str {
    let Some(rest) = label.strip_prefix('[') else {
        return label;
    };
    let Some((tag, after)) = rest.split_once(']') else {
        return label;
    };
    let tag_is_word = !tag.is_empty() && tag.chars().all(|c| c.is_alphanumeric() || c == '_');
    if tag_is_word && !after.is_empty() && !after.starts_with('[') {
        let end = after.find('[').unwrap_or(after.len());
        return &after[..end];
    }
    if after.is_empty() && !tag.is_empty() && !tag.contains(']') {
        return tag;
    }
    label
}

/// Removes the game's inline color codes — `{^X}` and bare `^X` —
/// matching `TQVaultAE`'s `RegExTQTag` / `RemoveAllTQTags`.
fn strip_color_tags(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(current) = chars.next() {
        match current {
            '{' => {
                let mut lookahead = chars.clone();
                let is_tag = lookahead.next() == Some('^')
                    && lookahead.next().is_some_and(is_word)
                    && lookahead.next() == Some('}');
                if is_tag {
                    chars = lookahead;
                } else {
                    out.push(current);
                }
            }
            '^' if chars.peek().copied().is_some_and(is_word) => {
                chars.next();
            }
            _ => out.push(current),
        }
    }
    out
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16_file(content: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn parses_utf16_tag_lines() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file(
            "// item names\ntagSwordName01=Bronze Sword\n\ntagHelm=Χαλκός Helm\n",
        ));
        assert_eq!(db.get("tagSwordName01"), Some("Bronze Sword"));
        assert_eq!(db.get("TAGHELM"), Some("Χαλκός Helm"));
        assert_eq!(db.len(), 2);
    }

    #[test]
    fn windows_1252_without_bom_is_the_fallback() {
        let mut db = TextDb::new();
        db.add_file(b"tagCoin=P\xE9pite d'or\n");
        assert_eq!(db.get("tagCoin"), Some("Pépite d'or"));
    }

    #[test]
    fn utf8_bom_is_honoured() {
        let mut db = TextDb::new();
        db.add_file("\u{FEFF}tagCoin=Pépite d'or\n".as_bytes());
        assert_eq!(db.get("tagCoin"), Some("Pépite d'or"));
    }

    #[test]
    fn gendered_metadata_keeps_first_form() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file("tagSword=[ms]Épée[fs]Épées\n"));
        assert_eq!(db.get("tagSword"), Some("Épée"));
    }

    #[test]
    fn fully_bracketed_label_unwraps() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file("tagX=[Some Label]\n"));
        assert_eq!(db.get("tagX"), Some("Some Label"));
    }

    #[test]
    fn later_files_override_earlier_ones() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file("tagSword=Old Name\n"));
        db.add_file(&utf16_file("tagSword=New Name\n"));
        assert_eq!(db.get("tagSword"), Some("New Name"));
    }

    #[test]
    fn value_after_second_equals_is_dropped_like_tqvaultae() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file("tagEq=first=second\n"));
        assert_eq!(db.get("tagEq"), Some("first"));
    }

    #[test]
    fn color_codes_are_stripped_from_labels() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file(
            "tagA={^l}Redfist Battle Guards\ntagB=Ink ^rRed^_ Mix\ntagC={not a tag}\n",
        ));
        assert_eq!(db.get("tagA"), Some("Redfist Battle Guards"));
        assert_eq!(db.get("tagB"), Some("Ink Red Mix"));
        assert_eq!(db.get("tagC"), Some("{not a tag}"));
    }

    #[test]
    fn comments_and_short_lines_are_skipped() {
        let mut db = TextDb::new();
        db.add_file(&utf16_file("//tagA=Hidden\nx\n=\ntagB=Ok\n"));
        assert_eq!(db.get("tagA"), None);
        assert_eq!(db.get("tagB"), Some("Ok"));
        assert_eq!(db.len(), 1);
    }
}
