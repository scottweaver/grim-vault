//! What the game data says about an item, resolved once and memoized.
//! The panes ask per painted tile every frame, and the layered lookups
//! behind the answer — record → tag → text, bitmap → archive entry →
//! `.tex` header — are far too slow to repeat at that rate.

use std::collections::HashMap;

use grimvault_core::bucket::Bucket;
use grimvault_core::gamedata::{BitmapPath, Footprint, GameData, ItemClass, Rarity};
use grimvault_core::item::Item;
use grimvault_core::transfer::Footprints;
use univault_engine::ids::RecordId;

/// Whether the database had the base record at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordStatus {
    Known,
    Unknown,
}

/// Everything the panes need about one base record.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseFacts {
    pub name: String,
    pub record: RecordStatus,
    pub rarity: Option<Rarity>,
    pub class: Option<ItemClass>,
    pub level_requirement: Option<u32>,
    pub footprint: Option<Footprint>,
    pub bitmap: Option<BitmapPath>,
    pub bucket: Bucket,
}

/// One item's facts: its base record's, with the affix names the
/// database could resolve.
#[derive(Clone, Copy, Debug)]
pub struct ItemFacts<'a> {
    pub base: &'a BaseFacts,
    pub prefix: Option<&'a str>,
    pub suffix: Option<&'a str>,
}

impl ItemFacts<'_> {
    /// `Prefix Base Suffix`, parts the database cannot name omitted.
    #[must_use]
    pub fn display_name(&self) -> String {
        [self.prefix, Some(self.base.name.as_str()), self.suffix]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Up to two initials of the base name, for the fallback tile.
    #[must_use]
    pub fn initials(&self) -> String {
        self.base
            .name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
}

/// The memo: base facts by base record path, affix names by affix
/// record path. Only [`Footprints`] reads it without warming, so a
/// caller that will move items warms every item involved first
/// ([`FactsCache::warm`]).
#[derive(Default)]
pub struct FactsCache {
    bases: HashMap<String, BaseFacts>,
    affixes: HashMap<String, Option<String>>,
}

impl FactsCache {
    /// The facts for `item`, resolving and memoizing on first sight.
    pub fn facts(&mut self, game: &GameData, item: &Item) -> ItemFacts<'_> {
        self.warm(game, item);
        let base = self
            .bases
            .get(&item.base_name)
            .unwrap_or_else(|| unreachable!("warm inserted the base"));
        let affix = |name: &str| self.affixes.get(name).and_then(|name| name.as_deref());
        ItemFacts {
            base,
            prefix: affix(&item.prefix_name),
            suffix: affix(&item.suffix_name),
        }
    }

    /// The base facts alone.
    pub fn base(&mut self, game: &GameData, item: &Item) -> &BaseFacts {
        self.warm(game, item);
        self.bases
            .get(&item.base_name)
            .unwrap_or_else(|| unreachable!("warm inserted the base"))
    }

    /// Resolves and memoizes everything about `item` so later reads —
    /// including the [`Footprints`] view — can answer.
    pub fn warm(&mut self, game: &GameData, item: &Item) {
        if !self.bases.contains_key(&item.base_name) {
            self.bases
                .insert(item.base_name.clone(), resolve_base(game, &item.base_name));
        }
        for affix in [&item.prefix_name, &item.suffix_name] {
            if !affix.is_empty() && !self.affixes.contains_key(affix) {
                let name = RecordId::parse(affix.clone()).and_then(|id| game.affix_name(&id));
                self.affixes.insert(affix.clone(), name);
            }
        }
    }

    /// Warms every item of an iterator.
    pub fn warm_all<'i>(&mut self, game: &GameData, items: impl IntoIterator<Item = &'i Item>) {
        for item in items {
            self.warm(game, item);
        }
    }
}

impl Footprints for FactsCache {
    fn footprint(&self, item: &Item) -> Option<Footprint> {
        self.bases.get(&item.base_name)?.footprint
    }
}

fn resolve_base(game: &GameData, base_name: &str) -> BaseFacts {
    let Some(id) = RecordId::parse(base_name.to_string()) else {
        return unknown("<empty record>".to_string());
    };
    let Some(Ok(info)) = game.item_info(&id) else {
        return unknown(format!("<unknown> {}", id.file_stem()));
    };
    let footprint = info
        .bitmap
        .as_ref()
        .and_then(|bitmap| game.footprint(bitmap))
        .and_then(Result::ok);
    let bucket = info.class.as_ref().map_or(Bucket::Misc, Bucket::of);
    BaseFacts {
        name: info.name,
        record: RecordStatus::Known,
        rarity: info.rarity,
        class: info.class,
        level_requirement: info.level_requirement,
        footprint,
        bitmap: info.bitmap,
        bucket,
    }
}

fn unknown(name: String) -> BaseFacts {
    BaseFacts {
        name,
        record: RecordStatus::Unknown,
        rarity: None,
        class: None,
        level_requirement: None,
        footprint: None,
        bitmap: None,
        bucket: Bucket::Misc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        name: &str,
        prefix: Option<&str>,
        suffix: Option<&str>,
    ) -> (BaseFacts, Option<String>, Option<String>) {
        (
            unknown(name.to_string()),
            prefix.map(str::to_string),
            suffix.map(str::to_string),
        )
    }

    #[test]
    fn display_name_joins_the_named_parts() {
        let (base, prefix, suffix) = facts("Aether Cluster", Some("Cruel"), None);
        let view = ItemFacts {
            base: &base,
            prefix: prefix.as_deref(),
            suffix: suffix.as_deref(),
        };
        assert_eq!(view.display_name(), "Cruel Aether Cluster");
        assert_eq!(view.initials(), "AC");
    }

    #[test]
    fn initials_skip_punctuation_and_cap_at_two() {
        let (base, _, _) = facts("<unknown> a01_thing", None, None);
        let view = ItemFacts {
            base: &base,
            prefix: None,
            suffix: None,
        };
        assert_eq!(view.initials(), "A");
        let (base, _, _) = facts("Mark of the Dark Woods", None, None);
        let view = ItemFacts {
            base: &base,
            prefix: None,
            suffix: None,
        };
        assert_eq!(view.initials(), "MO");
    }

    #[test]
    fn an_empty_cache_knows_no_footprints_until_warmed() {
        let cache = FactsCache::default();
        let item = Item {
            base_name: "records/items/x.dbr".into(),
            ..Item::default()
        };
        assert_eq!(cache.footprint(&item), None);
    }
}
