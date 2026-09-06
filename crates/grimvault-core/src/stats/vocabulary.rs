//! The Grim Dawn stat vocabulary: which record variables are stats,
//! how a variable name splits into a stat family and a part
//! (`offensiveFireModifierChance` → `offensiveFire` + `ModifierChance`),
//! which `tags_ui.txt` tag renders each part, how the families sort,
//! and the handful of variables the game displays by their own rules.
//! Every table here was read off the game's own `tags_ui.txt` and the
//! variable census of its item and skill records (2026-09-06,
//! `docs/format-references.md` "Item stat lines"); nothing is copied
//! from another tool's text. This is the dialect the engine-generic
//! renderer in [`super::render`] is parametrised by — the part of the
//! module that would stay behind when the machinery moves to
//! `univault-engine`.

use std::fmt;

/// A stat family the vocabulary knows (`offensiveFire`,
/// `defensiveCold`, `characterLife`, `skillManaCost`…). Only
/// [`parse_variable`] mints one, so a value in hand names a family the
/// tables can render.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Effect {
    family: Family,
    stem: &'static str,
}

impl Effect {
    /// The game's variable stem, e.g. `offensiveFire`.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.stem
    }

    #[must_use]
    pub fn family(self) -> Family {
        self.family
    }

    /// The tail after the family prefix (`Fire` in `offensiveFire`,
    /// `Bleeding` in `offensiveSlowBleeding`).
    fn tail(self) -> &'static str {
        &self.stem[self.family.prefix().len()..]
    }
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.stem)
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.stem)
    }
}

/// The shape a stat family renders in. Flat and duration damage
/// carry a min/max amount the label takes as text; defense and
/// character values carry the number inside their own label; skill
/// stats compose a label with one of the `Skill*Format` tags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Family {
    Offense,
    OffenseDuration,
    Retaliation,
    RetaliationDuration,
    Defense,
    Character,
    SkillStat,
}

impl Family {
    const ALL: [Self; 7] = [
        Self::OffenseDuration,
        Self::Offense,
        Self::RetaliationDuration,
        Self::Retaliation,
        Self::Defense,
        Self::Character,
        Self::SkillStat,
    ];

    /// The variable-name prefix every member starts with. Longer
    /// prefixes are tried first so `offensiveSlowFire` is duration
    /// damage, not the flat effect `offensiveSlowFire`.
    fn prefix(self) -> &'static str {
        match self {
            Self::Offense => "offensive",
            Self::OffenseDuration => "offensiveSlow",
            Self::Retaliation => "retaliation",
            Self::RetaliationDuration => "retaliationSlow",
            Self::Defense => "defensive",
            Self::Character => "character",
            Self::SkillStat => "",
        }
    }

    fn stems(self) -> &'static [&'static str] {
        match self {
            Self::Offense => OFFENSE,
            Self::OffenseDuration => OFFENSE_DURATION,
            Self::Retaliation => RETALIATION,
            Self::RetaliationDuration => RETALIATION_DURATION,
            Self::Defense => DEFENSE,
            Self::Character => CHARACTER,
            Self::SkillStat => SKILL_STAT,
        }
    }

    /// Where the family's lines sit in the tooltip.
    #[must_use]
    pub fn section(self) -> Section {
        match self {
            Self::Offense => Section::Offense,
            Self::OffenseDuration => Section::OffenseDuration,
            Self::Retaliation | Self::RetaliationDuration => Section::Retaliation,
            Self::Defense => Section::Defense,
            Self::Character => Section::Character,
            Self::SkillStat => Section::Skill,
        }
    }
}

/// The sub-variable of a stat family.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Part {
    /// The bare stem: a defense or character value, a skill stat, or
    /// the flat amount when a family has no `Min`.
    Value,
    Min,
    Max,
    Chance,
    Global,
    Xor,
    DurationMin,
    DurationMax,
    /// A defense `…Duration` ("Reduction in Burn Duration"), not an
    /// effect duration.
    Duration,
    DurationChance,
    Modifier,
    ModifierChance,
    DurationModifier,
    DurationModifierChance,
    MaxResist,
    DrainMin,
    DrainMax,
    DamageRatio,
}

/// Suffixes in the order they must be tried: a longer suffix that
/// ends in a shorter one (`DurationModifierChance` / `Chance`) goes
/// first.
const PART_SUFFIXES: [(&str, Part); 17] = [
    ("DurationModifierChance", Part::DurationModifierChance),
    ("DurationModifier", Part::DurationModifier),
    ("DurationChance", Part::DurationChance),
    ("DurationMin", Part::DurationMin),
    ("DurationMax", Part::DurationMax),
    ("ModifierChance", Part::ModifierChance),
    ("Modifier", Part::Modifier),
    ("MaxResist", Part::MaxResist),
    ("DrainMin", Part::DrainMin),
    ("DrainMax", Part::DrainMax),
    ("DamageRatio", Part::DamageRatio),
    ("Chance", Part::Chance),
    ("Global", Part::Global),
    ("XOR", Part::Xor),
    ("Min", Part::Min),
    ("Max", Part::Max),
    ("Duration", Part::Duration),
];

/// A variable name split into its family stem and part.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Attribute {
    pub effect: Effect,
    pub part: Part,
}

/// Splits a record variable name into a known stat family and part.
/// `None` for anything the vocabulary does not know — the caller
/// reports it, never guesses. Matching is case-insensitive because the
/// database itself is not consistent (`retaliationPercentcurrentLifeGlobal`).
#[must_use]
pub fn parse_variable(name: &str) -> Option<Attribute> {
    for family in Family::ALL {
        let prefix = family.prefix();
        if name.len() < prefix.len() || !name[..prefix.len()].eq_ignore_ascii_case(prefix) {
            continue;
        }
        for (suffix, part) in PART_SUFFIXES {
            if let Some(stem) = strip_suffix_ignore_case(name, suffix)
                && let Some(effect) = known_stem(family, stem)
            {
                return Some(Attribute { effect, part });
            }
        }
        if let Some(effect) = known_stem(family, name) {
            return Some(Attribute {
                effect,
                part: Part::Value,
            });
        }
    }
    None
}

fn strip_suffix_ignore_case<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = name.len().checked_sub(suffix.len())?;
    name.is_char_boundary(cut)
        .then(|| &name[..cut])
        .filter(|_| name[cut..].eq_ignore_ascii_case(suffix))
}

