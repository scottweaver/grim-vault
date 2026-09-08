//! Which documents an unsaved edit binds together. A move, a socket
//! fill, a bulk move or an auto-move edits two documents in one
//! gesture — the item leaves one file and lands in another — but the
//! external-change guard decides per document. Left alone, "reload
//! from disk" on the changed document discards its half and lets the
//! partner's half be written, and the item is gone from both places
//! (the user's Black Tallow, 2026-09-08). A link records the pairing
//! while either side is unsaved, so the decision reaches both ends;
//! it settles when a side is written or reloaded, since the halves are
//! then no longer a pending pair.

use crate::documents::Doc;

/// Unordered pairs of documents with a pending two-document edit
/// between them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links(Vec<(Doc, Doc)>);

impl Links {
    /// Binds `a` and `b`; a document is never linked to itself, and a
    /// pair is held once however many edits made it.
    pub fn link(&mut self, a: Doc, b: Doc) {
        if a != b && !self.joins(a, b) {
            self.0.push((a, b));
        }
    }

    /// `doc`'s unsaved edits are gone — written, or discarded by a
    /// reload — so no pending pair reaches through it any more.
    pub fn settled(&mut self, doc: Doc) {
        self.0.retain(|(a, b)| *a != doc && *b != doc);
    }

    /// Every document a pending edit binds to one of `docs`, directly
    /// or through further links, without `docs` themselves; in the
    /// order they are reached.
    #[must_use]
    pub fn bound_to(&self, docs: &[Doc]) -> Vec<Doc> {
        let mut reached: Vec<Doc> = docs.to_vec();
        let mut bound = Vec::new();
        let mut index = 0;
        while index < reached.len() {
            let current = reached[index];
            index += 1;
            for &(a, b) in &self.0 {
                let other = match (a == current, b == current) {
                    (true, _) => b,
                    (_, true) => a,
                    (false, false) => continue,
                };
                if !reached.contains(&other) {
                    reached.push(other);
                    bound.push(other);
                }
            }
        }
        bound
    }

    fn joins(&self, a: Doc, b: Doc) -> bool {
        self.0
            .iter()
            .any(|&(x, y)| (x == a && y == b) || (x == b && y == a))
    }
}

#[cfg(test)]
mod tests {
    use crate::documents::CharacterSlot;

    use super::*;

    const ZARK: Doc = Doc::Character(CharacterSlot::new(1));
    const SYF: Doc = Doc::Character(CharacterSlot::new(2));

    #[test]
    fn a_pair_is_held_once_and_a_document_never_binds_itself() {
        let mut links = Links::default();
        links.link(Doc::Store, Doc::Stash);
        links.link(Doc::Stash, Doc::Store);
        links.link(Doc::Store, Doc::Store);
        assert_eq!(links, Links(vec![(Doc::Store, Doc::Stash)]));
    }

    #[test]
    fn a_conflict_reaches_every_document_bound_to_it() {
        let mut links = Links::default();
        links.link(ZARK, Doc::Store);
        links.link(Doc::Store, Doc::Stash);
        links.link(SYF, Doc::Reagents);
        assert_eq!(links.bound_to(&[ZARK]), vec![Doc::Store, Doc::Stash]);
        assert_eq!(links.bound_to(&[Doc::Stash]), vec![Doc::Store, ZARK]);
        assert_eq!(links.bound_to(&[SYF]), vec![Doc::Reagents]);
        assert_eq!(links.bound_to(&[Doc::Blueprints]), Vec::<Doc>::new());
        assert_eq!(
            links.bound_to(&[ZARK, Doc::Store]),
            vec![Doc::Stash],
            "the documents asked about are not repeated"
        );
    }

    #[test]
    fn a_written_or_reloaded_document_settles_its_links() {
        let mut links = Links::default();
        links.link(ZARK, Doc::Store);
        links.link(Doc::Store, Doc::Stash);
        links.settled(Doc::Store);
        assert_eq!(links, Links::default());
        links.link(ZARK, Doc::Store);
        links.link(SYF, Doc::Store);
        links.settled(ZARK);
        assert_eq!(links.bound_to(&[Doc::Store]), vec![SYF]);
    }
}
