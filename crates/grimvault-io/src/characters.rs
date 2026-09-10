//! Finding the characters: the `player.gdc` of every character folder
//! under `main/` (the main campaign) and `user/` (custom games), in
//! folder order. The file never names its realm; the folder it sits
//! in is the only witness, so every character is handed back with the
//! realm it was found under.

use std::path::{Path, PathBuf};

use grimvault_core::gdc::Realm;

use crate::dirs::SaveDir;

/// The `player.gdc` of every character folder under `dir`, in folder
/// order.
#[must_use]
pub fn character_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|folder| folder.join("player.gdc"))
        .filter(|path| path.is_file())
        .collect()
}

/// Every character file under `main/` then `user/`, each realm in
/// folder order; a realm whose folder is absent contributes nothing.
#[must_use]
pub fn discover(save_dir: &SaveDir) -> Vec<(Realm, PathBuf)> {
    Realm::ALL
        .into_iter()
        .flat_map(|realm| {
            character_files(&save_dir.characters_dir(realm))
                .into_iter()
                .map(move |path| (realm, path))
        })
        .collect()
}

/// The display name the game's folder carries (`_<Name>`), for a
/// character whose file could not be opened.
#[must_use]
pub fn folder_name(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().trim_start_matches('_').to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_are_listed_main_campaign_first_then_custom_games() {
        let scratch =
            std::env::temp_dir().join(format!("grimvault-characters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("transfer.gst"), b"").unwrap();
        for folder in ["user/_Zark", "main/_Sif", "main/_Aria"] {
            let folder = scratch.join(folder);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("player.gdc"), b"x").unwrap();
        }
        std::fs::create_dir_all(scratch.join("main/_NoFile")).unwrap();

        let listed = discover(&SaveDir::parse(&scratch).unwrap());

        assert_eq!(
            listed,
            vec![
                (Realm::Main, scratch.join("main/_Aria/player.gdc")),
                (Realm::Main, scratch.join("main/_Sif/player.gdc")),
                (Realm::Custom, scratch.join("user/_Zark/player.gdc")),
            ]
        );
        assert_eq!(folder_name(&listed[2].1), "Zark");
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