fn known_stem(family: Family, stem: &str) -> Option<Effect> {
    family
        .stems()
        .iter()
        .find(|known| known.eq_ignore_ascii_case(stem))
        .map(|known| Effect {
            family,
            stem: known,
        })
}

/// Sort key of an effect: its family's position, then its position in
/// the family table, so lines come out in the game's rough order
/// (base damage, then bonuses by kind).
#[must_use]
pub fn order(effect: Effect) -> (u8, usize) {
    let family_rank = match effect.family {
        Family::Offense => 0,
        Family::OffenseDuration => 1,
        Family::Retaliation => 2,
        Family::RetaliationDuration => 3,
        Family::Defense => 4,
        Family::Character => 5,
        Family::SkillStat => 6,
    };
    let index = effect
        .family
        .stems()
        .iter()
        .position(|stem| *stem == effect.stem)
        .unwrap_or(usize::MAX);
    (family_rank, index)
}

// Flat offense: the tail after `offensive`. `Base*` are the white base
// damage lines of weapons and shields; the `*Reduction*` effects carry
// a `DurationMin` and read "for N Seconds".
const OFFENSE: &[&str] = &[
    "offensiveBasePhysical",
    "offensiveBaseFire",
    "offensiveBaseCold",
    "offensiveBaseLightning",
    "offensiveBasePoison",
    "offensiveBaseLife",
    "offensiveBaseAether",
    "offensiveBaseChaos",
    "offensivePhysical",
    "offensivePierceRatio",
    "offensiveBonusPhysical",
    "offensivePierce",
    "offensiveFire",
    "offensiveCold",
    "offensiveLightning",
    "offensivePoison",
    "offensiveLife",
    "offensiveAether",
    "offensiveChaos",
    "offensiveElemental",
    "offensiveTotalDamage",
    "offensiveCritDamage",
    "offensiveDamageMult",
    "offensiveLifeLeech",
    "offensivePercentCurrentLife",
    "offensiveManaBurn",
    "offensiveStun",
    "offensiveFreeze",
    "offensivePetrify",
    "offensiveTrap",
    "offensiveConfusion",
    "offensiveConvert",
    "offensiveFear",
    "offensiveSleep",
    "offensiveDisruption",
    "offensiveKnockdown",
    "offensiveTaunt",
    "offensiveFumble",
    "offensiveProjectileFumble",
    "offensiveTotalDamageReductionPercent",
    "offensiveTotalDamageReductionAbsolute",
    "offensiveTotalResistanceReductionPercent",
    "offensiveTotalResistanceReductionAbsolute",
    "offensiveElementalReductionPercent",
    "offensiveElementalResistanceReductionPercent",
    "offensiveElementalResistanceReductionAbsolute",
    "offensivePhysicalReductionPercent",
    "offensivePhysicalResistanceReductionPercent",
    "offensivePhysicalResistanceReductionAbsolute",
];

const OFFENSE_DURATION: &[&str] = &[
    "offensiveSlowPhysical",
    "offensiveSlowBleeding",
    "offensiveSlowFire",
    "offensiveSlowCold",
    "offensiveSlowLightning",
    "offensiveSlowPoison",
    "offensiveSlowLife",
    "offensiveSlowElemental",
    "offensiveSlowLifeLeach",
    "offensiveSlowManaLeach",
    "offensiveSlowTotalSpeed",
    "offensiveSlowAttackSpeed",
    "offensiveSlowRunSpeed",
    "offensiveSlowSpellCastSpeed",
    "offensiveSlowOffensiveAbility",
    "offensiveSlowDefensiveAbility",
    "offensiveSlowOffensiveReduction",
    "offensiveSlowDefensiveReduction",
];

const RETALIATION: &[&str] = &[
    "retaliationPhysical",
    "retaliationPierceRatio",
    "retaliationPierce",
    "retaliationFire",
    "retaliationCold",
    "retaliationLightning",
    "retaliationPoison",
    "retaliationLife",
    "retaliationAether",
    "retaliationChaos",
    "retaliationElemental",
    "retaliationTotalDamage",
    "retaliationDamagePct",
    "retaliationPercentCurrentLife",
    "retaliationStun",
    "retaliationFreeze",
    "retaliationPetrify",
    "retaliationTrap",
    "retaliationConfusion",
    "retaliationConvert",
    "retaliationFear",
    "retaliationSleep",
    "retaliationKnockdown",
];

const RETALIATION_DURATION: &[&str] = &[
    "retaliationSlowPhysical",
    "retaliationSlowBleeding",
    "retaliationSlowFire",
    "retaliationSlowCold",
    "retaliationSlowLightning",
    "retaliationSlowPoison",
    "retaliationSlowLife",
    "retaliationSlowLifeLeach",
    "retaliationSlowManaLeach",
    "retaliationSlowAttackSpeed",
    "retaliationSlowRunSpeed",
    "retaliationSlowSpellCastSpeed",
    "retaliationSlowOffensiveAbility",
    "retaliationSlowDefensiveAbility",
    "retaliationSlowOffensiveReduction",
    "retaliationSlowDefensiveReduction",
];

const DEFENSE: &[&str] = &[
    "defensiveProtection",
    "defensiveBonusProtection",
    "defensiveAbsorption",
    "defensiveBlock",
    "defensiveBlockAmount",
    "defensivePhysical",
    "defensivePierce",
    "defensiveFire",
    "defensiveCold",
    "defensiveLightning",
    "defensivePoison",
    "defensiveLife",
    "defensiveAether",
    "defensiveChaos",
    "defensiveElementalResistance",
    "defensiveElemental",
    "defensiveAllResistance",
    "defensiveAll",
    "defensiveBleeding",
    "defensiveSlowLifeLeach",
    "defensiveSlowManaLeach",
    "defensivePercentCurrentLife",
    "defensiveManaBurnRatio",
    "defensiveDisruption",
    "defensiveReflect",
    "defensivePercentReflectionResistance",
    "defensiveStun",
    "defensiveFreeze",
    "defensivePetrify",
    "defensiveTrap",
    "defensiveKnockdown",
    "defensiveSleep",
    "defensiveConfusion",
    "defensiveConvert",
    "defensiveFear",
    "defensiveTaunt",
    "defensiveCrowdControl",
    "defensiveTotalSpeedResistance",
];

