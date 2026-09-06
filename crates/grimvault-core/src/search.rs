//! The item query: a name fragment and facet constraints, answered
//! per item with a three-way [`Verdict`] so an item whose facets could
//! not be resolved is reported as such rather than silently dropped
//! as a non-match. A shell owns the [`Query`] and asks; the store pane
//! is the first caller, and the query is meant to grow (rarity,
//! bucket, stats) without changing its callers.

use univault_engine::ids::RecordId;

use crate::facets::{Ascension, AscensionTable, DoubleRare, Facets, MonsterInfrequent};
use crate::gamedata::GameData;
use crate::item::Item;

/// Whether a facet must hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Constraint {
    #[default]
    Any,
    Required,
}

impl Constraint {
    /// Flips between the two states, for a toggle.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Any => Self::Required,
            Self::Required => Self::Any,
        }
    }

    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Which ascension standing, if any, an item must have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AscensionFilter {
    #[default]
    Any,
    /// Eligible for an ascendant affix and without one yet.
    Upgradeable,
    /// Carrying an ascendant affix.
    Ascended,
}

/// The whole query. `name` matches case-insensitively as a substring
/// of the displayed name; blank means any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub name: String,
    pub monster_infrequent: Constraint,
    pub double_rare: Constraint,
    pub ascension: AscensionFilter,
}

/// How one item answers a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Matches,
    Excluded,
    /// A required facet could not be resolved for the item.
    Unresolved,
}

/// What a query is asked about: the item's displayed name and its
/// facets, as a shell has them memoized.
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    pub name: &'a str,
    pub facets: Facets,
}

/// One constraint's answer, folded into the item's verdict.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Holds,
    Fails,
    Unknown,
}

impl Query {
    /// Whether the query constrains nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.trim().is_empty()
            && self.monster_infrequent == Constraint::Any
            && self.double_rare == Constraint::Any
            && self.ascension == AscensionFilter::Any
    }

    /// Answers for a subject the shell has already resolved.
    #[must_use]
    pub fn verdict(&self, subject: Subject<'_>) -> Verdict {
        let answers = [
            self.name_answer(subject.name),
            required(self.monster_infrequent, || {
                match subject.facets.monster_infrequent {
                    MonsterInfrequent::Yes => Answer::Holds,
                    MonsterInfrequent::No => Answer::Fails,
                    MonsterInfrequent::Unresolved => Answer::Unknown,
                }
            }),
            required(self.double_rare, || match subject.facets.double_rare {
                DoubleRare::Yes => Answer::Holds,
                DoubleRare::No => Answer::Fails,
                DoubleRare::Unresolved => Answer::Unknown,
            }),
            self.ascension_answer(subject.facets.ascension),
        ];
        if answers.contains(&Answer::Fails) {
            Verdict::Excluded
        } else if answers.contains(&Answer::Unknown) {
            Verdict::Unresolved
        } else {
            Verdict::Matches
        }
    }

    /// Answers for an item straight from the database.
    #[must_use]
    pub fn verdict_of(&self, game: &GameData, table: &AscensionTable, item: &Item) -> Verdict {
        let name = RecordId::parse(item.base_name.clone())
            .and_then(|base| {
                game.display_name(
                    &base,
                    RecordId::parse(item.prefix_name.clone()).as_ref(),
                    RecordId::parse(item.suffix_name.clone()).as_ref(),
                )
            })
            .unwrap_or_else(|| item.base_name.clone());
        self.verdict(Subject {
            name: &name,
            facets: Facets::of(game, table, item),
        })
    }

    fn name_answer(&self, name: &str) -> Answer {
        let needle = self.name.trim().to_lowercase();
        if needle.is_empty() || name.to_lowercase().contains(&needle) {
            Answer::Holds
        } else {
            Answer::Fails
        }
    }

    /// An ascended item is known from its own fields, so the
    /// `Ascended` filter never leaves an item unresolved; only
    /// eligibility depends on records that may be missing.
    fn ascension_answer(&self, ascension: Ascension) -> Answer {
        match self.ascension {
            AscensionFilter::Any => Answer::Holds,
            AscensionFilter::Ascended => {
                if ascension == Ascension::Ascended {
                    Answer::Holds
                } else {
                    Answer::Fails
                }
            }
            AscensionFilter::Upgradeable => match ascension {
                Ascension::Eligible => Answer::Holds,
                Ascension::Ascended | Ascension::Ineligible => Answer::Fails,
                Ascension::Unresolved => Answer::Unknown,
            },
        }
    }
}

