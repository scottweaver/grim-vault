//! The MCP tool surface. The record database loads once, on first
//! use; every tool that looks at a save or the vault store re-reads
//! it on call; none of them writes anything — the read-only
//! constraint recorded in ARCHITECTURE.md "Planned surfaces".

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use grimvault_core::blueprint::available_blueprints;
use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::campaign::Campaign;
use grimvault_core::facets::AscensionTable;
use grimvault_core::formulas::FormulaRead;
use grimvault_core::gamedata::{GameData, Rarity};
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::reference::{AffixQuery, AffixTable, Coverage, Position};
use grimvault_core::respec::{RespecRules, RulesError};
use grimvault_core::search::{
    AscensionFilter, CategoryFilter, Constraint, Criterion, Query, RequirementCaps, Resolved,
    SetFilter, SocketFilter, ValueBounds, Verdict,
};
use grimvault_core::stats::item::set_info;
use grimvault_core::store::VaultStore;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use univault_engine::ids::{RecordId, normalize};

use crate::build;
use crate::devotion::{self, Constellations};
use crate::gamedb::{self, IndexEntry, Needle};
use crate::skills;
use crate::view::{self, item_json};
use crate::world::{self, CharacterFile, Loaded, Paths, PathsError};

/// The loaded record database with what the tools derive from it
/// once: the mastery trees, the ascension table, and — on first use
/// — the affix reference and the record index.
pub struct Game {
    loaded: Loaded,
    rules: Result<RespecRules, RulesError>,
    ascension: AscensionTable,
    affixes: OnceLock<AffixTable>,
    index: OnceLock<Vec<IndexEntry>>,
    constellations: OnceLock<Constellations>,
}

impl Game {
    fn data(&self) -> &GameData {
        &self.loaded.game
    }

    fn affixes(&self) -> &AffixTable {
        self.affixes.get_or_init(|| AffixTable::build(self.data()))
    }

    fn index(&self) -> &[IndexEntry] {
        self.index.get_or_init(|| gamedb::build_index(self.data()))
    }

    fn constellations(&self) -> &Constellations {
        self.constellations
            .get_or_init(|| Constellations::read(self.data()))
    }

    fn rules(&self) -> Result<&RespecRules, String> {
        self.rules.as_ref().map_err(|error| {
            format!("the mastery rules could not be read from the record database: {error}")
        })
    }
}

pub struct GrimVault {
    paths: Result<Paths, PathsError>,
    game: Mutex<Option<Arc<Game>>>,
}

impl GrimVault {
    /// A server over the desktop shell's settings; a server with no
    /// usable settings still starts, and every tool says why it has
    /// nothing to read.
    #[must_use]
    pub fn from_settings() -> Self {
        Self {
            paths: Paths::from_settings(),
            game: Mutex::new(None),
        }
    }

    fn paths(&self) -> Result<&Paths, String> {
        self.paths.as_ref().map_err(ToString::to_string)
    }

    /// The record database, loaded on first use (a few seconds).
    fn game(&self) -> Result<Arc<Game>, String> {
        let mut guard = self
            .game
            .lock()
            .expect("lock poisoned only if a loader panicked");
        if let Some(game) = guard.as_ref() {
            return Ok(Arc::clone(game));
        }
        let paths = self.paths()?;
        let loaded = world::load_game_data(&paths.game).map_err(|error| error.to_string())?;
        let rules = RespecRules::load(&loaded.game);
        let ascension = AscensionTable::read(&loaded.game);
        let game = Arc::new(Game {
            loaded,
            rules,
            ascension,
            affixes: OnceLock::new(),
            index: OnceLock::new(),
            constellations: OnceLock::new(),
        });
        *guard = Some(Arc::clone(&game));
        Ok(game)
    }

    fn store(&self) -> Result<VaultStore, String> {
        world::read_store(&self.paths()?.store).map_err(|error| error.to_string())
    }

    fn characters(&self) -> Result<Vec<CharacterFile>, String> {
        Ok(world::discover_characters(&self.paths()?.save))
    }

    /// The campaign a caller means: when none is given, the one the
    /// desktop shell remembered, else the one whose stash the game
    /// wrote last (the shell's own default); `main`; or a mod folder
    /// that exists.
    fn resolve_campaign(&self, wanted: Option<&str>) -> Result<Campaign, String> {
        let paths = self.paths()?;
        let known = paths.save.campaigns();
        let campaign = match wanted.map(str::trim).filter(|name| !name.is_empty()) {
            None => paths
                .settings
                .campaign
                .clone()
                .unwrap_or_else(|| paths.save.campaign_written_last()),
            Some(name) if name.eq_ignore_ascii_case("main") => Campaign::Main,
            Some(name) => known
                .iter()
                .find(|campaign| campaign.wire_name().eq_ignore_ascii_case(name))
                .cloned()
                .ok_or_else(|| {
                    let names: Vec<String> = known.iter().map(ToString::to_string).collect();
                    format!(
                        "no campaign '{name}' in the save directory (known: {})",
                        names.join(", ")
                    )
                })?,
        };
        if known.contains(&campaign) {
            Ok(campaign)
        } else {
            Err(format!(
                "the remembered campaign {campaign} has no folder in the save directory any more"
            ))
        }
    }

