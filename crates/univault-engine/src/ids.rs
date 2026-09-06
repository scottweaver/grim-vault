// Vendored from tq-univault crates/univault-core/src/chr.rs (RecordId, ItemSeed, GridPos) and src/arz.rs (normalize) @ 36e7774; adapted per docs/engine-extraction.md.
//! Identifier types shared by every engine format: database record
//! paths, item seeds, and grid positions. Both games spell these the
//! same way in their saves and databases; only the parsers around them
//! differ. `RecordId` and `normalize` ported from `TQVaultAE`'s
//! `RecordId` / `NormalizeRecordPath` (MIT).

/// Path of a game database record (`records\...\something.dbr`), the
/// game's identifier for an item base, affix, or relic. Never empty —
/// absence is `Option<RecordId>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordId(String);

impl RecordId {
    /// `None` when `raw` is empty or whitespace-only, which is how the
    /// save formats spell "no record here". Surrounding whitespace is
    /// trimmed, matching `TQVaultAE`'s `RecordId` constructor.
    #[must_use]
    pub fn parse(raw: String) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() == raw.len() {
            Some(Self(raw))
        } else {
            Some(Self(trimmed.to_string()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Final path segment without the `.dbr` extension — the closest
    /// thing to a display name until ARZ text resources are wired up.
    #[must_use]
    pub fn file_stem(&self) -> &str {
        let name = self.0.rsplit(['\\', '/']).next().unwrap_or(self.0.as_str());
        name.strip_suffix(".dbr").unwrap_or(name)
    }
}

impl std::fmt::Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The RNG seed rolled when an item dropped; fixes its stat rolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemSeed(i32);

impl ItemSeed {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self(seed)
    }

    #[must_use]
    pub fn value(self) -> i32 {
        self.0
    }
}

/// Item position in a sack's grid. In Titan Quest saves (-1,-1) never
/// survives parsing — it marks a stack continuation entry and is
/// folded away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

/// `TQVaultAE`'s `NormalizeRecordPath`: uppercase, `/` → `\`. Shared
/// by the ARZ and ARC lookup keys.
#[must_use]
pub fn normalize(path: &str) -> String {
    path.to_uppercase().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_id_rejects_blank_and_trims() {
        assert_eq!(RecordId::parse(String::new()), None);
        assert_eq!(RecordId::parse("   ".to_string()), None);
        assert_eq!(
            RecordId::parse("  records\\a.dbr ".to_string())
                .unwrap()
                .as_str(),
            "records\\a.dbr"
        );
    }

    #[test]
    fn file_stem_drops_directories_and_extension() {
        let id = RecordId::parse("records/items/gearhead/a01_head.dbr".to_string()).unwrap();
        assert_eq!(id.file_stem(), "a01_head");
        let bare = RecordId::parse("loose".to_string()).unwrap();
        assert_eq!(bare.file_stem(), "loose");
    }

    #[test]
    fn normalize_uppercases_and_flips_slashes() {
        assert_eq!(normalize("records/Items/x.dbr"), "RECORDS\\ITEMS\\X.DBR");
    }

    #[test]
    fn item_seed_round_trips() {
        assert_eq!(ItemSeed::new(-7).value(), -7);
    }
}