const CHARACTER: &[&str] = &[
    "characterStrength",
    "characterDexterity",
    "characterIntelligence",
    "characterLife",
    "characterMana",
    "characterOffensiveAbility",
    "characterDefensiveAbility",
    "characterAttackSpeed",
    "characterAttackSpeedMax",
    "characterSpellCastSpeed",
    "characterSpellCastSpeedMax",
    "characterRunSpeed",
    "characterRunSpeedMax",
    "characterTotalSpeed",
    "characterLifeRegen",
    "characterManaRegen",
    "characterHealIncreasePercent",
    "characterConstitution",
    "characterEnergyAbsorptionPercent",
    "characterManaLimitReserve",
    "characterManaLimitReserveReduction",
    "characterDodgePercent",
    "characterDeflectProjectile",
    "characterDefensiveBlockRecoveryReduction",
    "characterIncreasedExperience",
    "characterLightRadius",
    "characterGlobalReqReduction",
    "characterLevelReqReduction",
    "characterArmorStrengthReqReduction",
    "characterArmorDexterityReqReduction",
    "characterArmorIntelligenceReqReduction",
    "characterWeaponStrengthReqReduction",
    "characterWeaponDexterityReqReduction",
    "characterWeaponIntelligenceReqReduction",
    "characterWeapon2HStrengthReqReduction",
    "characterWeapon2HDexterityReqReduction",
    "characterWeapon2HIntelligenceReqReduction",
    "characterMeleeStrengthReqReduction",
    "characterMeleeDexterityReqReduction",
    "characterMeleeIntelligenceReqReduction",
    "characterHuntingStrengthReqReduction",
    "characterHuntingDexterityReqReduction",
    "characterHuntingIntelligenceReqReduction",
    "characterStaffStrengthReqReduction",
    "characterStaffDexterityReqReduction",
    "characterStaffIntelligenceReqReduction",
    "characterShieldStrengthReqReduction",
    "characterShieldDexterityReqReduction",
    "characterShieldIntelligenceReqReduction",
    "characterJewelryStrengthReqReduction",
    "characterJewelryDexterityReqReduction",
    "characterJewelryIntelligenceReqReduction",
];

const SKILL_STAT: &[&str] = &[
    "skillCooldownReduction",
    "skillManaCostReduction",
    "skillProjectileSpeedModifier",
    "skillLifeBonus",
    "skillLifePercent",
    "skillManaBonus",
    "skillManaPercent",
    "skillManaCost",
    "skillActiveDuration",
    "skillCooldownTime",
    "skillTargetRadius",
    "skillTargetNumber",
    "skillChanceWeight",
    "skillProjectileNumber",
    "projectileExplosionRadius",
    "projectileLaunchNumber",
    "projectileLaunchRotation",
    "projectilePiercingChance",
    "piercingProjectile",
    "projectilePiercing",
    "weaponDamagePct",
    "petLimit",
    "petBurstSpawn",
    "spawnObjectsTimeToLive",
    "damageAbsorption",
    "damageAbsorptionPercent",
    "onHitActivationChance",
    "lifeMonitorPercent",
    "cooldownCharges",
    "skillActiveLifeCost",
    "skillActiveManaCost",
];

/// Where a line sits in the tooltip, in display order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Section {
    /// Weapon damage, armor, block, speed: the white lines.
    Base,
    Offense,
    OffenseDuration,
    Retaliation,
    Defense,
    Character,
    Skill,
    Conversion,
    Racial,
    SkillBonus,
    GrantedSkill,
    SkillModifier,
    PetBonus,
    Set,
    Requirement,
}

/// How a line is coloured: the game's white base stats, its bonus
/// text, block headings, and muted descriptions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Emphasis {
    Base,
    Bonus,
    Heading,
    Muted,
}

/// Whether an effect's value line is one of the white base stats —
/// weapon and shield damage and armor piercing, a shield's block, an
/// armor piece's armor. The text archive strips the `{^S}` codes the
/// tags mark these with, so the rule lives here.
#[must_use]
pub fn is_base_value(effect: Effect, kind: RecordKind) -> bool {
    match effect.family {
        Family::Offense => match effect.stem {
            "offensivePhysical" | "offensivePierceRatio" => {
                matches!(kind, RecordKind::Weapon | RecordKind::Shield)
            }
            stem => stem.starts_with("offensiveBase"),
        },
        Family::Defense => match effect.stem {
            "defensiveProtection" => kind == RecordKind::Armor,
            "defensiveBlock" => kind == RecordKind::Shield,
            _ => false,
        },
        Family::OffenseDuration
        | Family::Retaliation
        | Family::RetaliationDuration
        | Family::Character
        | Family::SkillStat => false,
    }
}

/// The class of the record a stat sits on, as far as rendering cares:
/// weapons and shields show their damage white and their speed line,
/// armor shows its armor value white; nothing else does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordKind {
    Weapon,
    Shield,
    Armor,
    Other,
}

impl RecordKind {
    #[must_use]
    pub fn of_class(class: &str) -> Self {
        let starts_with = |prefix: &str| {
            class.len() >= prefix.len() && class[..prefix.len()].eq_ignore_ascii_case(prefix)
        };
        if class.eq_ignore_ascii_case("WeaponArmor_Shield") {
            Self::Shield
        } else if starts_with("Weapon") && !class.eq_ignore_ascii_case("WeaponArmor_Offhand") {
            Self::Weapon
        } else if starts_with("ArmorProtective") {
            Self::Armor
        } else {
            Self::Other
        }
    }
}

