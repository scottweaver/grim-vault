//! Respec on real files. Offline, the vendored fixture (see
//! `fixtures/FIXTURES.md`) is reset under rules carried by a synthetic
//! database — the base game's verified numbers, and mastery trees
//! whose members are the fixture's own `playerclassNN/` skills — and
//! must survive encode → parse with every other block intact. With
//! `GRIMVAULT_GAME_DIR` naming an install and `GRIMVAULT_SAVE_DIR` a
//! **copy** of a save directory (never the live one), the real rules
//! are read from the real records and both resets run over every
//! character; either variable unset skips that test.

#![expect(
    clippy::float_cmp,
    reason = "the pools are exact sums of small whole numbers; any drift is a wrong rule"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use grimvault_core::gamedata::{GameData, shipped_layers};
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::respec::{
    LEVELS_RECORD, MasteryTree, PLAYER_RECORD, PerAttribute, Report, Reset, RespecRules,
};
use univault_engine::arz::fixture::{ArzBuilder, Values};
use univault_engine::arz::{ArzDialect, ArzFile};
use univault_engine::text::TextDb;

const FIXTURE: &[u8] = include_bytes!("fixtures/v11_player.gdc");

/// The mastery folders of a save's skills, keyed by class index.
fn mastery_folders(file: &PlayerFile) -> BTreeMap<u32, Vec<String>> {
    let mut folders: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for skill in &file.skills().unwrap().skills {
        let Some(rest) = skill.name.strip_prefix("records/skills/playerclass") else {
            continue;
        };
        let index: u32 = rest[..2].parse().unwrap();
        folders.entry(index).or_default().push(skill.name.clone());
    }
    folders
}

