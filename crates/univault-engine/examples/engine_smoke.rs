//! Smoke test of the engine parsers against a real Grim Dawn install.
//!
//! ```text
//! cargo run -p univault-engine --example engine_smoke -- "/path/to/Grim Dawn"
//! ```
//!
//! Parses the three databases with the Grim Dawn dialect and two
//! resource archives with the LZ4 codec, then prints enough of what it
//! found to eyeball against the game.

use std::error::Error;
use std::path::Path;
use std::time::Instant;

use univault_engine::arc::ArcFile;
use univault_engine::arz::{ArzDialect, ArzFile, DbRecord, DbValues};
use univault_engine::codec::Codec;
use univault_engine::tex;
use univault_engine::text::TextDb;

const DATABASES: [&str; 3] = [
    "database/database.arz",
    "gdx1/database/GDX1.arz",
    "gdx2/database/GDX2.arz",
];

fn main() -> Result<(), Box<dyn Error>> {
    let game_dir = std::env::args()
        .nth(1)
        .ok_or("usage: engine_smoke <grim dawn game dir>")?;
    let game_dir = Path::new(&game_dir);

    let databases = DATABASES
        .iter()
        .map(|relative| load_arz(game_dir, relative))
        .collect::<Result<Vec<_>, _>>()?;
    let base = &databases[0];

    println!("\nfirst five record ids of {}:", DATABASES[0]);
    for id in base.record_ids().take(5) {
        println!("  {}", id.as_str());
    }

    let item = sample_item(base)?;
    println!(
        "\nitem record {} ({}), {} variables:",
        item.id.as_str(),
        item.record_type,
        item.variables().count()
    );
    for variable in item.variables() {
        println!("  {} = {}", variable.name, render_values(&variable.values));
    }

    let text = load_text(game_dir)?;
    println!("\nText_EN.arc: {} tags", text.len());
    for tag_variable in ["itemNameTag", "description"] {
        if let Some(tag) = item.string(tag_variable) {
            match text.get(tag) {
                Some(label) => println!("  {tag_variable} {tag} => {label:?}"),
                None => println!("  {tag_variable} {tag} => (no label)"),
            }
        }
    }

    load_items_arc(game_dir, &item)?;
    Ok(())
}

fn load_arz(game_dir: &Path, relative: &str) -> Result<ArzFile, Box<dyn Error>> {
    let started = Instant::now();
    let bytes = std::fs::read(game_dir.join(relative))?;
    let size = bytes.len();
    let arz = ArzFile::parse(bytes, ArzDialect::grim_dawn())?;
    println!(
        "{relative}: {} records, {} MiB, parsed in {:?}",
        arz.len(),
        size / (1024 * 1024),
        started.elapsed()
    );
    let sweep = Instant::now();
    let failures: Vec<String> = arz
        .record_ids()
        .filter_map(|id| match arz.record(id) {
            Some(Err(error)) => Some(format!("{}: {error}", id.as_str())),
            Some(Ok(_)) | None => None,
        })
        .collect();
    println!(
        "  decoded every record in {:?}: {} failures{}",
        sweep.elapsed(),
        failures.len(),
        failures
            .first()
            .map_or_else(String::new, |first| format!(" (first: {first})"))
    );
    Ok(arz)
}

/// The first `records/items/` record carrying an `itemNameTag`, so the
/// text lookup below has something to resolve; falls back to any
/// item record.
fn sample_item(base: &ArzFile) -> Result<DbRecord, Box<dyn Error>> {
    let item_ids = base
        .record_ids()
        .filter(|id| id.as_str().to_ascii_lowercase().contains("records/items/"));
    let mut fallback = None;
    for id in item_ids {
        let record = base.record(id).ok_or("record vanished")??;
        if record.string("itemNameTag").is_some() {
            return Ok(record);
        }
        fallback.get_or_insert(record);
    }
    fallback.ok_or_else(|| "no records/items/ record in the base database".into())
}

fn render_values(values: &DbValues) -> String {
    match values {
        DbValues::Integers(values) => format!("{values:?}"),
        DbValues::Floats(values) => format!("{values:?}"),
        DbValues::Strings(values) => format!("{values:?}"),
        DbValues::Booleans(values) => format!("{values:?}"),
    }
}

fn load_text(game_dir: &Path) -> Result<TextDb, Box<dyn Error>> {
    let started = Instant::now();
    let bytes = std::fs::read(game_dir.join("resources/Text_EN.arc"))?;
    let arc = ArcFile::parse(bytes, Codec::Lz4Block)?;
    let mut names: Vec<&str> = arc.file_names().collect();
    names.sort_unstable();
    println!(
        "\nresources/Text_EN.arc: {} files, parsed in {:?}",
        names.len(),
        started.elapsed()
    );
    let mut text = TextDb::new();
    for name in names {
        if name.to_ascii_lowercase().ends_with(".txt") {
            let file = arc.file(name).ok_or("entry vanished")??;
            text.add_file(&file);
            println!("  {name}: {} bytes", file.len());
        }
    }
    Ok(text)
}

fn load_items_arc(game_dir: &Path, item: &DbRecord) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let bytes = std::fs::read(game_dir.join("resources/Items.arc"))?;
    let size = bytes.len();
    let arc = ArcFile::parse(bytes, Codec::Lz4Block)?;
    let mut names: Vec<&str> = arc.file_names().collect();
    names.sort_unstable();
    println!(
        "\nresources/Items.arc: {} files, {} MiB, parsed in {:?}",
        names.len(),
        size / (1024 * 1024),
        started.elapsed()
    );

    let tex_names: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| name.to_ascii_lowercase().ends_with(".tex"))
        .collect();
    let first_tex = tex_names.first().ok_or("no .tex in Items.arc")?;
    report_tex(&arc, first_tex);
    if let Some(bitmap) = item.string("bitmap") {
        // Record bitmap paths start with the archive's name
        // (`items/...`), which entry names inside Items.arc omit.
        let inside_archive = bitmap.split_once('/').map_or(bitmap, |(_, rest)| rest);
        report_tex(&arc, inside_archive);
    }

    let sweep = Instant::now();
    let mut failures = Vec::new();
    let mut extracted_bytes = 0_usize;
    for name in &tex_names {
        match arc.file(name).ok_or("entry vanished")? {
            Ok(bytes) => {
                extracted_bytes += bytes.len();
                if let Err(error) = tex::dimensions(&bytes) {
                    failures.push(format!("{name}: {error}"));
                }
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    println!(
        "  extracted all {} .tex files ({} MiB) and read their headers in {:?}: {} failures{}",
        tex_names.len(),
        extracted_bytes / (1024 * 1024),
        sweep.elapsed(),
        failures.len(),
        failures
            .first()
            .map_or_else(String::new, |first| format!(" (first: {first})"))
    );
    Ok(())
}

fn report_tex(arc: &ArcFile, name: &str) {
    match arc.file(name) {
        None => println!("  {name}: not in archive"),
        Some(Err(error)) => println!("  {name}: extract failed: {error}"),
        Some(Ok(bytes)) => match tex::dimensions(&bytes) {
            Ok((width, height)) => {
                let (cols, rows) = tex::cells(width, height);
                println!(
                    "  {name}: {} bytes, {width}x{height} px = {cols}x{rows} cells",
                    bytes.len()
                );
            }
            Err(error) => println!(
                "  {name}: {} bytes, header not decoded: {error} (first bytes {:02x?})",
                bytes.len(),
                &bytes[..bytes.len().min(16)]
            ),
        },
    }
}
