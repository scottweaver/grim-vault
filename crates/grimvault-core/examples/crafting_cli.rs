//! The blueprint and illusion loop, on a **copy** of a real save
//! directory: list a campaign's known blueprints and unlocked
//! illusions with their names and the database's verdict on each, add
//! one by record, export either list as this app's JSON document, and
//! import such a document into another campaign — every write
//! backup-first and re-parsed to prove it.
//!
//! ```text
//! crafting_cli [--mod NAME] [--game DIR] [--save DIR] blueprints
//! crafting_cli [--mod NAME] ... add-blueprint <record>
//! crafting_cli [--mod NAME] ... export-blueprints <file.json>
//! crafting_cli [--mod NAME] ... import-blueprints <file.json>
//! crafting_cli [--mod NAME] ... illusions
//! crafting_cli [--mod NAME] ... add-illusion <record>
//! crafting_cli [--mod NAME] ... export-illusions <file.json>
//! crafting_cli [--mod NAME] ... import-illusions <file.json>
//! ```
//!
//! `--mod NAME` works a mod's `save/<NAME>/formulas.gst` and
//! `transmutes.gst` instead of the main campaign's. Adds only: no
//! command removes an entry (ARCHITECTURE.md "Source of truth").

mod support;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use grimvault_core::blueprint::{
    self, BlueprintExport, add_blueprint, available_blueprints, check_blueprint, import_blueprints,
};
use grimvault_core::formulas::{FormulaRead, Formulas};
use grimvault_core::gamedata::GameData;
use grimvault_core::gst::{GstFile, Illusions};
use grimvault_core::illusion::{
    IllusionCategory, IllusionExport, add_illusion, audit, available_illusions, import_illusions,
};
use grimvault_core::item::Item;
use grimvault_core::loaded::Loaded;
use grimvault_core::store::Timestamp;
use univault_engine::ids::RecordId;
use univault_io::{BackupPolicy, backup_first_write, read_verified};

use support::{cli_paths, describe, load_game_data};

const BACKUPS: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);
const USAGE: &str = "usage: crafting_cli [--game DIR] [--save DIR] [--mod NAME] \
                     (blueprints | add-blueprint <record> | export-blueprints <file> \
                     | import-blueprints <file> | illusions | add-illusion <record> \
                     | export-illusions <file> | import-illusions <file>) \
                     — paths not given come from the app's saved settings";

enum Command {
    Blueprints,
    AddBlueprint(RecordId),
    ExportBlueprints(PathBuf),
    ImportBlueprints(PathBuf),
    Illusions,
    AddIllusion(RecordId),
    ExportIllusions(PathBuf),
    ImportIllusions(PathBuf),
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = cli_paths(&args)?;
    let command = parse_command(&paths.rest)?;
    let game = load_game_data(&paths.game_dir)?;
    let shared = paths.campaign.shared_dir(&paths.save_dir);
    let formulas_path = shared.join("formulas.gst");
    let transmutes_path = shared.join("transmutes.gst");
    let campaign = paths.campaign;
    match command {
        Command::Blueprints => {
            let formulas = load_formulas(&formulas_path)?;
            print_blueprints(&game, formulas.model());
        }
        Command::AddBlueprint(record) => {
            let mut formulas = load_formulas(&formulas_path)?;
            add_blueprint(formulas.model_mut(), &game, record.clone())?;
            println!("added blueprint {record}");
            write_formulas(&formulas_path, &formulas)?;
            print_blueprints(&game, &reparse_formulas(&formulas_path)?);
        }
        Command::ExportBlueprints(file) => {
            let formulas = load_formulas(&formulas_path)?;
            let export = BlueprintExport::of(formulas.model(), campaign, now()?);
            std::fs::write(&file, export.to_json())?;
            println!(
                "exported {} blueprints of the {} to {}",
                export.entries.len(),
                export.campaign,
                file.display()
            );
        }
        Command::ImportBlueprints(file) => {
            let export = BlueprintExport::from_json(&std::fs::read(&file)?)?;
            println!(
                "{}: {} blueprints exported from the {} at {}",
                file.display(),
                export.entries.len(),
                export.campaign,
                export.exported_at.unix_seconds()
            );
            let mut formulas = load_formulas(&formulas_path)?;
            let report = import_blueprints(formulas.model_mut(), &game, &export);
            println!("import into the {campaign}: {report}");
            if report.changed() {
                write_formulas(&formulas_path, &formulas)?;
                print_blueprints(&game, &reparse_formulas(&formulas_path)?);
            }
        }
        Command::Illusions => {
            let transmutes = load_transmutes(&transmutes_path)?;
            print_illusions(&game, illusions(transmutes.model())?);
        }
        Command::AddIllusion(record) => {
            let mut transmutes = load_transmutes(&transmutes_path)?;
            let category = add_illusion(
                illusions_mut(transmutes.model_mut())?,
                &game,
                record.clone(),
            )?;
            println!("added illusion {record} under {category}");
            write_transmutes(&transmutes_path, &transmutes)?;
            print_illusions(&game, illusions(&reparse_transmutes(&transmutes_path)?)?);
        }
        Command::ExportIllusions(file) => {
            let transmutes = load_transmutes(&transmutes_path)?;
            let export = IllusionExport::of(illusions(transmutes.model())?, campaign, now()?);
            std::fs::write(&file, export.to_json())?;
            println!(
                "exported {} illusions in {} slots of the {} to {}",
                export.total_count(),
                export.slots.len(),
                export.campaign,
                file.display()
            );
        }
        Command::ImportIllusions(file) => {
            let export = IllusionExport::from_json(&std::fs::read(&file)?)?;
            println!(
                "{}: {} illusions exported from the {} at {}",
                file.display(),
                export.total_count(),
                export.campaign,
                export.exported_at.unix_seconds()
            );
            let mut transmutes = load_transmutes(&transmutes_path)?;
            let report = import_illusions(illusions_mut(transmutes.model_mut())?, &game, &export);
            println!("import into the {campaign}: {report}");
            if report.changed() {
                write_transmutes(&transmutes_path, &transmutes)?;
                print_illusions(&game, illusions(&reparse_transmutes(&transmutes_path)?)?);
            }
        }
    }
    Ok(())
}