/// The base game's verified rules with one tree per folder given.
fn synthetic_rules(trees: &BTreeMap<u32, Vec<String>>) -> RespecRules {
    let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
    builder.record(
        LEVELS_RECORD,
        "",
        &[
            ("strengthIncrement", Values::Ints(&[8])),
            ("dexterityIncrement", Values::Ints(&[8])),
            ("intelligenceIncrement", Values::Ints(&[8])),
            ("lifeIncrement", Values::Ints(&[20])),
            ("lifeIncrementDexterity", Values::Ints(&[8])),
            ("lifeIncrementIntelligence", Values::Ints(&[12])),
            ("manaIncrement", Values::Ints(&[16])),
            ("characterModifierPoints", Values::Ints(&[1])),
            ("skillModifierPoints", Values::Ints(&[3, 3, 3])),
            ("maxDevotionPoints", Values::Ints(&[55])),
        ],
    );
    let tree_paths: Vec<(String, String)> = trees
        .keys()
        .map(|index| {
            (
                format!("skillTree{index}"),
                format!("records/skills/playerclass{index:02}/_classtree_class{index:02}.dbr"),
            )
        })
        .collect();
    let tree_refs: Vec<(&str, [&str; 1])> = tree_paths
        .iter()
        .map(|(name, path)| (name.as_str(), [path.as_str()]))
        .collect();
    let mut player: Vec<(&str, Values<'_>)> = vec![
        ("characterStrength", Values::Floats(&[50.0])),
        ("characterDexterity", Values::Floats(&[50.0])),
        ("characterIntelligence", Values::Floats(&[50.0])),
        ("characterLife", Values::Floats(&[250.0])),
        ("characterMana", Values::Floats(&[250.0])),
    ];
    player.extend(
        tree_refs
            .iter()
            .map(|(name, path)| (*name, Values::Strings(path))),
    );
    builder.record(PLAYER_RECORD, "Player", &player);
    for ((_, path), members) in tree_paths.iter().zip(trees.values()) {
        let names: Vec<String> = (1..=members.len())
            .map(|slot| format!("skillName{slot}"))
            .collect();
        let member_refs: Vec<[&str; 1]> = members.iter().map(|m| [m.as_str()]).collect();
        let variables: Vec<(&str, Values<'_>)> = names
            .iter()
            .zip(&member_refs)
            .map(|(name, member)| (name.as_str(), Values::Strings(member)))
            .collect();
        builder.record(path, "SkillTree", &variables);
    }
    let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
    RespecRules::load(&GameData::from_parts(vec![database], TextDb::new(), vec![])).unwrap()
}

/// Applies `reset`, proves the edit survives encode → parse with every
/// block still typed, and that a second application changes nothing;
/// returns the report and the edited model.
fn assert_reset_survives(
    file: &PlayerFile,
    reset: Reset,
    rules: &RespecRules,
    label: &str,
) -> (Report, PlayerFile) {
    let mut edited = file.clone();
    let report = reset
        .apply(&mut edited, rules)
        .unwrap_or_else(|error| panic!("{label}: reset {reset}: {error}"));
    if report.is_noop() {
        assert_eq!(
            &edited, file,
            "{label}: a no-op {reset} reset changed the model"
        );
    } else {
        assert_ne!(
            &edited, file,
            "{label}: {reset} reset reported a change but made none"
        );
    }
    let bytes = edited
        .encode()
        .unwrap_or_else(|error| panic!("{label}: reset {reset}: encode: {error}"));
    let parsed = PlayerFile::parse(&bytes)
        .unwrap_or_else(|error| panic!("{label}: reset {reset}: re-parse: {error}"));
    assert_eq!(parsed, edited, "{label}: reset {reset} did not survive");
    assert_eq!(parsed.blocks().len(), file.blocks().len());
    assert!(parsed.is_fully_typed());

    let mut again = edited.clone();
    let second = reset.apply(&mut again, rules).unwrap();
    assert!(
        second.is_noop(),
        "{label}: a second {reset} reset changed something: {second}"
    );
    assert_eq!(again, edited);
    (report, edited)
}

fn assert_attributes_at_base(file: &PlayerFile, rules: &RespecRules, label: &str) {
    let bio = file.bio().unwrap();
    assert_eq!(
        PerAttribute {
            physique: bio.physique,
            cunning: bio.cunning,
            spirit: bio.spirit
        },
        rules.base.attributes,
        "{label}: attributes"
    );
    assert_eq!(bio.health, rules.base.health, "{label}: health");
    assert_eq!(bio.energy, rules.base.energy, "{label}: energy");
}

fn assert_no_mastery_skill_remains(file: &PlayerFile, trees: &[MasteryTree], label: &str) {
    let remaining: Vec<&str> = file
        .skills()
        .unwrap()
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .filter(|name| trees.iter().any(|tree| tree.contains(name)))
        .collect();
    assert!(
        remaining.is_empty(),
        "{label}: mastery skills remain: {remaining:?}"
    );
    assert_eq!(file.header().class_tag, "", "{label}: class tag");
}

#[test]
fn fixture_attributes_reset_to_base_and_survive_re_encode() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    let rules = synthetic_rules(&mastery_folders(&file));
    let (report, edited) = assert_reset_survives(&file, Reset::Attributes, &rules, "fixture");
    let Report::Attributes(reset) = report else {
        panic!("an attribute reset reports attributes");
    };
    assert_eq!(
        reset.refunded,
        PerAttribute {
            physique: 107,
            cunning: 0,
            spirit: 0
        }
    );
    assert_attributes_at_base(&edited, &rules, "fixture");
    assert_eq!(edited.bio().unwrap().attribute_points_unspent, 108);
    assert_eq!(edited.bio().unwrap().skill_points_unspent, 0);
    assert_eq!(edited.skills(), file.skills());
}

#[test]
fn fixture_masteries_reset_and_survive_re_encode() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    let folders = mastery_folders(&file);
    assert_eq!(folders.keys().copied().collect::<Vec<u32>>(), vec![5, 6]);
    let rules = synthetic_rules(&folders);
    let (report, edited) = assert_reset_survives(&file, Reset::Masteries, &rules, "fixture");
    let Report::Masteries(reset) = report else {
        panic!("a mastery reset reports masteries");
    };
    let removed: Vec<(u32, usize, u32)> = reset
        .masteries
        .iter()
        .map(|m| (m.index.value(), m.skills_removed, m.points_refunded))
        .collect();
    assert_eq!(removed, vec![(5, 9, 68), (6, 18, 180)]);
    assert!(reset.class_tag_cleared);
    assert_eq!(file.header().class_tag, "tagSkillClassName0506");
    assert_no_mastery_skill_remains(&edited, &rules.base.masteries, "fixture");
    let skills = edited.skills().unwrap();
    assert_eq!(
        skills.skills.len(),
        file.skills().unwrap().skills.len() - 27
    );
    assert_eq!(skills.masteries_allowed, 2);
    assert_eq!(skills.skill_reclamation_points_used, 327);
    assert_eq!(edited.bio().unwrap().skill_points_unspent, 248);
    assert_eq!(edited.bio().unwrap().attribute_points_unspent, 1);
    assert_eq!(edited.bio().unwrap().physique, 906.0);
}