/// The tag naming a flat, duration, or defense/character value part.
/// `None` when the game has no such line for that part of that
/// family (a defense `Chance` alone, a `Global` flag).
#[must_use]
pub fn value_tag(effect: Effect, kind: RecordKind) -> Option<String> {
    let tail = effect.tail();
    Some(match effect.family {
        Family::Offense => match effect.stem {
            "offensivePhysical" => match kind {
                RecordKind::Weapon | RecordKind::Shield => "DamageBasePhysical".to_string(),
                RecordKind::Armor | RecordKind::Other => "DamagePhysical".to_string(),
            },
            "offensivePierceRatio" => "DamageBasePierceRatio".to_string(),
            "offensiveManaBurn" => "DamageManaDrain".to_string(),
            "offensiveSleep" => "tagDamageSleep".to_string(),
            "offensiveFumble" => "DamageDurationFumble".to_string(),
            "offensiveProjectileFumble" => "DamageDurationProjectileFumble".to_string(),
            "offensiveTotalDamage" | "offensiveCritDamage" | "offensiveDamageMult" => return None,
            "offensiveBaseLife" => "tagDamageBaseVitality".to_string(),
            stem if stem.starts_with("offensiveBase") => format!("tagDamageBase{}", &stem[13..]),
            _ => format!("Damage{tail}"),
        },
        Family::OffenseDuration => format!("DamageDuration{tail}"),
        Family::Retaliation => match effect.stem {
            "retaliationTotalDamage" | "retaliationDamagePct" | "retaliationPierceRatio" => {
                return None;
            }
            _ => format!("Retaliation{tail}"),
        },
        Family::RetaliationDuration => format!("RetaliationDuration{tail}"),
        Family::Defense => match effect.stem {
            "defensiveProtection" => "DefenseAbsorptionProtection".to_string(),
            "defensiveBonusProtection" => "DefenseAbsorptionProtectionPlus".to_string(),
            "defensiveAbsorption"
            | "defensiveBlockAmount"
            | "defensiveElemental"
            | "defensiveAll" => return None,
            "defensiveSlowLifeLeach" => "DefenseLifeLeach".to_string(),
            "defensiveSlowManaLeach" => "DefenseManaLeach".to_string(),
            "defensiveTotalSpeedResistance" => "tagTotalSpeedResistance".to_string(),
            "defensivePercentReflectionResistance" => "DefenseReflectResist".to_string(),
            "defensiveSleep" => "tagDefenseSleep".to_string(),
            _ => format!("Defense{tail}"),
        },
        Family::Character => character_tag(effect.stem),
        Family::SkillStat => return None,
    })
}

/// The tag naming a `Modifier` part.
#[must_use]
pub fn modifier_tag(effect: Effect) -> Option<String> {
    let tail = effect.tail();
    Some(match effect.family {
        Family::Offense => match effect.stem {
            "offensiveTotalDamage" => "tagDamageModifierTotalDamage".to_string(),
            "offensiveCritDamage" => "tagDamageModifierCritDamage".to_string(),
            "offensiveDamageMult" => "tagDamageModifierDamageMult".to_string(),
            "offensiveSleep" => "tagDamageModifierSleep".to_string(),
            "offensiveManaBurn" => "DamageModifierManaBurn".to_string(),
            _ => format!("DamageModifier{tail}"),
        },
        Family::OffenseDuration => format!("DamageDurationModifier{tail}"),
        Family::Retaliation => match effect.stem {
            "retaliationTotalDamage" => "tagRetaliationModifierTotalDamage".to_string(),
            "retaliationDamagePct" => "tagRetaliationModifierDamageMult".to_string(),
            _ => format!("RetaliationModifier{tail}"),
        },
        Family::RetaliationDuration => format!("RetaliationDurationModifier{tail}"),
        Family::Defense => match effect.stem {
            "defensiveProtection" | "defensiveBonusProtection" => {
                "DefenseProtectionModifier".to_string()
            }
            "defensiveSlowLifeLeach" => "DefenseLifeLeachModifier".to_string(),
            "defensiveSlowManaLeach" => "DefenseManaLeachModifier".to_string(),
            "defensiveSleep" => "tagDefenseSleepModifier".to_string(),
            "defensivePercentCurrentLife" => "DefensePercentLifeModifier".to_string(),
            _ => format!("Defense{tail}Modifier"),
        },
        Family::Character => format!("{}Modifier", character_tag(effect.stem)),
        Family::SkillStat => match effect.stem {
            "skillCooldownReduction" => "SkillCooldownReductionModifier".to_string(),
            "skillManaCostReduction" => "SkillManaCostReductionModifier".to_string(),
            _ => return None,
        },
    })
}

/// The tag of a defense `Duration` / `DurationModifier` / `MaxResist`
/// part ("Reduction in Burn Duration", "Max Fire Resistance").
#[must_use]
pub fn defense_part_tag(effect: Effect, part: Part) -> Option<String> {
    if effect.family != Family::Defense {
        return None;
    }
    let base = match effect.stem {
        "defensiveSlowLifeLeach" => "DefenseLifeLeach".to_string(),
        "defensiveSlowManaLeach" => "DefenseManaLeach".to_string(),
        "defensiveSleep" => "tagDefenseSleep".to_string(),
        _ => format!("Defense{}", effect.tail()),
    };
    let suffix = match part {
        Part::Duration => "Duration",
        Part::DurationModifier => "DurationModifier",
        Part::MaxResist => "MaxResist",
        Part::Value
        | Part::Min
        | Part::Max
        | Part::Chance
        | Part::Global
        | Part::Xor
        | Part::DurationMin
        | Part::DurationMax
        | Part::DurationChance
        | Part::Modifier
        | Part::ModifierChance
        | Part::DurationModifierChance
        | Part::DrainMin
        | Part::DrainMax
        | Part::DamageRatio => return None,
    };
    Some(format!("{base}{suffix}"))
}

/// The game names its character tags after the attribute's slot in
/// the character sheet rather than the variable.
fn character_tag(stem: &str) -> String {
    match stem {
        "characterDexterity" => "tagCharAttribute01".to_string(),
        "characterStrength" => "tagCharAttribute02".to_string(),
        "characterIntelligence" => "tagCharAttribute03".to_string(),
        "characterLife" => "tagCharAttribute04".to_string(),
        "characterMana" => "tagCharAttribute05".to_string(),
        "characterDeflectProjectile" => "tagCharDeflectProjectiles".to_string(),
        "characterGlobalReqReduction" => "tagCharItemGlobalReduction".to_string(),
        "characterHealIncreasePercent" => "tagCharPercentHealIncreaseModifier".to_string(),
        _ => format!("tagChar{}", &stem["character".len()..]),
    }
}