    /// The character a caller means: by name (case-insensitive, then
    /// as a substring), narrowed by realm when two folders share a
    /// name — the main-campaign and the custom-game Zark are two
    /// characters.
    fn resolve_character(
        &self,
        wanted: &str,
        realm: Option<RealmParam>,
    ) -> Result<CharacterFile, String> {
        let all = self.characters()?;
        let realm = realm.map(Realm::from);
        let candidates: Vec<CharacterFile> = all
            .iter()
            .filter(|entry| realm.is_none_or(|realm| entry.realm == realm))
            .cloned()
            .collect();
        let wanted = wanted.trim();
        let lowered = wanted.to_lowercase();
        let exact: Vec<CharacterFile> = candidates
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(wanted))
            .cloned()
            .collect();
        let partial: Vec<CharacterFile> = candidates
            .iter()
            .filter(|entry| entry.name.to_lowercase().contains(&lowered))
            .cloned()
            .collect();
        match (exact.as_slice(), partial.as_slice()) {
            ([one], _) | ([], [one]) => Ok(one.clone()),
            ([], []) => Err(format!(
                "no character matching '{wanted}' (known: {})",
                character_names(&all).join(", ")
            )),
            (several @ [_, _, ..], _) | ([], several) => Err(ambiguous(wanted, several)),
        }
    }

    fn character_summary(game: &Game, entry: &CharacterFile) -> Value {
        match open_character(entry) {
            Ok(file) => {
                let header = file.header();
                let class = game
                    .data()
                    .tag_text(&header.class_tag)
                    .map_or_else(|| header.class_tag.clone(), str::to_string);
                let masteries: Vec<String> = game.rules().map_or_else(
                    |_| Vec::new(),
                    |rules| {
                        file.skills().map_or_else(Vec::new, |skills| {
                            rules
                                .base
                                .masteries
                                .iter()
                                .filter(|tree| {
                                    skills.skills.iter().any(|skill| tree.contains(&skill.name))
                                })
                                .map(|tree| tree.name.clone())
                                .collect()
                        })
                    },
                );
                json!({
                    "name": header.name,
                    "realm": realm_name(entry.realm),
                    "level": header.level,
                    "class": if class.is_empty() { Value::Null } else { json!(class) },
                    "masteries": masteries,
                    "hardcore": header.hardcore,
                    "iron_bits": file.character_info().map(|info| info.money),
                    "path": entry.path.display().to_string(),
                })
            }
            Err(error) => json!({
                "name": entry.name,
                "realm": realm_name(entry.realm),
                "path": entry.path.display().to_string(),
                "error": error,
            }),
        }
    }

    /// Every item the server can see, each with where it is, handed
    /// to `consider` — the store, the open campaign's stashes, and
    /// every character's gear, sacks, and own stash — narrowed by
    /// `scope`.
    fn visit_items(
        &self,
        scope: ScopeParam,
        campaign: Option<&str>,
        character: Option<&str>,
        consider: &mut dyn FnMut(String, &Item),
    ) -> Result<Vec<String>, String> {
        let mut skipped = Vec::new();
        let paths = self.paths()?;
        if matches!(scope, ScopeParam::All | ScopeParam::Vault) {
            let store = self.store()?;
            for stored in store.items() {
                consider(format!("vault {}", stored.id()), stored.item());
            }
        }
        if matches!(scope, ScopeParam::All | ScopeParam::Stashes) {
            let campaign = self.resolve_campaign(campaign)?;
            match world::read_gst(&paths.save.transfer_stash(&campaign)) {
                Ok(file) => {
                    if let Some(stash) = file.transfer_stash() {
                        for (index, tab) in stash.tabs.iter().enumerate() {
                            for placed in &tab.items {
                                consider(
                                    format!("{campaign} transfer stash › tab {index}"),
                                    &placed.item,
                                );
                            }
                        }
                    }
                }
                Err(error) => skipped.push(error.to_string()),
            }
            let reagents = paths.save.reagent_storage(&campaign);
            if reagents.is_file() {
                match world::read_gst(&reagents) {
                    Ok(file) => {
                        if let Some(storage) = file.reagent_storage() {
                            for entry in &storage.entries {
                                let item = Item {
                                    base_name: entry.record.clone(),
                                    stack_count: entry.count,
                                    ..Item::default()
                                };
                                consider(format!("{campaign} component storage"), &item);
                            }
                        }
                    }
                    Err(error) => skipped.push(error.to_string()),
                }
            }
        }
        if matches!(scope, ScopeParam::All | ScopeParam::Characters) {
            let entries = match character {
                Some(name) => vec![self.resolve_character(name, None)?],
                None => self.characters()?,
            };
            for entry in entries {
                let file = match open_character(&entry) {
                    Ok(file) => file,
                    Err(error) => {
                        skipped.push(error);
                        continue;
                    }
                };
                let who = format!("character '{}' ({})", entry.name, realm_name(entry.realm));
                if let Some(inventory) = file.inventory() {
                    if let Some(contents) = inventory.contents() {
                        for (slot, worn) in contents.slots() {
                            if !worn.item.is_empty() {
                                consider(format!("{who} › equipped › {slot}"), &worn.item);
                            }
                        }
                    }
                    for (index, sack) in inventory.sacks().iter().enumerate() {
                        for placed in &sack.items {
                            consider(format!("{who} › sack {index}"), &placed.item);
                        }
                    }
                }
                if let Some(stash) = file.stash() {
                    for (index, tab) in stash.tabs.iter().enumerate() {
                        for placed in &tab.items {
                            consider(format!("{who} › own stash › tab {index}"), &placed.item);
                        }
                    }
                }
            }
        }
        Ok(skipped)
    }
}

fn open_character(entry: &CharacterFile) -> Result<PlayerFile, String> {
    world::read_character(&entry.path).map_err(|error| error.to_string())
}

fn character_names(all: &[CharacterFile]) -> Vec<String> {
    all.iter()
        .map(|entry| format!("{} ({})", entry.name, realm_name(entry.realm)))
        .collect()
}

fn ambiguous(wanted: &str, several: &[CharacterFile]) -> String {
    let names: Vec<String> = several
        .iter()
        .map(|entry| format!("{} ({})", entry.name, realm_name(entry.realm)))
        .collect();
    format!(
        "'{wanted}' matches several characters ({}); pass realm: \"main\" or \"custom\", or a longer name",
        names.join(", ")
    )
}

fn realm_name(realm: Realm) -> &'static str {
    match realm {
        Realm::Main => "main",
        Realm::Custom => "custom",
    }
}

fn ok_json<T: serde::Serialize>(value: &T) -> CallToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => fail(format!("serialize response: {error}")),
    }
}

