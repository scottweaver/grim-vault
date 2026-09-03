// Vendored from tq-univault crates/univault-gui/src/sort.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Sort direction — the one flip every sortable surface shares, so
//! "ascending" reads the same way in a store pane's bucket grid as it
//! does in a search table.

use std::cmp::Ordering;

/// Which way a sorted view reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    /// Orients a comparison that was computed ascending. Every
    /// sortable surface builds its comparison one way and hands it
    /// here, so a key's ranking never encodes its own direction.
    #[must_use]
    pub fn apply(self, ordering: Ordering) -> Ordering {
        match self {
            Self::Ascending => ordering,
            Self::Descending => ordering.reverse(),
        }
    }

    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    #[must_use]
    pub fn arrow(self) -> &'static str {
        match self {
            Self::Ascending => "▲",
            Self::Descending => "▼",
        }
    }

    /// The toggle shows the direction in force, so its hint has to
    /// name the one a click would switch to.
    #[must_use]
    pub fn flip_hint(self) -> &'static str {
        match self {
            Self::Ascending => "Ascending — click for descending",
            Self::Descending => "Descending — click for ascending",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descending_reverses_what_ascending_leaves_alone() {
        assert_eq!(
            SortDirection::Ascending.apply(Ordering::Less),
            Ordering::Less
        );
        assert_eq!(
            SortDirection::Descending.apply(Ordering::Less),
            Ordering::Greater
        );
        assert_eq!(
            SortDirection::Descending.apply(Ordering::Equal),
            Ordering::Equal
        );
    }

    #[test]
    fn flipping_twice_is_the_identity() {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            assert_eq!(direction.flipped().flipped(), direction);
            assert_ne!(direction.flipped(), direction);
        }
    }
}
