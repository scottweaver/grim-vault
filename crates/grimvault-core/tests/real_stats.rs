//! Stat lines rendered from the real installed database, pinned to the
//! exact text the game's own `tags_ui.txt` templates produce for
//! hand-picked records — one of each shape the renderer composes. Runs
//! against `$GRIMVAULT_GAME_DIR` (the game install; read in place, it
//! is never written) and passes vacuously when the variable is unset.

use std::fs;
use std::path::{Path, PathBuf};

use grimvault_core::gamedata::{GameData, LayerFiles, LayerSet, shipped_layers};
use grimvault_core::item::Item;
use grimvault_core::stats::{self, BlockSource, Emphasis, Requirement, Scale, Section, SkillLevel};
use univault_engine::arc::ArcFile;
use univault_engine::arz::{ArzDialect, ArzFile};
use univault_engine::codec::Codec;
use univault_engine::ids::RecordId;

const SPINECARVER: &str = "records/items/gearweapons/swords1h/c012_sword.dbr";
const VESTMENTS: &str = "records/items/geartorso/b301c_torso.dbr";
const MAW: &str = "records/items/gearweapons/shields/d104_shield.dbr";
const PHYS_RETALIATION: &str = "records/items/lootaffixes/completion/a24a_physretaliation.dbr";
const HYSTERIA: &str = "records/items/gearrelic/b105_relic.dbr";
const MINDWARP: &str = "records/items/gearweapons/swords1h/d004_sword.dbr";
const DOMINION: &str = "records/items/gearaccessories/necklaces/b303d_necklace.dbr";
const GOREWAKE: &str = "records/items/lootaffixes/prefix/b_sh304_d.dbr";
const COWL: &str = "records/items/gearhead/c009_head.dbr";
const SEAL_OF_NIGHT: &str = "records/items/materia/compa_sealnight.dbr";

fn game() -> Option<GameData> {
    let dir = std::env::var_os("GRIMVAULT_GAME_DIR").map(PathBuf::from)?;
    dir.join("database/database.arz")
        .is_file()
        .then(|| load(&dir))
}

fn load(game_dir: &Path) -> GameData {
    let mut shipped = LayerSet::default();
    for LayerFiles {
        database,
        text,
        items,
        ui: _,
        resources: _,
    } in shipped_layers()
    {
        if let Ok(bytes) = fs::read(game_dir.join(database)) {
            shipped
                .databases
                .push(ArzFile::parse(bytes, ArzDialect::grim_dawn()).unwrap());
        }
        if let Ok(bytes) = fs::read(game_dir.join(text)) {
            shipped
                .text_archives
                .push(ArcFile::parse(bytes, Codec::Lz4Block).unwrap());
        }
        if let Ok(bytes) = fs::read(game_dir.join(items)) {
            shipped
                .item_archives
                .push(ArcFile::parse(bytes, Codec::Lz4Block).unwrap());
        }
    }
    GameData::layered(shipped, LayerSet::default()).unwrap()
}

fn record(raw: &str) -> RecordId {
    RecordId::parse(raw.to_string()).unwrap()
}

fn texts(game: &GameData, id: &str, scale: Scale) -> Vec<String> {
    game.record_stats(&record(id), SkillLevel::ONE, scale)
        .unwrap_or_else(|| panic!("{id} is in the database"))
        .lines
        .iter()
        .map(|line| line.text.clone())
        .collect()
}

fn assert_renders(game: &GameData, id: &str, expected: &[&str]) {
    let lines = texts(game, id, Scale::NONE);
    for line in expected {
        assert!(
            lines.contains(&(*line).to_string()),
            "{line:?} in {lines:?}"
        );
    }
}

fn bare(id: &str) -> Item {
    Item {
        base_name: id.to_string(),
        ..Item::default()
    }
}

#[test]
fn a_weapon_renders_base_damage_speed_modifiers_and_skill_bonuses() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        SPINECARVER,
        &[
            "89-97 Physical Damage",
            "Speed: Very Fast",
            "+65% Physical Damage",
            "9-20 Physical Damage",
            "25.0% Chance of 150 Bleeding Damage over 3.0 Seconds",
            "+75% Bleeding Damage",
            "+8% Attack Speed",
            "+2 to Fighting Form",
        ],
    );
}

#[test]
fn armor_renders_its_armor_resistances_regen_and_conversion() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        VESTMENTS,
        &[
            "520 Armor",
            "5% Physical Resistance",
            "+400 Health",
            "+5.5 Energy Regenerated per second",
            "30% Lightning Damage converted to Cold Damage",
            "+24% Cold Damage",
        ],
    );
}

#[test]
fn a_shield_renders_block_leech_duration_modifier_and_skill_modifiers() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        MAW,
        &[
            "1128 Damage Blocked",
            "30% Chance to Block",
            "0.85 second Block Recovery",
            "144 Vitality Damage",
            "8% of Attack Damage converted to Health",
            "+70% Bleeding Damage with +100% Increased Duration",
            "+1 to all skills in Occultist",
            "Grants Skill: Hungering Maw",
            "10% Damage Absorption to Mark of Torment",
            "180 Bleeding Damage over 1.0 Seconds to Siphon Souls",
        ],
    );
}