fn required(constraint: Constraint, facet: impl FnOnce() -> Answer) -> Answer {
    match constraint {
        Constraint::Any => Answer::Holds,
        Constraint::Required => facet(),
    }
}

#[cfg(test)]
mod tests {
    use crate::facets::{AffixEvidence, BaseEvidence};

    use super::*;

    fn facets(
        monster_infrequent: MonsterInfrequent,
        double_rare: DoubleRare,
        ascension: Ascension,
    ) -> Facets {
        let mut facets = Facets::classify(
            &Item::default(),
            BaseEvidence::Unresolved,
            AffixEvidence::Absent,
            AffixEvidence::Absent,
            &AscensionTable::Absent,
        );
        facets.monster_infrequent = monster_infrequent;
        facets.double_rare = double_rare;
        facets.ascension = ascension;
        facets
    }

    fn resolved(mi: bool, dr: bool, ascension: Ascension) -> Facets {
        facets(
            if mi {
                MonsterInfrequent::Yes
            } else {
                MonsterInfrequent::No
            },
            if dr { DoubleRare::Yes } else { DoubleRare::No },
            ascension,
        )
    }

    #[test]
    fn an_empty_query_matches_everything_resolved_or_not() {
        let query = Query::default();
        assert!(query.is_empty());
        let unknown = facets(
            MonsterInfrequent::Unresolved,
            DoubleRare::Unresolved,
            Ascension::Unresolved,
        );
        assert_eq!(
            query.verdict(Subject {
                name: "anything",
                facets: unknown
            }),
            Verdict::Matches
        );
    }

    #[test]
    fn the_name_matches_as_a_case_insensitive_substring() {
        let query = Query {
            name: "  WENDIGO ".into(),
            ..Query::default()
        };
        assert!(!query.is_empty());
        let plain = resolved(false, false, Ascension::Ineligible);
        assert_eq!(
            query.verdict(Subject {
                name: "Cruel Wendigo Claw",
                facets: plain
            }),
            Verdict::Matches
        );
        assert_eq!(
            query.verdict(Subject {
                name: "Wendig Claw",
                facets: plain
            }),
            Verdict::Excluded
        );
    }

    #[test]
    fn required_facets_exclude_known_misses_and_flag_unresolved_ones() {
        let query = Query {
            monster_infrequent: Constraint::Required,
            double_rare: Constraint::Required,
            ..Query::default()
        };
        let verdict = |facets: Facets| query.verdict(Subject { name: "x", facets });
        assert_eq!(
            verdict(resolved(true, true, Ascension::Ineligible)),
            Verdict::Matches
        );
        assert_eq!(
            verdict(resolved(true, false, Ascension::Ineligible)),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::Yes,
                Ascension::Ineligible
            )),
            Verdict::Unresolved
        );
        assert_eq!(
            verdict(facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::No,
                Ascension::Ineligible
            )),
            Verdict::Excluded
        );
    }

    #[test]
    fn the_ascension_filter_never_leaves_an_ascended_item_unresolved() {
        let upgradeable = Query {
            ascension: AscensionFilter::Upgradeable,
            ..Query::default()
        };
        let ascended = Query {
            ascension: AscensionFilter::Ascended,
            ..Query::default()
        };
        let verdict = |query: &Query, ascension: Ascension| {
            query.verdict(Subject {
                name: "x",
                facets: resolved(false, false, ascension),
            })
        };
        assert_eq!(verdict(&upgradeable, Ascension::Eligible), Verdict::Matches);
        assert_eq!(
            verdict(&upgradeable, Ascension::Ascended),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(&upgradeable, Ascension::Unresolved),
            Verdict::Unresolved
        );
        assert_eq!(verdict(&ascended, Ascension::Ascended), Verdict::Matches);
        assert_eq!(verdict(&ascended, Ascension::Eligible), Verdict::Excluded);
        assert_eq!(verdict(&ascended, Ascension::Unresolved), Verdict::Excluded);
    }

    #[test]
    fn a_constraint_toggles() {
        assert_eq!(Constraint::Any.toggled(), Constraint::Required);
        assert_eq!(Constraint::Required.toggled(), Constraint::Any);
        assert!(Constraint::Required.is_required());
    }
}