fn parse_command(rest: &[String]) -> Result<Command, Box<dyn Error>> {
    let record = |raw: &String| {
        RecordId::parse(raw.clone()).ok_or_else(|| Box::<dyn Error>::from("empty record path"))
    };
    Ok(match rest {
        [command] if command == "blueprints" => Command::Blueprints,
        [command, raw] if command == "add-blueprint" => Command::AddBlueprint(record(raw)?),
        [command, file] if command == "export-blueprints" => {
            Command::ExportBlueprints(PathBuf::from(file))
        }
        [command, file] if command == "import-blueprints" => {
            Command::ImportBlueprints(PathBuf::from(file))
        }
        [command] if command == "illusions" => Command::Illusions,
        [command, raw] if command == "add-illusion" => Command::AddIllusion(record(raw)?),
        [command, file] if command == "export-illusions" => {
            Command::ExportIllusions(PathBuf::from(file))
        }
        [command, file] if command == "import-illusions" => {
            Command::ImportIllusions(PathBuf::from(file))
        }
        _ => return Err(USAGE.into()),
    })
}

fn now() -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_unix_seconds(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    ))
}

fn load_formulas(path: &Path) -> Result<Loaded<Formulas>, Box<dyn Error>> {
    let loaded = Loaded::<Formulas>::load(read_verified(path)?)?;
    println!(
        "loaded {} ({} bytes, lossless; {} {} entries)",
        path.display(),
        loaded.baseline().len(),
        loaded.model().version,
        loaded.model().entries.len()
    );
    Ok(loaded)
}

fn load_transmutes(path: &Path) -> Result<Loaded<GstFile>, Box<dyn Error>> {
    let loaded = Loaded::<GstFile>::load(read_verified(path)?)?;
    println!(
        "loaded {} ({} bytes, lossless)",
        path.display(),
        loaded.baseline().len()
    );
    Ok(loaded)
}

fn illusions(file: &GstFile) -> Result<&Illusions, Box<dyn Error>> {
    file.illusions()
        .ok_or_else(|| "transmutes.gst carries no typed illusion collection (block 19)".into())
}

fn illusions_mut(file: &mut GstFile) -> Result<&mut Illusions, Box<dyn Error>> {
    file.illusions_mut()
        .ok_or_else(|| "transmutes.gst carries no typed illusion collection (block 19)".into())
}

fn write_formulas(path: &Path, formulas: &Loaded<Formulas>) -> Result<(), Box<dyn Error>> {
    let bytes = formulas.encode()?;
    let backup = backup_first_write(path, &bytes, BACKUPS)?;
    report_write(path, bytes.len(), backup.as_deref());
    Ok(())
}