/// How a skill stat's number and label combine: the label tag either
/// carries its own placeholder or is a bare noun the game wraps in one
/// of its `Skill*Format` tags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkillStatShape {
    /// The tag formats the value itself (`SkillPetLimit={%d0} …`).
    Direct(&'static str),
    /// `SkillIntFormat` around a noun tag ("60 Energy Cost").
    Count(&'static str),
    /// `SkillSecondFormat` around a noun tag ("2.0 Second Skill Recharge").
    Seconds(&'static str),
    /// `SkillDistanceFormat` around a noun tag ("4.0 Meter Target Area").
    Meters(&'static str),
    /// A percentage before a noun tag ("100% Chance of Activating").
    Percent(&'static str),
}

#[must_use]
pub fn skill_stat_shape(effect: Effect) -> Option<SkillStatShape> {
    if effect.family != Family::SkillStat {
        return None;
    }
    Some(match effect.stem {
        "skillCooldownReduction" => SkillStatShape::Direct("SkillCooldownReduction"),
        "skillManaCostReduction" => SkillStatShape::Direct("SkillManaCostReduction"),
        "skillProjectileSpeedModifier" => SkillStatShape::Direct("SkillProjectileSpeedModifier"),
        "skillLifeBonus" => SkillStatShape::Direct("SkillLifeBonus"),
        "skillLifePercent" => SkillStatShape::Direct("SkillLifePercent"),
        "skillManaBonus" => SkillStatShape::Direct("SkillManaBonus"),
        "skillManaPercent" => SkillStatShape::Direct("SkillManaPercent"),
        "skillManaCost" => SkillStatShape::Count("ManaCost"),
        "skillActiveDuration" => SkillStatShape::Seconds("ActiveDuration"),
        "skillCooldownTime" => SkillStatShape::Seconds("CooldownTime"),
        "skillTargetRadius" => SkillStatShape::Meters("TargetRadius"),
        "projectileExplosionRadius" => SkillStatShape::Meters("ExplosionRadius"),
        "skillTargetNumber" => SkillStatShape::Direct("TargetNumber"),
        "skillChanceWeight" => SkillStatShape::Direct("SkillChanceWeight"),
        "skillProjectileNumber" => SkillStatShape::Direct("SkillNumProjectilesFormat"),
        "projectileLaunchNumber" => SkillStatShape::Direct("ProjectileLaunchNumber"),
        "projectileLaunchRotation" => SkillStatShape::Direct("ProjectileLaunchRotation"),
        "projectilePiercingChance" | "piercingProjectile" | "projectilePiercing" => {
            SkillStatShape::Direct("ProjectilePiercingChance")
        }
        "weaponDamagePct" => SkillStatShape::Direct("SkillWeaponDamageFormat"),
        "petLimit" => SkillStatShape::Direct("SkillPetLimit"),
        "petBurstSpawn" => SkillStatShape::Direct("SkillPetBurstSpawn"),
        "spawnObjectsTimeToLive" => SkillStatShape::Direct("tagSkillPetTimeToLive"),
        "damageAbsorption" => SkillStatShape::Direct("SkillDamageAbsorption"),
        "damageAbsorptionPercent" => SkillStatShape::Direct("SkillDamageAbsorptionPercent"),
        "onHitActivationChance" => SkillStatShape::Percent("SkillActivationChance"),
        "lifeMonitorPercent" => SkillStatShape::Direct("LifeMonitorPercent"),
        "cooldownCharges" => SkillStatShape::Direct("CooldownCharges"),
        "skillActiveLifeCost" => SkillStatShape::Count("ActiveLifeCost"),
        "skillActiveManaCost" => SkillStatShape::Count("ActiveManaCost"),
        _ => return None,
    })
}

/// Effects whose `Min` is a duration in seconds ("Stun target for
/// 1.5 Seconds"), formatted with the fixed-time tags.
#[must_use]
pub fn is_influence(effect: Effect) -> bool {
    const INFLUENCES: [&str; 11] = [
        "Stun",
        "Freeze",
        "Petrify",
        "Trap",
        "Confusion",
        "Convert",
        "Fear",
        "Sleep",
        "Disruption",
        "Knockdown",
        "Taunt",
    ];
    matches!(effect.family, Family::Offense | Family::Retaliation)
        && INFLUENCES.contains(&effect.tail())
}

/// Effects whose amount is a total dealt over the duration ("150
/// Bleeding Damage over 3.0 Seconds": the record's per-second value
/// times the duration). Everything else with a duration lasts "for N
/// Seconds" at its face value.
#[must_use]
pub fn scales_with_duration(effect: Effect) -> bool {
    const FIXED: [&str; 11] = [
        "TotalSpeed",
        "AttackSpeed",
        "RunSpeed",
        "SpellCastSpeed",
        "OffensiveAbility",
        "DefensiveAbility",
        "OffensiveReduction",
        "DefensiveReduction",
        "Fumble",
        "ProjectileFumble",
        "Elemental",
    ];
    match effect.family {
        Family::OffenseDuration | Family::RetaliationDuration => !FIXED.contains(&effect.tail()),
        Family::Offense
        | Family::Retaliation
        | Family::Defense
        | Family::Character
        | Family::SkillStat => false,
    }
}

/// Effects an item's `attributeScalePercent` multiplies (its own
/// values and its affixes'): flat and percent offense other than the
/// base weapon damage, pierce ratio, leech and crit. Recorded from GD
/// Stash's scaling categories (eyes-only); unverified in-game.
#[must_use]
pub fn scales_with_item(effect: Effect) -> bool {
    match effect.family {
        Family::Offense => {
            !matches!(
                effect.stem,
                "offensivePhysical"
                    | "offensivePierceRatio"
                    | "offensiveLifeLeech"
                    | "offensiveCritDamage"
                    | "offensivePercentCurrentLife"
                    | "offensiveDamageMult"
                    | "offensiveManaBurn"
            ) && !is_influence(effect)
                && !effect.stem.starts_with("offensiveBase")
        }
        Family::OffenseDuration => true,
        Family::Retaliation => effect.stem == "retaliationDamagePct",
        Family::SkillStat => matches!(effect.stem, "weaponDamagePct" | "damageAbsorptionPercent"),
        Family::RetaliationDuration | Family::Defense | Family::Character => false,
    }
}

/// Numeric variables the game never displays as stats: bookkeeping,
/// costs, mesh and physics parameters, slot flags, and the inputs
/// other lines consume.
const HIDDEN: &[&str] = &[
    "itemLevel",
    "itemCost",
    "itemCostScalePercent",
    "levelRequirement",
    "strengthRequirement",
    "dexterityRequirement",
    "intelligenceRequirement",
    "lootRandomizerCost",
    "lootRandomizerJitter",
    "lootRandomizerScale",
    "marketAdjustmentPercent",
    "attributeScalePercent",
    "characterBaseAttackSpeed",
    "characterRunSpeedJitter",
    "completedRelicLevel",
    "maxTransparency",
    "outlineThickness",
    "scale",
    "castsShadows",
    "craftingMaterial",
    "soulbound",
    "itemSkillLevel",
    "petBonusLevel",
    "augmentAllLevel",
    "skillMaxLevel",
    "skillTier",
    "skillUltimateLevel",
    "skillMasteryLevelRequired",
    "cameraShakeAmplitude",
    "cameraShakeDurationSecs",
    "expansionTime",
    "ragDollAmplification",
    "instantCast",
    "exclusiveSkill",
    "hidePrefixName",
    "hideSuffixName",
    "cannotPickUpMultiple",
    "maxStackSize",
    "blockAbsorption",
    "blockPathing",
    "blockRecoveryTime",
    "levelOffset",
    "intactEffect",
    "artifactCreateQuantity",
    "reagentBaseQuantity",
    "overwriteBaseSkill",
    "isPetBonusScaling",
    "preventEasyDrops",
    "preventMaleTransmute",
    "preventFemaleTransmute",
    "requiredTaskUID",
    "markerRange",
    "untradeable",
    "notDispelable",
    "forcedRelicCompletion",
    "launchAboveTarget",
    "boostedMultiplier",
    "showQuestItemTag",
    "useTargetDir",
    "targetFxFirstOnly",
    "thresholdDuration",
    "skillComboChargeDuration",
    "skillComboChargeLevel",
    "dualWieldOnly",
    "experienceBonus",
    "maxMoveRatio",
    "noteWidth",
    "explosionRadius",
    "tailVelocity",
    "headVelocity",
    "removeTransmute",
    "roundBitmap",
    "petPadding",
    "pointBlank",
    "dropRadius",
    "dropVariation",
    "skillTargetAngle",
    "skillConnectionSpacing",
    "skillProjectileTargetGroundOnly",
    "skillAllowsWarmup",
    "skillLifeCostPerSecond",
    "skillLifePercentBuffDuration",
    "projectileUsesAllDamage",
    "isPetDisplayable",
    "petBurstSpawnLimit",
    "spawnObjectsDistanceIncrement",
    "spawnObjectsDistanceInnerCircle",
    "spawnObjectsNumberOfRings",
    "spawnObjectsSpacingAngle",
    "spawnObjectsRandomRotation",
    "refreshCooldownChance",
    "refreshCooldownAmount",
    "refreshCooldownMax",
    "refreshDurationChance",
    "refreshDurationAmount",
    "refreshDurationMax",
    "waveEndWidth",
    "waveDistance",
    "waveDepth",
    "waveStartWidth",
    "waveTime",
    "sparkGap",
    "sparkChance",
    "sparkMaxNumber",
    "dropOffset",
    "dropHeight",
    "numProjectiles",
    "bonusLifePercent",
    "bonusLifePoints",
    "bonusManaPercent",
    "bonusManaPoints",
    "displayAsQuestItem",
    "quest",
    "actorScale",
    "actorScaleTime",
    "forceIgnoreRunSpeedCaps",
    "noHighlightDefaultColorA",
    "debufSkill",
    "hideFromUI",
    "useDelayTime",
    "decrementStatType",
    "allSkillEnhancement",
    "skillWeaponTintRed",
    "skillWeaponTintGreen",
    "skillWeaponTintBlue",
    "skillComboChargeSpendReduction",
    "skillCooldownReductionChance",
    "racialBonusPercentDamage",
    "racialBonusPercentDefense",
    "racialBonusAbsoluteDamage",
    "racialBonusAbsoluteDefense",
    "conversionPercentage",
    "conversionPercentage2",
    "offensiveGlobalChance",
    "retaliationGlobalChance",
];
const HIDDEN_PREFIXES: &[&str] = &[
    "actor",
    "physics",
    "augmentSkillLevel",
    "augmentMasteryLevel",
    "randomizer",
    "reagent",
    "taskUID",
    "fxPak",
    "projectileFragments",
    "projectileDamageRange",
    "waist",
    "dissolve",
    "distress",
    "dHanded",
    "dualRanged",
    "dualMelee",
    "singleMelee",
    "singleRanged",
    "twoHanded",
    "unarmed",
    "shield",
    "staff",
    "ranged",
    "offhand",
    "amulet",
    "ring",
    "medal",
    "belt",
    "head",
    "shoulders",
    "chest",
    "hands",
    "legs",
    "feet",
    "sword",
    "axe",
    "mace",
    "dagger",
    "scepter",
    "spear",
    "characterAttributeEquations",
];

/// Whether a numeric variable is one of the [`HIDDEN`] bookkeeping
/// values or matches a hidden prefix or suffix.
#[must_use]
pub fn is_hidden(name: &str) -> bool {
    HIDDEN
        .iter()
        .any(|hidden| hidden.eq_ignore_ascii_case(name))
        || HIDDEN_PREFIXES.iter().any(|prefix| {
            name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
        })
        || name.ends_with("AnimSpeed")
        || name.ends_with("AnimWeight")
        || name.ends_with("BlendTime")
        || name.ends_with("DamageQualifier")
        || name.ends_with("DamageQualifi")
}

/// Fixed format tags the renderer composes lines with.
pub mod tags {
    pub const SINGLE: &str = "DamageSingleFormat";
    pub const RANGE: &str = "DamageRangeFormat";
    pub const OVER_SINGLE: &str = "DamageSingleFormatTime";
    pub const OVER_RANGE: &str = "DamageRangeFormatTime";
    pub const FOR_SINGLE: &str = "DamageFixedSingleFormatTime";
    pub const FOR_RANGE: &str = "DamageFixedRangeFormatTime";
    pub const RETALIATION_FOR_SINGLE: &str = "RetaliationFixedSingleFormatTime";
    pub const RETALIATION_FOR_RANGE: &str = "RetaliationFixedRangeFormatTime";
    pub const CHANCE_OF: &str = "tagChanceOf";
    pub const IMPROVED_DURATION: &str = "ImprovedTimeFormat";
    pub const MANA_BURN_RATIO: &str = "DamageManaBurnRatio";
    pub const GLOBAL_ALL: &str = "GlobalPercentChanceOfAllTag";
    pub const GLOBAL_ONE: &str = "GlobalPercentChanceOfOneTag";
    pub const BLOCK_CHANCE: &str = "tagCharStatsBlockChance";
    pub const BLOCK_RECOVERY: &str = "ShieldBlockRecoveryTime";
    pub const SKILL_COUNT: &str = "SkillIntFormat";
    pub const SKILL_SECONDS: &str = "SkillSecondFormat";
    pub const SKILL_METERS: &str = "SkillDistanceFormat";
    pub const SKILL_BONUS: &str = "ItemSkillIncrement";
    pub const MASTERY_BONUS: &str = "ItemMasteryIncrement";
    pub const ALL_SKILLS_BONUS: &str = "ItemAllSkillIncrement";
    pub const GRANTS_SKILL: &str = "tagItemGrantSkill";
    pub const SKILL_LEVEL: &str = "MenuLevel";
    pub const MODIFIED_SKILL: &str = "tagItemSkillModified";
    pub const PET_BONUS: &str = "tagPetBonusNameAllPets";
    pub const CONVERSION: &str = "tagDamageConversion";
    pub const RACIAL_PERCENT_DAMAGE: &str = "RacialBonusPercentDamage";
    pub const RACIAL_PERCENT_DEFENSE: &str = "RacialBonusPercentDefense";
    pub const RACIAL_ABSOLUTE_DAMAGE: &str = "RacialBonusAbsoluteDamage";
    pub const RACIAL_ABSOLUTE_DEFENSE: &str = "RacialBonusAbsoluteDefense";
    pub const REQUIREMENT: &str = "MeetsRequirement";
    pub const ASCENDED_HEADING: &str = "tagTooltipAscensionHeader";
}

/// The autocast condition tag for a controller's `triggerType`.
#[must_use]
pub fn autocast_tag(trigger: &str) -> Option<&'static str> {
    Some(match trigger.to_ascii_lowercase().as_str() {
        "lowhealth" => "tagAutoSkillCondition01",
        "lowmana" => "tagAutoSkillCondition02",
        "hitbyenemy" => "tagAutoSkillCondition03",
        "hitbymelee" => "tagAutoSkillCondition04",
        "hitbyprojectile" => "tagAutoSkillCondition05",
        "castbuff" => "tagAutoSkillCondition06",
        "attackenemy" => "tagAutoSkillCondition07",
        "onequip" => "tagAutoSkillCondition08",
        "hitbycrit" => "tagAutoSkillCondition09",
        "attackenemycrit" => "tagAutoSkillCondition10",
        "block" => "tagAutoSkillCondition11",
        "onkill" => "tagAutoSkillCondition12",
        _ => return None,
    })
}

/// The tag naming a damage type in a conversion line
/// (`conversionInType = "Life"` → `tagConversionLife` → "Vitality Damage").
#[must_use]
pub fn conversion_tag(damage_type: &str) -> String {
    format!("tagConversion{damage_type}")
}

/// The tag naming a race (`racialBonusRace = "Race007"` → `tagRace007`).
#[must_use]
pub fn race_tag(race: &str) -> String {
    format!("tag{race}")
}

/// The requirement equation prefix for an item class
/// (`WeaponMelee_Sword` → `sword`, so `swordDexterityEquation`).
#[must_use]
pub fn requirement_equation_prefix(class: &str) -> Option<&'static str> {
    const PREFIXES: [(&str, &str); 21] = [
        ("ArmorProtective_Head", "head"),
        ("ArmorProtective_Shoulders", "shoulders"),
        ("ArmorProtective_Chest", "chest"),
        ("ArmorProtective_Legs", "legs"),
        ("ArmorProtective_Feet", "feet"),
        ("ArmorProtective_Hands", "hands"),
        ("ArmorProtective_Waist", "waist"),
        ("ArmorJewelry_Ring", "ring"),
        ("ArmorJewelry_Amulet", "amulet"),
        ("WeaponMelee_Axe", "axe"),
        ("WeaponMelee_Mace", "mace"),
        ("WeaponMelee_Sword", "sword"),
        ("WeaponMelee_Dagger", "dagger"),
        ("WeaponMelee_Scepter", "scepter"),
        ("WeaponMelee_Axe2h", "melee2h"),
        ("WeaponMelee_Mace2h", "melee2h"),
        ("WeaponMelee_Sword2h", "melee2h"),
        ("WeaponMelee_Spear2h", "melee2h"),
        ("WeaponHunting_Ranged1h", "ranged1h"),
        ("WeaponHunting_Ranged2h", "ranged2h"),
        ("WeaponArmor_Shield", "shield"),
    ];
    if class.eq_ignore_ascii_case("WeaponArmor_Offhand") {
        return Some("offhand");
    }
    PREFIXES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(class))
        .map(|(_, prefix)| *prefix)
}

/// The record every item without an `itemCostName` takes its
/// requirement equations from.
pub const DEFAULT_COST_RECORD: &str = "records/game/itemcostformulas.dbr";

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute(name: &str) -> Attribute {
        parse_variable(name).unwrap_or_else(|| panic!("{name} should parse"))
    }

    #[test]
    fn variables_split_into_family_stem_and_part() {
        let bleeding = attribute("offensiveSlowBleedingDurationMin");
        assert_eq!(bleeding.effect.family(), Family::OffenseDuration);
        assert_eq!(bleeding.effect.name(), "offensiveSlowBleeding");
        assert_eq!(bleeding.part, Part::DurationMin);

        let fire = attribute("offensiveFireModifierChance");
        assert_eq!(fire.effect.name(), "offensiveFire");
        assert_eq!(fire.part, Part::ModifierChance);

        let resist = attribute("defensiveFire");
        assert_eq!(resist.effect.family(), Family::Defense);
        assert_eq!(resist.part, Part::Value);
        assert_eq!(
            attribute("defensiveFireDuration").part,
            Part::Duration,
            "a defense Duration is a value, not an effect length"
        );
        assert_eq!(attribute("defensiveAetherMaxResist").part, Part::MaxResist);

        assert_eq!(
            attribute("characterLifeRegenModifier").effect.name(),
            "characterLifeRegen"
        );
        assert_eq!(
            attribute("characterManaLimitReserveReductionModifier")
                .effect
                .name(),
            "characterManaLimitReserveReduction"
        );
        assert_eq!(
            attribute("skillManaCost").effect.family(),
            Family::SkillStat
        );
    }

    #[test]
    fn parsing_is_case_insensitive_and_rejects_unknown_stems() {
        assert_eq!(
            attribute("retaliationPercentcurrentLifeGlobal")
                .effect
                .name(),
            "retaliationPercentCurrentLife"
        );
        assert_eq!(parse_variable("offensiveWhimsyMin"), None);
        assert_eq!(parse_variable("characterBaseAttackSpeed"), None);
        assert_eq!(parse_variable("itemLevel"), None);
    }

    #[test]
    fn tags_follow_the_games_naming() {
        let tag = |name: &str, kind| value_tag(attribute(name).effect, kind);
        assert_eq!(
            tag("offensivePhysicalMin", RecordKind::Weapon).as_deref(),
            Some("DamageBasePhysical")
        );
        assert_eq!(
            tag("offensivePhysicalMin", RecordKind::Other).as_deref(),
            Some("DamagePhysical")
        );
        assert_eq!(
            tag("offensiveBaseLifeMin", RecordKind::Shield).as_deref(),
            Some("tagDamageBaseVitality")
        );
        assert_eq!(
            tag("offensiveSlowColdMin", RecordKind::Other).as_deref(),
            Some("DamageDurationCold")
        );
        assert_eq!(
            tag("retaliationSlowBleedingMin", RecordKind::Other).as_deref(),
            Some("RetaliationDurationBleeding")
        );
        assert_eq!(
            tag("defensiveSlowLifeLeach", RecordKind::Other).as_deref(),
            Some("DefenseLifeLeach")
        );
        assert_eq!(
            tag("characterStrength", RecordKind::Other).as_deref(),
            Some("tagCharAttribute02")
        );
        assert_eq!(tag("offensiveTotalDamageModifier", RecordKind::Other), None);

        let modifier = |name: &str| modifier_tag(attribute(name).effect);
        assert_eq!(
            modifier("offensiveTotalDamageModifier").as_deref(),
            Some("tagDamageModifierTotalDamage")
        );
        assert_eq!(
            modifier("offensiveSlowFireModifier").as_deref(),
            Some("DamageDurationModifierFire")
        );
        assert_eq!(
            modifier("defensiveFireModifier").as_deref(),
            Some("DefenseFireModifier")
        );
        assert_eq!(
            modifier("characterOffensiveAbilityModifier").as_deref(),
            Some("tagCharOffensiveAbilityModifier")
        );
        assert_eq!(
            defense_part_tag(attribute("defensiveFireDuration").effect, Part::Duration).as_deref(),
            Some("DefenseFireDuration")
        );
    }

    #[test]
    fn durations_split_into_totals_and_fixed_lengths() {
        assert!(scales_with_duration(
            attribute("offensiveSlowBleedingMin").effect
        ));
        assert!(!scales_with_duration(
            attribute("offensiveSlowRunSpeedMin").effect
        ));
        assert!(!scales_with_duration(
            attribute("offensiveTotalResistanceReductionPercentMin").effect
        ));
        assert!(is_influence(attribute("offensiveStunMin").effect));
        assert!(!is_influence(attribute("offensiveFireMin").effect));
    }

    #[test]
    fn item_scale_skips_base_damage_and_defenses() {
        assert!(scales_with_item(attribute("offensiveFireMin").effect));
        assert!(scales_with_item(
            attribute("offensiveSlowBleedingModifier").effect
        ));
        assert!(!scales_with_item(attribute("offensivePhysicalMin").effect));
        assert!(!scales_with_item(attribute("offensiveBaseFireMin").effect));
        assert!(!scales_with_item(attribute("defensiveFire").effect));
        assert!(!scales_with_item(attribute("characterLife").effect));
    }

    #[test]
    fn record_kinds_separate_weapons_shields_and_the_rest() {
        assert_eq!(
            RecordKind::of_class("WeaponMelee_Sword"),
            RecordKind::Weapon
        );
        assert_eq!(
            RecordKind::of_class("WeaponArmor_Shield"),
            RecordKind::Shield
        );
        assert_eq!(
            RecordKind::of_class("WeaponArmor_Offhand"),
            RecordKind::Other
        );
        assert_eq!(
            RecordKind::of_class("ArmorProtective_Chest"),
            RecordKind::Armor
        );
        assert_eq!(RecordKind::of_class("ArmorJewelry_Ring"), RecordKind::Other);
        assert_eq!(RecordKind::of_class("LootRandomizer"), RecordKind::Other);
    }

    #[test]
    fn base_values_are_weapon_damage_block_and_armor() {
        let base = |name: &str, kind| is_base_value(attribute(name).effect, kind);
        assert!(base("offensivePhysicalMin", RecordKind::Weapon));
        assert!(!base("offensivePhysicalMin", RecordKind::Other));
        assert!(base("offensiveBaseFireMin", RecordKind::Other));
        assert!(!base("offensiveFireMin", RecordKind::Weapon));
        assert!(base("defensiveProtection", RecordKind::Armor));
        assert!(!base("defensiveProtection", RecordKind::Weapon));
        assert!(base("defensiveBlock", RecordKind::Shield));
    }

    #[test]
    fn hidden_variables_cover_bookkeeping_and_slot_flags() {
        assert!(is_hidden("itemLevel"));
        assert!(is_hidden("axe2h"));
        assert!(is_hidden("dHandedAttackAnimSpeed1"));
        assert!(is_hidden("augmentSkillLevel3"));
        assert!(!is_hidden("offensiveFireMin"));
    }

    #[test]
    fn requirement_prefixes_cover_every_equipment_class() {
        assert_eq!(
            requirement_equation_prefix("WeaponMelee_Axe2h"),
            Some("melee2h")
        );
        assert_eq!(
            requirement_equation_prefix("ArmorProtective_Waist"),
            Some("waist")
        );
        assert_eq!(
            requirement_equation_prefix("WeaponArmor_Offhand"),
            Some("offhand")
        );
        assert_eq!(requirement_equation_prefix("ArmorJewelry_Medal"), None);
    }
}