#[test]
fn a_completion_bonus_renders_retaliation_with_its_chance() {
    let Some(game) = game() else {
        return;
    };
    assert_eq!(
        texts(&game, PHYS_RETALIATION, Scale::NONE),
        vec!["15.0% Chance of 100 Physical Damage Retaliation"]
    );
}

#[test]
fn a_relic_renders_its_granted_summon_and_pet_bonus() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        HYSTERIA,
        &[
            "+100 Health",
            "+20 Defensive Ability",
            "Grants Skill: Summon Crab Spirit",
            "180 Energy Cost",
            "30.0 Second Skill Recharge",
            "1 Summon Limit",
            "Bonus to All Pets",
            "+30% to All Damage",
        ],
    );
}

#[test]
fn an_autocast_skill_and_a_resistance_reduction_render() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        MINDWARP,
        &[
            "20% Armor Piercing",
            "20.0% Chance of 30% Reduced target's Resistances for 5.0 Seconds",
            "Grants Skill: Mindwarp (100% Chance on Attack)",
            "3.0 Second Duration",
            "50% Piercing Damage converted to Aether Damage",
        ],
    );
}

#[test]
fn racial_and_all_skills_bonuses_render() {
    let Some(game) = game() else {
        return;
    };
    assert_renders(
        &game,
        DOMINION,
        &[
            "8% Less Damage from Eldritch",
            "+1 to all Skills",
            "41% Chaos Resistance",
        ],
    );
}

#[test]
fn item_scale_multiplies_an_affix_but_not_the_base() {
    let Some(game) = game() else {
        return;
    };
    let plain = texts(&game, GOREWAKE, Scale::NONE);
    let scaled = texts(&game, GOREWAKE, Scale::percent(30));
    assert!(
        plain.contains(&"+36% Bleeding Damage".to_string()),
        "{plain:?}"
    );
    assert!(
        scaled.contains(&"+46% Bleeding Damage".to_string()),
        "{scaled:?}"
    );
    assert!(
        scaled.contains(&"30% Chaos Resistance".to_string()),
        "resistances never scale: {scaled:?}"
    );

    let details = stats::item_details(&game, &bare(SPINECARVER));
    let base = &details.blocks[0];
    assert_eq!(base.source, BlockSource::Base);
    assert!(
        base.lines
            .iter()
            .any(|line| line.text == "+75% Bleeding Damage"),
        "the base item's own values stay as authored: {:?}",
        base.lines
    );
}

#[test]
fn base_lines_come_first_and_read_white() {
    let Some(game) = game() else {
        return;
    };
    let stats = game
        .record_stats(&record(SPINECARVER), SkillLevel::ONE, Scale::NONE)
        .unwrap();
    let first = &stats.lines[0];
    assert_eq!(first.section, Section::Base);
    assert_eq!(first.emphasis, Emphasis::Base);
    assert_eq!(first.values, vec![89.0, 97.0]);
    assert!(stats.unrendered.is_empty(), "{:?}", stats.unrendered);
}

#[test]
fn requirements_and_set_come_from_the_records() {
    let Some(game) = game() else {
        return;
    };
    let cowl = bare(COWL);
    let details = stats::item_details(&game, &cowl);
    assert_eq!(
        details.requirements,
        vec![(Requirement::Level, 20), (Requirement::Physique, 147)]
    );
    assert_eq!(
        details
            .requirement_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Required Player Level: 20", "Required Physique: 147"]
    );
    assert_eq!(
        stats::set_name(&game, &cowl).as_deref(),
        Some("Bloodreaper's Glory")
    );
    let set = details.set.expect("the cowl belongs to a set");
    assert_eq!(set.members.len(), 5);
    assert_eq!(set.tiers[0].pieces, 2);
    assert_eq!(set.tiers[0].lines[0].text, "+50% Bleeding Damage");
    assert!(set.tiers.iter().any(|tier| {
        tier.pieces == 5
            && tier
                .lines
                .iter()
                .any(|line| line.text == "+1 to all skills in Nightblade")
    }));

    let sword = stats::requirements(&game, &bare(SPINECARVER));
    assert_eq!(
        sword,
        vec![(Requirement::Level, 65), (Requirement::Cunning, 422)]
    );
}

#[test]
fn a_component_socketed_in_a_weapon_gets_its_own_block() {
    let Some(game) = game() else {
        return;
    };
    let item = Item {
        base_name: SPINECARVER.to_string(),
        relic_name: SEAL_OF_NIGHT.to_string(),
        ..Item::default()
    };
    let details = stats::item_details(&game, &item);
    let component = details
        .blocks
        .iter()
        .find(|block| block.source == BlockSource::Component)
        .expect("component block");
    assert_eq!(component.title.as_deref(), Some("Seal of the Night"));
    assert!(
        component
            .lines
            .iter()
            .any(|line| line.text == "+44 Offensive Ability")
    );
    assert!(
        component
            .lines
            .iter()
            .any(|line| line.text == "10% Physical Damage converted to Cold Damage")
    );
    assert!(details.requirements.contains(&(Requirement::Level, 75)));
}