fn shipped_game_data(game_dir: &Path) -> GameData {
    let databases: Vec<ArzFile> = shipped_layers()
        .iter()
        .map(|layer| game_dir.join(&layer.database))
        .filter(|path| path.is_file())
        .map(|path| ArzFile::parse(std::fs::read(path).unwrap(), ArzDialect::grim_dawn()).unwrap())
        .collect();
    assert!(!databases.is_empty(), "no database.arz under the game dir");
    GameData::from_parts(databases, TextDb::new(), vec![])
}

fn real_save_paths(save_dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Realm::ALL
        .into_iter()
        .filter_map(|realm| std::fs::read_dir(save_dir.join(realm.dir_name())).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("player.gdc"))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
}

/// The base game's numbers as verified on the user's install
/// (`docs/format-references.md`, "Respec").
fn assert_real_rules(rules: &RespecRules) {
    assert_eq!(
        rules.levels.attribute_increment,
        PerAttribute {
            physique: 8.0,
            cunning: 8.0,
            spirit: 8.0
        }
    );
    assert_eq!(
        rules.levels.health_increment,
        PerAttribute {
            physique: 20.0,
            cunning: 8.0,
            spirit: 12.0
        }
    );
    assert_eq!(rules.levels.energy_increment, 16.0);
    assert_eq!(rules.levels.attribute_points_per_level, 1);
    assert_eq!(rules.levels.skill_points_by_level.first(), Some(&3));
    assert!(rules.levels.skill_points_by_level.len() >= 99);
    assert_eq!(rules.levels.max_devotion_points, 55);
    assert_eq!(
        rules.base.attributes,
        PerAttribute {
            physique: 50.0,
            cunning: 50.0,
            spirit: 50.0
        }
    );
    assert_eq!((rules.base.health, rules.base.energy), (250.0, 250.0));
    let trees = &rules.base.masteries;
    assert!(trees.len() >= 6, "{} mastery trees", trees.len());
    for (slot, tree) in trees.iter().enumerate() {
        let index = u32::try_from(slot + 1).unwrap();
        assert_eq!(tree.index.value(), index);
        assert!(
            tree.record
                .as_str()
                .starts_with(&format!("records/skills/playerclass{index:02}/")),
            "{}",
            tree.record.as_str()
        );
        assert!(
            tree.member_count() >= 30,
            "{}: {}",
            tree.name,
            tree.member_count()
        );
    }
}

#[test]
fn real_rules_reset_every_real_save() {
    let (Some(game_dir), Some(save_dir)) = (
        std::env::var_os("GRIMVAULT_GAME_DIR").map(PathBuf::from),
        std::env::var_os("GRIMVAULT_SAVE_DIR").map(PathBuf::from),
    ) else {
        return;
    };
    let rules = RespecRules::load(&shipped_game_data(&game_dir)).unwrap();
    assert_real_rules(&rules);
    let trees = &rules.base.masteries;

    let mut checked = 0;
    let mut paths = real_save_paths(&save_dir);
    assert!(!paths.is_empty(), "no player.gdc under the save dir");
    paths.push(PathBuf::from("fixture"));
    for path in paths {
        let label = path.display().to_string();
        let bytes = if path == Path::new("fixture") {
            FIXTURE.to_vec()
        } else {
            std::fs::read(&path).unwrap()
        };
        let file = PlayerFile::parse(&bytes).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert!(file.is_fully_typed(), "{label}: an opaque block remains");
        let before = file.bio().unwrap().clone();

        let (report, edited) = assert_reset_survives(&file, Reset::Attributes, &rules, &label);
        let Report::Attributes(reset) = report else {
            panic!("an attribute reset reports attributes");
        };
        assert_attributes_at_base(&edited, &rules, &label);
        assert_eq!(
            edited.bio().unwrap().attribute_points_unspent,
            before.attribute_points_unspent + reset.total(),
            "{label}: attribute pool"
        );

        let (report, edited) = assert_reset_survives(&file, Reset::Masteries, &rules, &label);
        let Report::Masteries(reset) = report else {
            panic!("a mastery reset reports masteries");
        };
        assert_no_mastery_skill_remains(&edited, trees, &label);
        assert_eq!(
            edited.bio().unwrap().skill_points_unspent,
            before.skill_points_unspent + reset.points_refunded(),
            "{label}: skill pool"
        );
        assert_eq!(
            edited.skills().unwrap().skills.len(),
            file.skills().unwrap().skills.len() - reset.skills_removed(),
            "{label}: skill count"
        );
        assert_eq!(
            edited.skills().unwrap().masteries_allowed,
            file.skills().unwrap().masteries_allowed,
            "{label}: masteries_allowed is the level gate and stays"
        );
        checked += 1;
    }
    assert!(checked > 1);
}
