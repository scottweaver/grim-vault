//! What the game data says about an item, resolved once and memoized.
//! The panes ask per painted tile every frame, and the layered lookups
//! behind the answer — record → tag → text, bitmap → archive entry →
//! `.tex` header — are far too slow to repeat at that rate. The facets
//! are derived on each ask from the memoized base and affix records;
//! that is a few comparisons.

use std::collections::HashMap;

use grimvault_core::bucket::Bucket;
use grimvault_core::bulk::{Identities, Identity};
use grimvault_core::facets::{AffixEvidence, AscensionTable, BaseEvidence, Facets};
use grimvault_core::gamedata::{AffixInfo, BitmapPath, Footprint, GameData, ItemClass, Rarity};
use grimvault_core::item::Item;
use grimvault_core::reagents::{ReagentKind, ReagentKinds};
use grimvault_core::search::{AffixName, Subject};
use grimvault_core::socket::Part;
use grimvault_core::stats::ItemDetails;
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
    pub reagent: Option<ReagentKind>,
    pub evidence: BaseEvidence,
    pub identity: Option<Identity>,
}

/// One item's facts: its base record's, the affix names the database
/// could resolve, and the facets derived from all three records.
#[derive(Clone, Copy, Debug)]
pub struct ItemFacts<'a> {
    pub base: &'a BaseFacts,
    pub prefix: Option<&'a str>,
    pub suffix: Option<&'a str>,
    pub facets: Facets,
}

impl<'a> ItemFacts<'a> {
    /// `Prefix Base Suffix`, parts the database cannot name omitted.
    #[must_use]
    pub fn display_name(&self) -> String {
        [self.prefix, Some(self.base.name.as_str()), self.suffix]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What a query asks about, from these facts; `name` is the
    /// caller's [`Self::display_name`] and `details` the stat body when
    /// the query needs one.
    #[must_use]
    pub fn subject<'s>(
        &self,
        item: &'s Item,
        name: &'s str,
        details: Option<&'s ItemDetails>,
    ) -> Subject<'s>
    where
        'a: 's,
    {
        Subject {
            item,
            name,
            prefix: AffixName::of(&item.prefix_name, self.prefix),
            suffix: AffixName::of(&item.suffix_name, self.suffix),
            base: self.base.evidence,
            facets: self.facets,
            details,
        }
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

/// The memo: base facts by base record path, affix records by affix
/// record path, the part a record is by record path, and the
/// ascension table read once. Only the [`Footprints`] and
/// [`ReagentKinds`] views read it without warming, so a caller that
/// will move items warms every item involved first
/// ([`FactsCache::warm`]).
#[derive(Default)]
pub struct FactsCache {
    bases: HashMap<String, BaseFacts>,
    affixes: HashMap<String, Option<AffixInfo>>,
    parts: HashMap<String, Option<Part>>,
    ascension: Option<AscensionTable>,
}

impl FactsCache {
    /// The part a record is — a component or an augment, with the
    /// slots it admits — memoized; `None` when the database does not
    /// know the record or it is no part.
    pub fn part(&mut self, game: &GameData, record: &str) -> Option<&Part> {
        if !self.parts.contains_key(record) {
            let part =
                RecordId::parse(record.to_string()).and_then(|id| Part::read(game, &id).ok());
            self.parts.insert(record.to_string(), part);
        }
        self.parts.get(record).and_then(Option::as_ref)
    }

    /// The facts for `item`, resolving and memoizing on first sight.
    pub fn facts(&mut self, game: &GameData, item: &Item) -> ItemFacts<'_> {
        self.warm(game, item);
        let base = self
            .bases
            .get(&item.base_name)
            .unwrap_or_else(|| unreachable!("warm inserted the base"));
        let table = self
            .ascension
            .as_ref()
            .unwrap_or_else(|| unreachable!("warm read the table"));
        let affix = |name: &str| self.affixes.get(name).and_then(Option::as_ref);
        let prefix = affix(&item.prefix_name);
        let suffix = affix(&item.suffix_name);
        let facets = Facets::classify(
            item,
            base.evidence,
            AffixEvidence::of(&item.prefix_name, prefix),
            AffixEvidence::of(&item.suffix_name, suffix),
            table,
        );
        ItemFacts {
            base,
            prefix: prefix.and_then(|info| info.name.as_deref()),
            suffix: suffix.and_then(|info| info.name.as_deref()),
            facets,
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
        if self.ascension.is_none() {
            self.ascension = Some(AscensionTable::read(game));
        }
        if !self.bases.contains_key(&item.base_name) {
            self.bases
                .insert(item.base_name.clone(), resolve_base(game, &item.base_name));
        }
        for affix in [&item.prefix_name, &item.suffix_name] {
            if !affix.is_empty() && !self.affixes.contains_key(affix) {
                let info = RecordId::parse(affix.clone())
                    .and_then(|id| game.affix_info(&id))
                    .and_then(Result::ok);
                self.affixes.insert(affix.clone(), info);
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

impl ReagentKinds for FactsCache {
    fn reagent_kind(&self, item: &Item) -> Option<ReagentKind> {
        self.bases.get(&item.base_name)?.reagent
    }
}

impl Identities for FactsCache {
    fn identity(&self, item: &Item) -> Option<Identity> {
        self.bases.get(&item.base_name)?.identity
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
    let evidence = BaseEvidence::of(Some(&info));
    BaseFacts {
        name: info.name,
        record: RecordStatus::Known,
        rarity: info.rarity,
        class: info.class,
        level_requirement: info.level_requirement,
        footprint,
        bitmap: info.bitmap,
        bucket,
        reagent: info.reagent,
        evidence,
        identity: Some(Identity::of(bucket, info.max_stack_size)),
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
        reagent: None,
        evidence: BaseEvidence::Unresolved,
        identity: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(
        base: &'a BaseFacts,
        prefix: Option<&'a str>,
        suffix: Option<&'a str>,
    ) -> ItemFacts<'a> {
        ItemFacts {
            base,
            prefix,
            suffix,
            facets: Facets::classify(
                &Item::default(),
                base.evidence,
                AffixEvidence::Absent,
                AffixEvidence::Absent,
                &AscensionTable::Absent,
            ),
        }
    }

    #[test]
    fn display_name_joins_the_named_parts() {
        let base = unknown("Aether Cluster".to_string());
        let view = view(&base, Some("Cruel"), None);
        assert_eq!(view.display_name(), "Cruel Aether Cluster");
        assert_eq!(view.initials(), "AC");
    }

    #[test]
    fn initials_skip_punctuation_and_cap_at_two() {
        let base = unknown("<unknown> a01_thing".to_string());
        assert_eq!(view(&base, None, None).initials(), "A");
        let base = unknown("Mark of the Dark Woods".to_string());
        assert_eq!(view(&base, None, None).initials(), "MO");
    }

    #[test]
    fn an_empty_cache_knows_no_footprints_or_kinds_until_warmed() {
        let cache = FactsCache::default();
        let item = Item {
            base_name: "records/items/x.dbr".into(),
            ..Item::default()
        };
        assert_eq!(cache.footprint(&item), None);
        assert_eq!(cache.reagent_kind(&item), None);
    }

    #[test]
    fn an_unknown_base_leaves_every_facet_unresolved() {
        let base = unknown("<unknown> x".to_string());
        let facets = view(&base, None, None).facets;
        assert!(!facets.is_resolved());
        assert_eq!(facets.symbol(), None);
    }
}