fn fail(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn or_fail<T: serde::Serialize>(result: Result<T, String>) -> CallToolResult {
    match result {
        Ok(value) => ok_json(&value),
        Err(message) => fail(message),
    }
}

fn resolve_group(wanted: &str) -> Result<Group, String> {
    Group::ALL
        .into_iter()
        .find(|group| group.label().eq_ignore_ascii_case(wanted.trim()))
        .ok_or_else(|| {
            let names: Vec<&str> = Group::ALL.iter().map(|group| group.label()).collect();
            format!("no group '{wanted}' (known: {})", names.join(", "))
        })
}

/// A bucket by its label, case-insensitive, the singular accepted
/// ("ring" for "Rings").
fn resolve_bucket(wanted: &str) -> Result<Bucket, String> {
    let wanted_lower = wanted.trim().to_lowercase();
    Bucket::ALL
        .into_iter()
        .find(|bucket| {
            let label = bucket.label().to_lowercase();
            label == wanted_lower || label.strip_suffix('s') == Some(wanted_lower.as_str())
        })
        .ok_or_else(|| {
            let names: Vec<&str> = Bucket::ALL.iter().map(|bucket| bucket.label()).collect();
            format!("no bucket '{wanted}' (known: {})", names.join(", "))
        })
}

fn resolve_rarity(wanted: &str) -> Result<Rarity, String> {
    Rarity::parse(wanted.trim()).ok_or_else(|| {
        let names: Vec<&str> = Rarity::ALL.iter().map(|rarity| rarity.label()).collect();
        format!("no rarity '{wanted}' (known: {})", names.join(", "))
    })
}

fn record_id(field: &str, raw: &str) -> Result<RecordId, String> {
    RecordId::parse(raw.to_string()).ok_or_else(|| format!("{field} is empty"))
}

fn constraint(required: Option<bool>) -> Constraint {
    if required == Some(true) {
        Constraint::Required
    } else {
        Constraint::Any
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RealmParam {
    /// `save/main/`: main-campaign characters.
    Main,
    /// `save/user/`: custom-game (mod) characters.
    Custom,
}

impl From<RealmParam> for Realm {
    fn from(realm: RealmParam) -> Self {
        match realm {
            RealmParam::Main => Self::Main,
            RealmParam::Custom => Self::Custom,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScopeParam {
    /// The vault store, the open campaign's stashes, and every
    /// character.
    #[default]
    All,
    /// The vault store only.
    Vault,
    /// The campaign's transfer stash and component storage only.
    Stashes,
    /// Characters' gear, sacks, and own stashes only.
    Characters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StashKind {
    /// The campaign's shared transfer stash (`transfer.gst`).
    Transfer,
    /// The campaign's component and crafting-material storage
    /// (`reagents.gst`).
    Components,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SocketParam {
    /// Only items carrying a component.
    Socketed,
    /// Only items with an empty socket.
    Unsocketed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AscensionParam {
    /// Items Fangs of Asterkarn lets you upgrade that have not been.
    Upgradeable,
    /// Items already ascended.
    Ascended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PositionParam {
    Prefix,
    Suffix,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BlueprintFilter {
    /// Blueprints the vault has recorded as learned, in any campaign.
    #[default]
    Known,
    /// Blueprints the campaign's own `formulas.gst` lists as learned.
    Learned,
    /// Blueprints the record database offers that the campaign has
    /// not learned.
    Unlearned,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CharacterParams {
    /// Character name (case-insensitive; a substring is accepted when
    /// it matches exactly one).
    pub character: String,
    /// Narrow to `main` (main campaign) or `custom` (custom game /
    /// mod) when two characters share the name.
    pub realm: Option<RealmParam>,
    /// Include every item in the sacks and own stash tabs (default
    /// true). false lists gear only and counts the rest.
    pub items: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct StashParams {
    /// Which stash: `transfer` (the shared stash) or `components`
    /// (the component / crafting-material storage).
    pub kind: StashKind,
    /// The campaign whose files to read: `main`, or a mod's folder
    /// name (as `overview` lists them). Omit for the campaign the
    /// desktop shell opened last.
    pub campaign: Option<String>,
    /// One transfer-stash tab (0-based); omit for all.
    pub tab: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct StoreParams {
    /// A group name, case-insensitive (Weapons, Armor, Accessories,
    /// Item Upgrades, Crafting, Consumables, Other).
    pub group: Option<String>,
    /// A bucket name, case-insensitive, singular accepted (e.g.
    /// "Rings", "Component", "Two-Handed"); see `list_buckets`.
    pub bucket: Option<String>,
    /// Include each item's tooltip lines (default false; large).
    pub details: Option<bool>,
    /// Maximum items to return (default 200).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct StatCriterion {
    /// Case-insensitive substring of a stat line as the game shows it
    /// ("Pierce Resistance", "to Cadence", "Offensive Ability").
    pub text: String,
    /// Lowest acceptable value of the line's largest number.
    pub min: Option<f32>,
    /// Highest acceptable value of the line's largest number.
    pub max: Option<f32>,
    /// true: the line must come from the item's prefix or suffix
    /// rather than the base record.
    pub affix_only: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Case-insensitive substring of the item's display name.
    pub name: Option<String>,
    /// Stat lines the item must carry, all of them.
    pub stats: Option<Vec<StatCriterion>>,
    /// A prefix or suffix name the item must carry ("Cleric's", "of
    /// Blood").
    pub affix: Option<String>,
    /// Highest level requirement accepted.
    pub max_level: Option<u32>,
    pub max_physique: Option<u32>,
    pub max_cunning: Option<u32>,
    pub max_spirit: Option<u32>,
    /// Common, Magical, Rare, Epic, Legendary, or Quest — the rarity
    /// the item displays as.
    pub rarity: Option<String>,
    /// Restrict to one group (see `list_buckets`).
    pub group: Option<String>,
    /// Restrict to one bucket (see `list_buckets`).
    pub bucket: Option<String>,
    /// `member` for any set item, or a set name (substring).
    pub set: Option<String>,
    pub socket: Option<SocketParam>,
    /// true: only monster infrequents.
    pub monster_infrequent: Option<bool>,
    /// true: only double rares (rare prefix and rare suffix).
    pub double_rare: Option<bool>,
    pub ascension: Option<AscensionParam>,
    /// Where to look (default all).
    pub scope: Option<ScopeParam>,
    /// The campaign whose stashes to search (default: the one opened
    /// last).
    pub campaign: Option<String>,
    /// Search one character only.
    pub character: Option<String>,
    /// Include each hit's tooltip lines (default false).
    pub details: Option<bool>,
    /// Maximum hits to return (default 100).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ItemDetailsParams {
    /// The item's base record path (as returned by the other tools).
    pub base_record: String,
    pub prefix_record: Option<String>,
    pub suffix_record: Option<String>,
    pub modifier_record: Option<String>,
    pub transmute_record: Option<String>,
    /// The component in the item's socket.
    pub component_record: Option<String>,
    /// The component's completion bonus.
    pub completion_bonus_record: Option<String>,
    pub augment_record: Option<String>,
    /// The item's seed; affects rolled values within ranges.
    pub seed: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MasteryParams {
    /// Mastery name ("Soldier"), index (1–10), or a substring of its
    /// tree record's path.
    pub mastery: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SkillParams {
    /// The skill's record path (as `get_mastery`, `get_constellation`
    /// or `get_character` return it).
    pub record: String,
    /// Levels to render the skill at (default [1]); its lines at each.
    pub levels: Option<Vec<u32>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AffixParams {
    /// Case-insensitive substring of the affix name or of a stat it
    /// grants ("Cleric", "Pierce Resistance").
    pub text: Option<String>,
    pub position: Option<PositionParam>,
    /// Magical, Rare, Epic, or Legendary.
    pub rarity: Option<String>,
    /// Include each entry's tier records (default false).
    pub tiers: Option<bool>,
    /// Maximum entries to return (default 50).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BlueprintParams {
    /// Which list (default known).
    pub which: Option<BlueprintFilter>,
    /// The campaign whose `formulas.gst` to read for `learned` and
    /// `unlearned` (default: the one opened last).
    pub campaign: Option<String>,
    /// Case-insensitive substring of the blueprint's name.
    pub name: Option<String>,
    /// Maximum entries to return (default 200).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ConstellationParams {
    /// Constellation name ("Bat", "Kraken"; case-insensitive, a
    /// substring is accepted) or a substring of its record path.
    pub constellation: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetsParams {
    /// Case-insensitive substring of the set's name or a member's
    /// name.
    pub name: Option<String>,
    /// Maximum sets to return (default 100).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SetParams {
    /// The set's record path (as `list_sets` or an item's details
    /// return it), or its name (case-insensitive).
    pub set: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordSearchParams {
    /// Case-insensitive substring matched against record paths,
    /// localized names, and file descriptions. Empty lists by class.
    pub query: String,
    /// Optional record class filter, e.g. `ItemRelic`, `Skill_Mastery`,
    /// `LootRandomizer` (see `list_record_classes`).
    pub class: Option<String>,
    /// Maximum hits to return (default 50).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordParams {
    /// The record's database path, e.g.
    /// `records/items/gearrelic/d011_relic.dbr`.
    pub record: String,
    /// true = byte-faithful dump including template-default
    /// (all-zero) variables; default omits them.
    pub everything: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ClassesParams {
    /// Case-insensitive substring of the class name ("Skill", "Item").
    pub filter: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TagParams {
    /// A localization tag, e.g. "tagSkillClassName01".
    pub tag: String,
}

const DEFAULT_STORE_LIMIT: usize = 200;
const DEFAULT_SEARCH_LIMIT: usize = 100;
const DEFAULT_AFFIX_LIMIT: usize = 50;
const DEFAULT_BLUEPRINT_LIMIT: usize = 200;
const DEFAULT_RECORD_SEARCH_LIMIT: usize = 50;
const DEFAULT_SETS_LIMIT: usize = 100;

/// The template every item-set record uses.
const ITEM_SET_TEMPLATE: &str = "itemset.tpl";

// The rmcp macros generate `async fn`s that only await when a tool
// is itself async; ours are sync, so the generated bodies trip the
// unused-async lints.
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
#[tool_router]
impl GrimVault {
    #[tool(
        description = "What this server can see right now: the directories from the app's settings, the campaigns in the save directory and which one is open, every character, the vault store's size, the installed mods, and whether the record database is loaded. Call this first to orient."
    )]
    fn overview(&self) -> CallToolResult {
        let paths = match self.paths() {
            Ok(paths) => paths,
            Err(error) => {
                return ok_json(&json!({"configured": false, "problem": error, "read_only": true}));
            }
        };
        let campaigns = paths.save.campaigns();
        let open = self.resolve_campaign(None);
        let store = self.store();
        let loaded = self
            .game
            .lock()
            .expect("lock poisoned only if a loader panicked")
            .as_ref()
            .map(|game| game.loaded.layers.clone());
        ok_json(&json!({
            "configured": true,
            "settings_file": paths.config.settings_file().display().to_string(),
            "game_dir": paths.game.path().display().to_string(),
            "save_dir": paths.save.path().display().to_string(),
            "campaigns": campaigns.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "open_campaign": open.as_ref().map(ToString::to_string).ok(),
            "characters": self.characters().map(|all| character_names(&all)).unwrap_or_default(),
            "store": match store {
                Ok(store) => json!({
                    "path": paths.store.display().to_string(),
                    "items": store.len(),
                    "blueprints_known": store.blueprints().len(),
                }),
                Err(error) => json!({"path": paths.store.display().to_string(), "error": error}),
            },
            "mods": grimvault_io::layers::list_mods(paths.game.path())
                .iter()
                .map(|listing| listing.folder.clone())
                .collect::<Vec<_>>(),
            "record_database": match loaded {
                Some(layers) => json!({"loaded": true, "layers": layers}),
                None => json!({"loaded": false, "note": "loads on the first tool that needs it (a few seconds)"}),
            },
            "read_only": true,
        }))
    }

    #[tool(
        description = "List every character found under save/main (main campaign) and save/user (custom games), with level, class, masteries, hardcore flag, iron bits, and file path."
    )]
    fn list_characters(&self) -> CallToolResult {
        let result = self.game().and_then(|game| {
            Ok(self
                .characters()?
                .iter()
                .map(|entry| Self::character_summary(&game, entry))
                .collect::<Vec<_>>())
        });
        or_fail(result)
    }

    #[tool(
        description = "Full detail for one character: header, attributes and pools, unspent points, the build (skills grouped by mastery with points and tiers, devotions, item-granted skills), worn gear in both weapon sets, every sack, the own stash, and play statistics."
    )]
    fn get_character(&self, Parameters(params): Parameters<CharacterParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let entry = self.resolve_character(&params.character, params.realm)?;
            let file = open_character(&entry)?;
            let data = game.data();
            let header = file.header();
            let mut out = json!({
                "name": header.name,
                "realm": realm_name(entry.realm),
                "path": entry.path.display().to_string(),
                "hardcore": header.hardcore,
                "sex": format!("{:?}", header.sex),
                "build": build::build_json(data, game.rules.as_ref(), game.constellations(), &file),
            });
            if let Some(info) = file.character_info() {
                out["iron_bits"] = json!(info.money);
                out["current_tribute"] = json!(info.current_tribute);
                out["greatest_difficulty"] = json!(info.greatest_difficulty);
            }
            if let Some(stats) = file.stats() {
                out["statistics"] = json!({
                    "playtime_seconds": stats.playtime,
                    "deaths": stats.deaths,
                    "kills": stats.kills,
                    "champion_kills": stats.champion_kills,
                    "hero_kills": stats.hero_kills,
                    "items_crafted": stats.items_crafted,
                    "shrines_restored": stats.shrines_restored,
                    "lore_notes_collected": stats.lore_notes_collected,
                });
            }
            let with_items = params.items.unwrap_or(true);
            match file.inventory() {
                None => out["equipment"] = json!({"note": "block 3 (inventory) is not typed"}),
                Some(inventory) => {
                    out["equipment"] = view::equipment_json(data, &game.ascension, inventory);
                    if with_items {
                        out["sacks"] = json!(view::sacks_json(data, &game.ascension, inventory));
                    } else {
                        out["sacks"] = json!(
                            inventory
                                .sacks()
                                .iter()
                                .enumerate()
                                .map(|(index, sack)| json!({"sack": index, "items": sack.items.len()}))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
            match file.stash() {
                None => out["own_stash"] = json!({"note": "block 4 (stash) is not typed"}),
                Some(stash) => {
                    out["own_stash"] = if with_items {
                        json!(view::tabs_json(data, &game.ascension, &stash.tabs, None))
                    } else {
                        json!(view::stash_summary(stash))
                    };
                }
            }
            Ok(out)
        });
        or_fail(result)
    }

    #[tool(
        description = "A campaign's shared stash tabs with every item placed in them, or its component / crafting-material storage with counts. Campaigns are the main game and each mod folder under the save directory."
    )]
    fn get_stash(&self, Parameters(params): Parameters<StashParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let paths = self.paths()?;
            let campaign = self.resolve_campaign(params.campaign.as_deref())?;
            let data = game.data();
            match params.kind {
                StashKind::Transfer => {
                    let path = paths.save.transfer_stash(&campaign);
                    let file = world::read_gst(&path).map_err(|error| error.to_string())?;
                    let stash = file.transfer_stash().ok_or_else(|| {
                        format!("{} holds no transfer stash block", path.display())
                    })?;
                    if let Some(tab) = params.tab
                        && tab >= stash.tabs.len()
                    {
                        return Err(format!(
                            "tab {tab} is out of range: {} tabs",
                            stash.tabs.len()
                        ));
                    }
                    Ok(json!({
                        "campaign": campaign.to_string(),
                        "path": path.display().to_string(),
                        "mod_name_inside": stash.mod_name,
                        "tabs": view::tabs_json(data, &game.ascension, &stash.tabs, params.tab),
                    }))
                }
                StashKind::Components => {
                    let path = paths.save.reagent_storage(&campaign);
                    if !path.is_file() {
                        return Ok(json!({
                            "campaign": campaign.to_string(),
                            "path": path.display().to_string(),
                            "note": "absent: the game writes it once the storage is used",
                            "entries": [],
                        }));
                    }
                    let file = world::read_gst(&path).map_err(|error| error.to_string())?;
                    let storage = file.reagent_storage().ok_or_else(|| {
                        format!("{} holds no component storage block", path.display())
                    })?;
                    let mut entries: Vec<Value> = storage
                        .entries
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| {
                            let kind = view::base_info(
                                data,
                                &Item {
                                    base_name: entry.record.clone(),
                                    ..Item::default()
                                },
                            )
                            .and_then(|info| info.reagent);
                            json!({
                                "index": index,
                                "record": entry.record,
                                "name": view::record_name(data, &entry.record),
                                "count": entry.count,
                                "kind": ReagentKind::in_storage(kind).label(),
                            })
                        })
                        .collect();
                    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    Ok(json!({
                        "campaign": campaign.to_string(),
                        "path": path.display().to_string(),
                        "mod_name_inside": storage.mod_name,
                        "total_units": storage.total_count(),
                        "entries": entries,
                    }))
                }
            }
        });
        or_fail(result)
    }

    #[tool(
        description = "The vault store's groups and type buckets with how many items each holds. Buckets are computed from each item's own record — nothing is filed by hand — so they are always accurate."
    )]
    fn list_buckets(&self) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let store = self.store()?;
            let mut counts: BTreeMap<Bucket, usize> = BTreeMap::new();
            for stored in store.items() {
                if let Some(info) = view::base_info(game.data(), stored.item()) {
                    *counts
                        .entry(Bucket::of(info.class.as_ref(), info.reagent))
                        .or_insert(0) += 1;
                }
            }
            Ok(json!({
                "items": store.len(),
                "blueprints_known": store.blueprints().len(),
                "groups": Group::ALL
                    .iter()
                    .map(|group| {
                        json!({
                            "group": group.label(),
                            "buckets": Bucket::ALL
                                .iter()
                                .filter(|bucket| bucket.group() == *group)
                                .map(|bucket| json!({"bucket": bucket.label(), "items": counts.get(bucket).copied().unwrap_or(0)}))
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect::<Vec<_>>(),
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Items in the vault store, each with its stored id, where it came from, and when. Filter by group or bucket (see list_buckets); omit both for everything."
    )]
    fn get_store(&self, Parameters(params): Parameters<StoreParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let store = self.store()?;
            let group = params.group.as_deref().map(resolve_group).transpose()?;
            let bucket = params.bucket.as_deref().map(resolve_bucket).transpose()?;
            let details = params.details.unwrap_or(false);
            let limit = params.limit.unwrap_or(DEFAULT_STORE_LIMIT);
            let data = game.data();
            let mut total = 0_usize;
            let mut items = Vec::new();
            for stored in store.items() {
                let filed = view::base_info(data, stored.item())
                    .map(|info| Bucket::of(info.class.as_ref(), info.reagent));
                if group.is_some_and(|group| filed.is_none_or(|filed| filed.group() != group))
                    || bucket.is_some_and(|bucket| filed != Some(bucket))
                {
                    continue;
                }
                total += 1;
                if items.len() < limit {
                    let mut item = item_json(data, &game.ascension, stored.item(), details);
                    item["stored_id"] = json!(stored.id().to_string());
                    item["origin"] = serde_json::to_value(stored.origin()).unwrap_or(Value::Null);
                    item["stored_at_unix"] = json!(stored.stored_at().unix_seconds());
                    items.push(item);
                }
            }
            Ok(json!({
                "total_matches": total,
                "shown": items.len(),
                "items": items,
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Search every possession — the vault store, the open campaign's transfer stash and component storage, and every character's gear, sacks and own stash — with the app's typed query: name, stat lines with value bounds, affix name, requirement caps, rarity, group or bucket, set membership, socket state, monster infrequent, double rare, ascension. Each hit carries its exact location. The build-planning tool: 'what do I own with +skills to Soldier under level 60'."
    )]
    fn search_items(&self, Parameters(params): Parameters<SearchParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let query = build_query(&params)?;
            let scope = params.scope.unwrap_or_default();
            let details = params.details.unwrap_or(false);
            let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
            let data = game.data();
            let mut hits = Vec::new();
            let mut total = 0_usize;
            let mut unresolved = 0_usize;
            let mut consider = |location: String, item: &Item| {
                let resolved = Resolved::of(data, &game.ascension, item);
                match query.verdict(&resolved.subject(item)) {
                    Verdict::Excluded => {}
                    Verdict::Unresolved => unresolved += 1,
                    Verdict::Matches => {
                        total += 1;
                        if hits.len() < limit {
                            let mut hit = item_json(data, &game.ascension, item, details);
                            hit["location"] = json!(location);
                            hits.push(hit);
                        }
                    }
                }
            };
            let skipped = self.visit_items(
                scope,
                params.campaign.as_deref(),
                params.character.as_deref(),
                &mut consider,
            )?;
            Ok(json!({
                "total_matches": total,
                "shown": hits.len(),
                "undecidable": unresolved,
                "skipped": skipped,
                "hits": hits,
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Tooltip-grade stat lines for an item given its record paths and seed (as returned by the other tools): every block by source (base, prefix, suffix, component, completion bonus, augment), requirements, the set it belongs to and the set bonuses, plus its classification and facets."
    )]
    fn get_item_details(
        &self,
        Parameters(params): Parameters<ItemDetailsParams>,
    ) -> CallToolResult {
        let result = self.game().and_then(|game| {
            record_id("base_record", &params.base_record)?;
            let item = Item {
                base_name: params.base_record,
                prefix_name: params.prefix_record.unwrap_or_default(),
                suffix_name: params.suffix_record.unwrap_or_default(),
                modifier_name: params.modifier_record.unwrap_or_default(),
                transmute_name: params.transmute_record.unwrap_or_default(),
                relic_name: params.component_record.unwrap_or_default(),
                relic_bonus: params.completion_bonus_record.unwrap_or_default(),
                augment_name: params.augment_record.unwrap_or_default(),
                seed: params.seed.unwrap_or(0),
                ..Item::default()
            };
            Ok(item_json(game.data(), &game.ascension, &item, true))
        });
        or_fail(result)
    }

    #[tool(
        description = "List the playable masteries the record database names, in the game's own numbering (the header class tag spells the chosen indices, e.g. 0306 = masteries 3 and 6)."
    )]
    fn list_masteries(&self) -> CallToolResult {
        let result = self
            .game()
            .and_then(|game| Ok(skills::masteries_json(game.rules()?)));
        or_fail(result)
    }

    #[tool(
        description = "One mastery's full skill tree: every skill record with localized name, description, class, tier, level caps, prerequisites and the skills it grants or modifies, tier order. Use get_skill for a skill's stat lines at given levels."
    )]
    fn get_mastery(&self, Parameters(params): Parameters<MasteryParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let tree = skills::resolve_mastery(game.rules()?, &params.mastery)?;
            Ok(skills::mastery_json(game.data(), tree))
        });
        or_fail(result)
    }

    #[tool(
        description = "One skill (mastery, devotion, or item-granted) with its summary and its stat lines rendered at each requested level, as the game would show them."
    )]
    fn get_skill(&self, Parameters(params): Parameters<SkillParams>) -> CallToolResult {
        let levels = params.levels.unwrap_or_else(|| vec![1]);
        let result = self
            .game()
            .and_then(|game| skills::skill_json(game.data(), &params.record, &levels));
        or_fail(result)
    }

    #[tool(
        description = "The affix reference: every named prefix and suffix (Cleric's, of Blood, …) with the rarity, the level range across its tiers, and each stat it grants written as its range across tiers, marked when only some tiers carry it. Search by name or by a stat's text."
    )]
    fn search_affixes(&self, Parameters(params): Parameters<AffixParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let query = AffixQuery {
                text: params.text.unwrap_or_default(),
                position: params.position.map(|position| match position {
                    PositionParam::Prefix => Position::Prefix,
                    PositionParam::Suffix => Position::Suffix,
                }),
                rarity: params.rarity.as_deref().map(resolve_rarity).transpose()?,
            };
            let with_tiers = params.tiers.unwrap_or(false);
            let limit = params.limit.unwrap_or(DEFAULT_AFFIX_LIMIT);
            let table = game.affixes();
            let matching: Vec<_> = table.matching(&query).collect();
            let entries: Vec<Value> = matching
                .iter()
                .take(limit)
                .map(|(_, entry)| {
                    let mut out = json!({
                        "name": entry.name,
                        "position": match entry.position {
                            Position::Prefix => "prefix",
                            Position::Suffix => "suffix",
                        },
                        "rarity": entry.rarity.map(Rarity::label),
                        "levels": entry.levels.map(|range| json!({"low": range.low, "high": range.high})),
                        "tiers": entry.tiers.len(),
                        "grants": entry
                            .grants
                            .iter()
                            .map(|grant| {
                                json!({
                                    "line": grant.line.text,
                                    "coverage": match grant.coverage {
                                        Coverage::Every => "every tier".to_string(),
                                        Coverage::Some { tiers, of } => format!("{tiers} of {of} tiers"),
                                    },
                                })
                            })
                            .collect::<Vec<_>>(),
                    });
                    if with_tiers {
                        out["tier_records"] = json!(
                            entry
                                .tiers
                                .iter()
                                .map(|tier| json!({
                                    "record": tier.record.as_str(),
                                    "level": tier.level,
                                    "lines": view::line_texts(&tier.stats.lines),
                                }))
                                .collect::<Vec<_>>()
                        );
                    }
                    out
                })
                .collect();
            Ok(json!({
                "total_matches": matching.len(),
                "shown": entries.len(),
                "entries": entries,
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Blueprints: those the vault has recorded as learned (with the campaign and moment), those a campaign's formulas.gst lists as learned, or those the record database offers that the campaign has not learned — each named."
    )]
    fn list_blueprints(&self, Parameters(params): Parameters<BlueprintParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let data = game.data();
            let which = params.which.unwrap_or_default();
            let needle = params.name.as_deref().map(str::to_lowercase);
            let limit = params.limit.unwrap_or(DEFAULT_BLUEPRINT_LIMIT);
            let admits = |name: &str| {
                needle
                    .as_deref()
                    .is_none_or(|needle| name.to_lowercase().contains(needle))
            };
            let mut entries: Vec<Value> = match which {
                BlueprintFilter::Known => self
                    .store()?
                    .blueprints()
                    .iter()
                    .filter_map(|learned| {
                        let name = view::record_name(data, &learned.record);
                        admits(&name).then(|| {
                            json!({
                                "record": learned.record,
                                "name": name,
                                "campaign": learned.campaign.to_string(),
                                "learned_at_unix": learned.learned_at.unix_seconds(),
                            })
                        })
                    })
                    .collect(),
                BlueprintFilter::Learned | BlueprintFilter::Unlearned => {
                    let campaign = self.resolve_campaign(params.campaign.as_deref())?;
                    let path = self.paths()?.save.blueprints(&campaign);
                    let learned = world::read_formulas(&path).map_err(|error| error.to_string())?;
                    let entries = learned.map(|formulas| formulas.entries).unwrap_or_default();
                    if which == BlueprintFilter::Learned {
                        entries
                            .iter()
                            .filter_map(|entry| {
                                let name = view::record_name(data, &entry.record);
                                admits(&name).then(|| {
                                    json!({
                                        "record": entry.record,
                                        "name": name,
                                        "campaign": campaign.to_string(),
                                        "new": entry.read == FormulaRead::Unread,
                                    })
                                })
                            })
                            .collect()
                    } else {
                        let known: std::collections::HashSet<String> = entries
                            .iter()
                            .map(|entry| normalize(&entry.record))
                            .collect();
                        available_blueprints(data)
                            .iter()
                            .filter(|id| !known.contains(&normalize(id.as_str())))
                            .filter_map(|id| {
                                let name = view::record_name(data, id.as_str());
                                admits(&name).then(|| {
                                    json!({
                                        "record": id.as_str(),
                                        "name": name,
                                        "campaign": campaign.to_string(),
                                    })
                                })
                            })
                            .collect()
                    }
                }
            };
            entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let total = entries.len();
            entries.truncate(limit);
            Ok(json!({
                "which": format!("{which:?}").to_lowercase(),
                "total_matches": total,
                "shown": entries.len(),
                "blueprints": entries,
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Every devotion constellation in the game's order: name, record, the affinity it requires and grants, and how many stars it has. Use get_constellation for the stars."
    )]
    fn list_constellations(&self) -> CallToolResult {
        let result = self
            .game()
            .map(|game| devotion::list_json(game.constellations()));
        or_fail(result)
    }

    #[tool(
        description = "One constellation in full: description, affinity required and granted, and every star with its skill record, name, class, which star it hangs from, and its stat lines (a passive star's bonuses, or a proc's lines at level 1)."
    )]
    fn get_constellation(
        &self,
        Parameters(params): Parameters<ConstellationParams>,
    ) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let constellation = game.constellations().resolve(&params.constellation)?;
            Ok(devotion::constellation_json(game.data(), constellation))
        });
        or_fail(result)
    }

    #[tool(
        description = "Every item set the record database defines, with its members' names and item level. First call builds the record index (a few seconds). Use get_set for the bonuses."
    )]
    fn list_sets(&self, Parameters(params): Parameters<SetsParams>) -> CallToolResult {
        let result = self.game().map(|game| {
            let needle = params.name.as_deref().map(str::to_lowercase);
            let limit = params.limit.unwrap_or(DEFAULT_SETS_LIMIT);
            let data = game.data();
            let mut sets: Vec<Value> = game
                .index()
                .iter()
                .filter(|entry| entry.uses_template(ITEM_SET_TEMPLATE))
                .filter_map(|entry| {
                    let id = RecordId::parse(entry.path.clone())?;
                    let info = set_info(data, &id)?;
                    let admits = needle.as_deref().is_none_or(|needle| {
                        info.name.to_lowercase().contains(needle)
                            || info
                                .members
                                .iter()
                                .any(|member| member.to_lowercase().contains(needle))
                    });
                    admits.then(|| {
                        json!({
                            "name": info.name,
                            "record": entry.path,
                            "members": info.members,
                            "item_level": data
                                .record(&id)
                                .and_then(Result::ok)
                                .and_then(|record| record.integer("itemLevel")),
                        })
                    })
                })
                .collect();
            sets.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let total = sets.len();
            sets.truncate(limit);
            json!({"total_matches": total, "shown": sets.len(), "sets": sets})
        });
        or_fail(result)
    }

    #[tool(
        description = "One item set: its members with their records, and the bonus lines each piece count adds, as the game shows them."
    )]
    fn get_set(&self, Parameters(params): Parameters<SetParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let data = game.data();
            let wanted = params.set.trim();
            let by_record = RecordId::parse(wanted.to_string())
                .filter(|id| data.record(id).is_some())
                .and_then(|id| set_info(data, &id).map(|info| (id, info)));
            let (id, info) = if let Some(found) = by_record {
                found
            } else {
                let lowered = wanted.to_lowercase();
                game.index()
                    .iter()
                    .filter(|entry| entry.uses_template(ITEM_SET_TEMPLATE))
                    .find_map(|entry| {
                        let id = RecordId::parse(entry.path.clone())?;
                        let info = set_info(data, &id)?;
                        (info.name.eq_ignore_ascii_case(wanted)
                            || info.name.to_lowercase().contains(&lowered))
                            .then_some((id, info))
                    })
                    .ok_or_else(|| format!("no item set matching '{wanted}'"))?
            };
            let record = data.record(&id).and_then(Result::ok);
            let member_records: Vec<String> = record
                .as_ref()
                .and_then(|record| record.variable("setMembers"))
                .map(|members| match &members.values {
                    univault_engine::arz::DbValues::Strings(values) => values.clone(),
                    univault_engine::arz::DbValues::Integers(_)
                    | univault_engine::arz::DbValues::Floats(_)
                    | univault_engine::arz::DbValues::Booleans(_) => Vec::new(),
                })
                .unwrap_or_default();
            Ok(json!({
                "name": info.name,
                "record": id.as_str(),
                "item_level": record.as_ref().and_then(|record| record.integer("itemLevel")),
                "members": member_records
                    .iter()
                    .map(|member| json!({"record": member, "name": view::record_name(data, member)}))
                    .collect::<Vec<_>>(),
                "bonuses": info
                    .tiers
                    .iter()
                    .map(|tier| json!({"pieces": tier.pieces, "lines": view::line_texts(&tier.lines)}))
                    .collect::<Vec<_>>(),
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Search the entire record database (items, skills, monsters, loot tables, equations — every record of every layer) by path substring, localized name, or file description, optionally filtered by record class. First call builds an index (a few seconds)."
    )]
    fn search_records(&self, Parameters(params): Parameters<RecordSearchParams>) -> CallToolResult {
        let result = self.game().map(|game| {
            let needle = Needle::new(&params.query);
            let class = params
                .class
                .as_deref()
                .map(str::trim)
                .filter(|class| !class.is_empty());
            let limit = params.limit.unwrap_or(DEFAULT_RECORD_SEARCH_LIMIT);
            let mut hits = Vec::new();
            let mut total = 0_usize;
            for entry in game.index() {
                if class.is_some_and(|class| !entry.class.eq_ignore_ascii_case(class)) {
                    continue;
                }
                if !needle.is_empty() && !entry.mentions(&needle) {
                    continue;
                }
                total += 1;
                if hits.len() < limit {
                    hits.push(json!({
                        "record": entry.path,
                        "class": entry.class,
                        "template": entry.template,
                        "name": entry.name,
                        "file_description": entry.file_description,
                    }));
                }
            }
            json!({
                "query": params.query,
                "total_matches": total,
                "shown": hits.len(),
                "hits": hits,
            })
        });
        or_fail(result)
    }

    #[tool(
        description = "One database record in full — every variable with values (per-level arrays included) and translated text where tags resolve — and which layers define it, topmost first (a mod layer only fills in what no shipped layer has)."
    )]
    fn get_record(&self, Parameters(params): Parameters<RecordParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            let id = record_id("record", &params.record)?;
            let record = match game.data().record(&id) {
                None => {
                    return Err(format!(
                        "no layer of the record database has {}",
                        params.record
                    ));
                }
                Some(Err(error)) => return Err(format!("{}: {error}", params.record)),
                Some(Ok(record)) => record,
            };
            let layers = gamedb::layers_of(game.data(), &game.loaded.layers, &id);
            Ok(gamedb::record_json(
                game.data(),
                &record,
                &layers,
                params.everything.unwrap_or(false),
            ))
        });
        or_fail(result)
    }

    #[tool(
        description = "Every record class in the database with how many records it has — the vocabulary for search_records' class filter."
    )]
    fn list_record_classes(&self, Parameters(params): Parameters<ClassesParams>) -> CallToolResult {
        let result = self.game().map(|game| {
            let needle = params.filter.as_deref().map(str::to_lowercase);
            gamedb::class_counts(game.data())
                .into_iter()
                .filter(|(class, _)| {
                    needle
                        .as_deref()
                        .is_none_or(|needle| class.to_lowercase().contains(needle))
                })
                .map(|(class, count)| json!({"class": class, "records": count}))
                .collect::<Vec<_>>()
        });
        or_fail(result)
    }

    #[tool(
        description = "Translate one localization tag (e.g. tagSkillClassName01) to its English text."
    )]
    fn translate_tag(&self, Parameters(params): Parameters<TagParams>) -> CallToolResult {
        let result = self.game().and_then(|game| {
            game.data()
                .tag_text(params.tag.trim())
                .map(|text| json!({"tag": params.tag, "text": text}))
                .ok_or_else(|| format!("no text for tag '{}'", params.tag))
        });
        or_fail(result)
    }
}