fn write_transmutes(path: &Path, transmutes: &Loaded<GstFile>) -> Result<(), Box<dyn Error>> {
    let bytes = transmutes.encode()?;
    let backup = backup_first_write(path, &bytes, BACKUPS)?;
    report_write(path, bytes.len(), backup.as_deref());
    Ok(())
}

fn report_write(path: &Path, len: usize, backup: Option<&Path>) {
    match backup {
        Some(backup) => println!(
            "wrote {} ({len} bytes, verified); backup at {}",
            path.display(),
            backup.display()
        ),
        None => println!("wrote {} ({len} bytes, verified); new file", path.display()),
    }
}

fn reparse_formulas(path: &Path) -> Result<Formulas, Box<dyn Error>> {
    let bytes = read_verified(path)?;
    let formulas = Formulas::parse(&bytes)?;
    report_round_trip(path, formulas.encode()? == bytes);
    Ok(formulas)
}

fn reparse_transmutes(path: &Path) -> Result<GstFile, Box<dyn Error>> {
    let bytes = read_verified(path)?;
    let file = GstFile::parse(&bytes)?;
    report_round_trip(path, file.encode()? == bytes);
    Ok(file)
}

fn report_round_trip(path: &Path, identical: bool) {
    let verdict = if identical {
        "byte-identical"
    } else {
        "NOT byte-identical"
    };
    println!("re-parsed {} after write: {verdict}", path.display());
}

fn print_blueprints(game: &GameData, formulas: &Formulas) {
    println!(
        "\nblueprints: {} known (expansion status {})",
        formulas.entries.len(),
        formulas.expansion_status
    );
    let mut problems = 0;
    for (index, entry) in formulas.entries.iter().enumerate() {
        let item = Item {
            base_name: entry.record.clone(),
            ..Item::default()
        };
        let read = match entry.read {
            FormulaRead::Read => "read",
            FormulaRead::Unread => "NEW",
        };
        let verdict = match RecordId::parse(entry.record.clone()) {
            None => "PROBLEM empty record".to_string(),
            Some(id) => match check_blueprint(game, &id) {
                Ok(_) => blueprint::BLUEPRINT_CLASS.to_string(),
                Err(error) => {
                    problems += 1;
                    format!("PROBLEM {error}")
                }
            },
        };
        println!(
            "  [{index:>3}] {:<6} {} — {verdict}  {}",
            read,
            describe(game, &item),
            entry.record
        );
    }
    let available = available_blueprints(game);
    let unknown = available
        .iter()
        .filter(|record| !formulas.contains(record.as_str()))
        .count();
    let outside_survey = formulas
        .entries
        .iter()
        .filter(|entry| {
            !available
                .iter()
                .any(|record| record.as_str().eq_ignore_ascii_case(&entry.record))
        })
        .count();
    println!(
        "database: {} blueprint records by table class, {unknown} not yet known, \
         {outside_survey} known entries outside that survey; {problems} problem(s)",
        available.len()
    );
}

fn print_illusions(game: &GameData, illusions: &Illusions) {
    println!(
        "\nillusions: {} unlocked in {} slots (mod {:?}, expansion status {})",
        illusions.total_count(),
        illusions.slots.len(),
        illusions.mod_name,
        illusions.expansion_status
    );
    for slot in &illusions.slots {
        let category = IllusionCategory::from_slot_id(slot.slot).map_or_else(
            || format!("slot {} (unknown)", slot.slot),
            |c| c.to_string(),
        );
        println!("  {category}: {} records", slot.records.len());
        for record in &slot.records {
            let item = Item {
                base_name: record.clone(),
                ..Item::default()
            };
            let class = RecordId::parse(record.clone())
                .and_then(|id| game.item_info(&id))
                .and_then(Result::ok)
                .and_then(|info| info.class)
                .map_or_else(|| "?".to_string(), |class| class.to_string());
            println!("    {} — {class}  {record}", describe(game, &item));
        }
    }
    let problems = audit(illusions, game);
    for (record, error) in &problems {
        println!("  PROBLEM {record}: {error}");
    }
    let available = available_illusions(game);
    let unknown = available
        .iter()
        .filter(|record| !illusions.contains(record.as_str()))
        .count();
    println!(
        "database: {} illusion-bearing records, {unknown} not yet unlocked; {} problem(s)",
        available.len(),
        problems.len()
    );
}
