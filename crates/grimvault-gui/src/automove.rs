//! Standing orders on the tabs: the tabs nominated to empty
//! themselves into the vault or to purge what the vault already
//! holds, resolved from the settings to the containers open right
//! now, and the requests a pane reports when the user toggles a
//! nomination. The moves and deletions themselves are
//! `grimvault_core::bulk`; the shell only decides which nominations
//! apply and when.

use grimvault_core::campaign::Campaign;
use grimvault_core::gdc::Realm;
use grimvault_core::settings::{AutoMoveTab, StandingOrder};
use grimvault_core::transfer::TabIndex;

use crate::documents::{CharacterEntry, CharacterSlot, Doc};

/// What a toggle on a tab's header asked for: the tab nominated for
/// an order, or withdrawn from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderRequest {
    Nominate {
        order: StandingOrder,
        tab: AutoMoveTab,
    },
    Withdraw {
        order: StandingOrder,
        tab: AutoMoveTab,
    },
}

/// A nominated tab among the documents open now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoMoveTarget {
    TransferStash(TabIndex),
    CharacterStash {
        character: CharacterSlot,
        tab: TabIndex,
    },
}

impl AutoMoveTarget {
    /// The document the tab lives in.
    #[must_use]
    pub fn doc(self) -> Doc {
        match self {
            Self::TransferStash(_) => Doc::Stash,
            Self::CharacterStash { character, .. } => Doc::Character(character),
        }
    }
}

/// An open character as a nomination names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenName<'a> {
    pub slot: CharacterSlot,
    pub realm: Realm,
    pub name: &'a str,
}

/// The readable characters as a nomination names them, with the
/// slot each occupies.
#[must_use]
pub fn open_names(characters: &[CharacterEntry]) -> Vec<OpenName<'_>> {
    characters
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            entry.doc().map(|doc| OpenName {
                slot: CharacterSlot::new(slot),
                realm: doc.realm(),
                name: doc.name(),
            })
        })
        .collect()
}

/// The target a nomination names among the open documents: a
/// transfer-stash tab only under its own campaign, a character's tab
/// only while that character — by realm and name, since two realms
/// may each hold one of the same name — is open and readable.
#[must_use]
pub fn resolve(
    nomination: &AutoMoveTab,
    campaign: &Campaign,
    characters: &[OpenName<'_>],
) -> Option<AutoMoveTarget> {
    match nomination {
        AutoMoveTab::TransferStash {
            campaign: nominated,
            tab,
        } => (nominated == campaign).then_some(AutoMoveTarget::TransferStash(*tab)),
        AutoMoveTab::CharacterStash { realm, name, tab } => characters
            .iter()
            .find(|open| open.realm == *realm && open.name == name)
            .map(|open| AutoMoveTarget::CharacterStash {
                character: open.slot,
                tab: *tab,
            }),
    }
}

/// Every nomination's target among the open documents, in
/// nomination order; nominations naming nothing open are skipped.
#[must_use]
pub fn targets(
    nominations: &[AutoMoveTab],
    campaign: &Campaign,
    characters: &[OpenName<'_>],
) -> Vec<AutoMoveTarget> {
    nominations
        .iter()
        .filter_map(|nomination| resolve(nomination, campaign, characters))
        .collect()
}

#[cfg(test)]
mod tests {
    use grimvault_core::campaign::ModName;

    use super::*;

    fn loot() -> Campaign {
        Campaign::Mod(ModName::parse("LootAscension").unwrap())
    }

    fn open<'a>() -> Vec<OpenName<'a>> {
        vec![
            OpenName {
                slot: CharacterSlot::new(0),
                realm: Realm::Main,
                name: "Zark",
            },
            OpenName {
                slot: CharacterSlot::new(2),
                realm: Realm::Custom,
                name: "Zark",
            },
        ]
    }

    #[test]
    fn a_transfer_stash_nomination_resolves_only_under_its_own_campaign() {
        let nomination = AutoMoveTab::TransferStash {
            campaign: loot(),
            tab: TabIndex::new(3),
        };
        assert_eq!(
            resolve(&nomination, &loot(), &open()),
            Some(AutoMoveTarget::TransferStash(TabIndex::new(3)))
        );
        assert_eq!(resolve(&nomination, &Campaign::Main, &open()), None);
    }

    #[test]
    fn a_character_nomination_resolves_by_realm_and_name() {
        let custom = AutoMoveTab::CharacterStash {
            realm: Realm::Custom,
            name: "Zark".into(),
            tab: TabIndex::new(1),
        };
        assert_eq!(
            resolve(&custom, &Campaign::Main, &open()),
            Some(AutoMoveTarget::CharacterStash {
                character: CharacterSlot::new(2),
                tab: TabIndex::new(1),
            })
        );
        let absent = AutoMoveTab::CharacterStash {
            realm: Realm::Main,
            name: "Sif".into(),
            tab: TabIndex::new(0),
        };
        assert_eq!(resolve(&absent, &Campaign::Main, &open()), None);
    }

    #[test]
    fn targets_keep_nomination_order_and_drop_what_is_not_open() {
        let nominations = vec![
            AutoMoveTab::CharacterStash {
                realm: Realm::Main,
                name: "Zark".into(),
                tab: TabIndex::new(0),
            },
            AutoMoveTab::TransferStash {
                campaign: loot(),
                tab: TabIndex::new(9),
            },
            AutoMoveTab::TransferStash {
                campaign: Campaign::Main,
                tab: TabIndex::new(0),
            },
        ];
        let found = targets(&nominations, &Campaign::Main, &open());
        assert_eq!(
            found,
            vec![
                AutoMoveTarget::CharacterStash {
                    character: CharacterSlot::new(0),
                    tab: TabIndex::new(0),
                },
                AutoMoveTarget::TransferStash(TabIndex::new(0)),
            ]
        );
        assert_eq!(found[0].doc(), Doc::Character(CharacterSlot::new(0)));
        assert_eq!(found[1].doc(), Doc::Stash);
    }
}