/// The typed query the search view runs, from the tool's parameters.
fn build_query(params: &SearchParams) -> Result<Query, String> {
    let mut criteria: Vec<Criterion> = params
        .stats
        .iter()
        .flatten()
        .map(|stat| {
            let bounds = ValueBounds {
                min: stat.min,
                max: stat.max,
            };
            if stat.affix_only == Some(true) {
                Criterion::AffixStat {
                    text: stat.text.clone(),
                    bounds,
                }
            } else {
                Criterion::StatContains {
                    text: stat.text.clone(),
                    bounds,
                }
            }
        })
        .collect();
    if let Some(affix) = &params.affix {
        criteria.push(Criterion::HasAffix {
            text: affix.clone(),
        });
    }
    let category = match (&params.bucket, &params.group) {
        (Some(bucket), _) => CategoryFilter::Bucket {
            bucket: resolve_bucket(bucket)?,
        },
        (None, Some(group)) => CategoryFilter::Group {
            group: resolve_group(group)?,
        },
        (None, None) => CategoryFilter::Any,
    };
    let set = match params.set.as_deref().map(str::trim) {
        None | Some("") => SetFilter::Any,
        Some(word) if word.eq_ignore_ascii_case("member") || word.eq_ignore_ascii_case("any") => {
            SetFilter::Member
        }
        Some(name) => SetFilter::Named {
            text: name.to_string(),
        },
    };
    Ok(Query {
        name: params.name.clone().unwrap_or_default(),
        criteria,
        requirements: RequirementCaps {
            level: params.max_level,
            physique: params.max_physique,
            cunning: params.max_cunning,
            spirit: params.max_spirit,
        },
        set,
        rarity: params.rarity.as_deref().map(resolve_rarity).transpose()?,
        category,
        socket: match params.socket {
            None => SocketFilter::Any,
            Some(SocketParam::Socketed) => SocketFilter::Socketed,
            Some(SocketParam::Unsocketed) => SocketFilter::Unsocketed,
        },
        monster_infrequent: constraint(params.monster_infrequent),
        double_rare: constraint(params.double_rare),
        ascension: match params.ascension {
            None => AscensionFilter::Any,
            Some(AscensionParam::Upgradeable) => AscensionFilter::Upgradeable,
            Some(AscensionParam::Ascended) => AscensionFilter::Ascended,
        },
    })
}

// Same as above: the generated `call_tool`/`list_tools` are async
// with nothing to await because every tool here is sync.
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
#[tool_handler]
impl ServerHandler for GrimVault {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::default();
        info.server_info.name = "grimvault-mcp".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(
            "Read-only access to a Grim Dawn install and its saves, for planning and refining \
             builds around what the player owns: characters with their builds (skills grouped by \
             mastery, devotions, item-granted skills, attributes, unspent points), worn gear, \
             sacks and own stashes; each campaign's transfer stash and component storage; the \
             Grim Vault item store by group and bucket; a typed item search over all of it \
             (stat lines with value bounds, affixes, requirement caps, rarity, sets, sockets, \
             monster infrequent, double rare, ascension); tooltip-grade item details; the \
             mastery skill trees and any skill rendered at chosen levels; the devotion \
             constellations with every star's lines; item sets with their bonuses; the affix \
             reference; blueprints known, learned, and not yet learned; and the entire layered record \
             database (search_records / get_record / translate_tag). Nothing is ever written. \
             Call `overview` first; paths come from the Grim Vault app's settings.json under the \
             config directory (GRIMVAULT_CONFIG_DIR overrides it)."
                .into(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_query_is_built_from_the_parameters() {
        let params = SearchParams {
            name: Some("ring".into()),
            stats: Some(vec![
                StatCriterion {
                    text: "Pierce Resistance".into(),
                    min: Some(10.0),
                    max: None,
                    affix_only: None,
                },
                StatCriterion {
                    text: "to Cadence".into(),
                    min: None,
                    max: None,
                    affix_only: Some(true),
                },
            ]),
            affix: Some("Cleric".into()),
            max_level: Some(60),
            max_physique: None,
            max_cunning: None,
            max_spirit: None,
            rarity: Some("rare".into()),
            group: Some("Accessories".into()),
            bucket: None,
            set: Some("member".into()),
            socket: Some(SocketParam::Unsocketed),
            monster_infrequent: Some(true),
            double_rare: None,
            ascension: Some(AscensionParam::Upgradeable),
            scope: None,
            campaign: None,
            character: None,
            details: None,
            limit: None,
        };
        let query = build_query(&params).unwrap();
        assert_eq!(query.name, "ring");
        assert_eq!(query.criteria.len(), 3);
        assert!(matches!(
            &query.criteria[1],
            Criterion::AffixStat { text, .. } if text == "to Cadence"
        ));
        assert!(matches!(&query.criteria[2], Criterion::HasAffix { text } if text == "Cleric"));
        assert_eq!(query.requirements.level, Some(60));
        assert_eq!(query.rarity, Some(Rarity::Rare));
        assert_eq!(
            query.category,
            CategoryFilter::Group {
                group: Group::Accessories
            }
        );
        assert_eq!(query.set, SetFilter::Member);
        assert_eq!(query.socket, SocketFilter::Unsocketed);
        assert_eq!(query.monster_infrequent, Constraint::Required);
        assert_eq!(query.double_rare, Constraint::Any);
        assert_eq!(query.ascension, AscensionFilter::Upgradeable);
    }

    #[test]
    fn unknown_names_are_refused_with_the_choices() {
        assert!(resolve_group("Hats").unwrap_err().contains("Weapons"));
        assert!(resolve_bucket("Hat").unwrap_err().contains("Ring"));
        assert!(
            resolve_rarity("mythical")
                .unwrap_err()
                .contains("Legendary")
        );
        assert_eq!(resolve_bucket("ring").unwrap(), Bucket::Ring);
        assert_eq!(resolve_bucket("Rings").unwrap(), Bucket::Ring);
        assert_eq!(resolve_bucket("two-handed").unwrap(), Bucket::TwoHanded);
    }
}
