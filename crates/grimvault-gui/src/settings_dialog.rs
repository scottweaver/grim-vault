//! The settings modal behind the gear: the directories setup asked
//! for, where the vault store lives, the two global rules, and the
//! vault's export and import. The fields are a draft the user applies
//! as one, so the shell decides once what the change costs — nothing,
//! a store switch, or a full reload ([`Change`]) — instead of reacting
//! to every keystroke.

use std::fmt;
use std::path::{Path, PathBuf};

use egui::{Id, Ui};
use grimvault_core::settings::{BlueprintSync, BulkDuplicates, ConfigDir, ReagentSync, Settings};
use thiserror::Error;
use univault_ui::theme::Theme;

use crate::setup::{DirProblem, GameDir, SaveDir};
use crate::theme::FITS;

/// The dialog's working state: the fields as text, re-validated every
/// frame so a verdict is never stale, over the settings it opened
/// from, whose campaign and nominations the draft carries unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDialog {
    game_field: String,
    save_field: String,
    store_field: String,
    sync: ReagentSync,
    blueprints: BlueprintSync,
    duplicates: BulkDuplicates,
    seed: Settings,
}

/// What the user asked for this frame. Export and import act at once
/// and leave the dialog open; the other two close it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Apply(Settings),
    Cancel,
    Export,
    Import,
}

/// What applying a draft costs the shell, most expensive wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing differs.
    Nothing,
    /// Only the rules changed; every open file stays.
    Rules,
    /// The store moved; the game files stay open.
    Store,
    /// A directory changed: the game data and every file reload.
    World,
}

impl Change {
    #[must_use]
    pub fn between(current: &Settings, next: &Settings, config: &ConfigDir) -> Self {
        if current.game_dir != next.game_dir || current.save_dir != next.save_dir {
            Self::World
        } else if current.store_file(config) != next.store_file(config) {
            Self::Store
        } else if current == next {
            Self::Nothing
        } else {
            Self::Rules
        }
    }

    fn apply_hint(self) -> &'static str {
        match self {
            Self::Nothing => "nothing has changed",
            Self::Rules => "saves the rules; every open file stays as it is",
            Self::Store => "switches to the other vault store; the game files stay open",
            Self::World => "reloads the game data and every file from the new directories",
        }
    }
}

/// Whether the store file is there already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreFileState {
    Present,
    Absent,
}

impl fmt::Display for StoreFileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Present => "exists; opened as the vault store",
            Self::Absent => "does not exist yet; created by the first save",
        })
    }
}

/// Why a store path was not accepted.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StoreFileProblem {
    #[error("choose a vault store file")]
    Empty,
    #[error("{} is a directory, not a file", .0.display())]
    IsDirectory(PathBuf),
    #[error("{} does not exist, so no store can be created there", .0.display())]
    NoParent(PathBuf),
}

/// A store path is acceptable when it is a file, or could become one:
/// its directory must exist, since a shared drive that is not mounted
/// is the likeliest mistake and creating a tree on the local disk in
/// its place would hide it.
pub fn store_file_verdict(path: &Path) -> Result<StoreFileState, StoreFileProblem> {
    if path.as_os_str().is_empty() {
        return Err(StoreFileProblem::Empty);
    }
    if path.is_dir() {
        return Err(StoreFileProblem::IsDirectory(path.to_path_buf()));
    }
    if path.is_file() {
        return Ok(StoreFileState::Present);
    }
    match path.parent() {
        Some(parent) if parent.is_dir() => Ok(StoreFileState::Absent),
        Some(parent) => Err(StoreFileProblem::NoParent(parent.to_path_buf())),
        None => Err(StoreFileProblem::NoParent(path.to_path_buf())),
    }
}

const SHARED_STORE_WHY: &str = "Point every machine's settings at one file on a shared drive to share a vault between \
     them; each keeps its own settings. Run one Grim Vault at a time on it — the app notices \
     another machine's write between saves, but two editing at once will lose one's edits.";

const SYNC_WHY: &str = "The open campaign's component and crafting-material storage raises the vault's counts to \
     its own whenever it loads; nothing is ever removed from the vault or the storage.";

const BLUEPRINT_SYNC_WHY: &str = "Every blueprint learned in the open campaign that the vault holds no item of is added to \
     the vault as a blueprint item whenever the list loads or changes; nothing is ever removed.";

const SKIP_DUPLICATES_WHY: &str = "Bulk moves and copies into the vault — the buttons on a tab and the auto-move standing \
     order — pass over an item whose record and roll seed the store already holds. Drags, \
     double-clicks and right-clicks always land.";

const EXPORT_WHY: &str = "Writes a copy of the vault as it is right now, unsaved edits included, to a file of your \
     choosing. The copy is a vault store file itself.";

const IMPORT_WHY: &str = "Adds every entry of another Grim Vault store file that this vault does not already hold; \
     nothing here is removed or changed. To use another file as the vault instead, set it \
     above.";

impl SettingsDialog {
    /// Opens on the settings in force, the store field showing the
    /// path they resolve to.
    #[must_use]
    pub fn open(current: &Settings, config: &ConfigDir) -> Self {
        Self {
            game_field: current.game_dir.display().to_string(),
            save_field: current.save_dir.display().to_string(),
            store_field: current.store_file(config).display().to_string(),
            sync: current.sync_reagents,
            blueprints: current.sync_blueprints,
            duplicates: current.bulk_duplicates,
            seed: current.clone(),
        }
    }

    /// The settings the fields describe: the seed's campaign and
    /// nominations over the fields' directories, store, and rules. A
    /// store field naming the default path is no override.
    #[must_use]
    pub fn draft(&self, config: &ConfigDir) -> Settings {
        let store = PathBuf::from(self.store_field.trim());
        Settings {
            store_file: (store != config.store_file()).then_some(store),
            sync_reagents: self.sync,
            sync_blueprints: self.blueprints,
            bulk_duplicates: self.duplicates,
            ..self.seed.with_dirs(
                PathBuf::from(self.game_field.trim()),
                PathBuf::from(self.save_field.trim()),
            )
        }
    }

    fn game_dir(&self) -> Result<GameDir, DirProblem> {
        GameDir::parse(Path::new(self.game_field.trim()))
    }

    fn save_dir(&self) -> Result<SaveDir, DirProblem> {
        SaveDir::parse(Path::new(self.save_field.trim()))
    }

    fn store_file(&self, config: &ConfigDir) -> Result<StoreFileState, StoreFileProblem> {
        store_file_verdict(&self.draft(config).store_file(config))
    }

    /// The modal; Esc and a click outside cancel.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        theme: &Theme,
        config: &ConfigDir,
        vault_items: usize,
    ) -> Option<Action> {
        let mut action = None;
        let response = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
            ui.set_width(720.0);
            ui.label(theme.heading("Settings"));
            ui.add_space(4.0);
            self.directories(ui, theme);
            ui.add_space(8.0);
            self.store(ui, theme, config);
            ui.add_space(8.0);
            self.rules(ui, theme);
            ui.add_space(8.0);
            ui.separator();
            if let Some(vault_action) = self.vault(ui, theme, config, vault_items) {
                action = Some(vault_action);
            }
            ui.separator();
            ui.label(theme.path_text(format!(
                "settings live in {}",
                config.settings_file().display()
            )));
            ui.add_space(8.0);
            if let Some(closing) = self.buttons(ui, theme, config) {
                action = Some(closing);
            }
        });
        if action.is_none() && response.should_close() {
            action = Some(Action::Cancel);
        }
        action
    }

    fn directories(&mut self, ui: &mut Ui, theme: &Theme) {
        dir_field(ui, "Game directory", &mut self.game_field, &[], theme);
        verdict_line(
            ui,
            self.game_dir().err().as_ref(),
            "database/database.arz found",
            theme,
        );
        ui.add_space(4.0);
        dir_field(ui, "Save directory", &mut self.save_field, &[], theme);
        verdict_line(
            ui,
            self.save_dir().err().as_ref(),
            "transfer.gst and main/ found",
            theme,
        );
    }

    fn store(&mut self, ui: &mut Ui, theme: &Theme, config: &ConfigDir) {
        ui.label(theme.section("Vault store file"))
            .on_hover_text(SHARED_STORE_WHY);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.store_field).desired_width(500.0));
            if ui
                .button("Browse…")
                .on_hover_text("An existing vault store file")
                .clicked()
                && let Some(picked) = store_dialog(&self.store_field).pick_file()
            {
                self.store_field = picked.display().to_string();
            }
            if ui
                .button("New…")
                .on_hover_text("Where a new, empty vault store should be created")
                .clicked()
                && let Some(picked) = store_dialog(&self.store_field)
                    .set_file_name(grimvault_core::settings::STORE_FILE)
                    .save_file()
            {
                self.store_field = picked.display().to_string();
            }
            if ui
                .button("Default")
                .on_hover_text("Beside the settings")
                .clicked()
            {
                self.store_field = config.store_file().display().to_string();
            }
        });
        match self.store_file(config) {
            Ok(state) => ui.colored_label(FITS, state.to_string()),
            Err(problem) => ui.colored_label(theme.palette.error, problem.to_string()),
        };
    }

    fn rules(&mut self, ui: &mut Ui, theme: &Theme) {
        ui.label(theme.section("Rules"));
        let mut syncing = self.sync == ReagentSync::On;
        if ui
            .checkbox(&mut syncing, "Sync component storage into the vault")
            .on_hover_text(SYNC_WHY)
            .changed()
        {
            self.sync = if syncing {
                ReagentSync::On
            } else {
                ReagentSync::Off
            };
        }
        let mut learning = self.blueprints == BlueprintSync::On;
        if ui
            .checkbox(&mut learning, "Sync learned blueprints into the vault")
            .on_hover_text(BLUEPRINT_SYNC_WHY)
            .changed()
        {
            self.blueprints = if learning {
                BlueprintSync::On
            } else {
                BlueprintSync::Off
            };
        }
        let mut skipping = self.duplicates == BulkDuplicates::Skip;
        if ui
            .checkbox(&mut skipping, "Skip duplicates in bulk moves")
            .on_hover_text(SKIP_DUPLICATES_WHY)
            .changed()
        {
            self.duplicates = if skipping {
                BulkDuplicates::Skip
            } else {
                BulkDuplicates::Allow
            };
        }
    }

    fn vault(
        &self,
        ui: &mut Ui,
        theme: &Theme,
        config: &ConfigDir,
        vault_items: usize,
    ) -> Option<Action> {
        ui.label(theme.section("Vault"));
        ui.label(theme.path_text(format!(
            "{vault_items} items open from {}",
            self.seed.store_file(config).display()
        )));
        let mut action = None;
        ui.horizontal(|ui| {
            if ui
                .button("Export a copy…")
                .on_hover_text(EXPORT_WHY)
                .clicked()
            {
                action = Some(Action::Export);
            }
            if ui
                .button("Import from a copy…")
                .on_hover_text(IMPORT_WHY)
                .clicked()
            {
                action = Some(Action::Import);
            }
        });
        action
    }

    fn buttons(&self, ui: &mut Ui, theme: &Theme, config: &ConfigDir) -> Option<Action> {
        let draft = self.draft(config);
        let valid =
            self.game_dir().is_ok() && self.save_dir().is_ok() && self.store_file(config).is_ok();
        let change = Change::between(&self.seed, &draft, config);
        let mut action = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(valid, egui::Button::new("Apply")).clicked() {
                action = Some(Action::Apply(draft));
            }
            if ui.button("Cancel").clicked() {
                action = Some(Action::Cancel);
            }
            ui.weak(change.apply_hint());
        });
        if !valid {
            ui.colored_label(theme.palette.warn, "fix the entries above to apply");
        }
        action
    }
}

fn store_dialog(field: &str) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new().add_filter("Grim Vault store", &["json"]);
    Path::new(field)
        .parent()
        .filter(|dir| dir.is_dir())
        .map_or_else(|| dialog.clone(), |dir| dialog.clone().set_directory(dir))
}

/// A directory field with a folder picker and the candidates the
/// platform found, shared with the first-run setup screen.
pub fn dir_field(
    ui: &mut Ui,
    label: &str,
    field: &mut String,
    candidates: &[PathBuf],
    theme: &Theme,
) {
    ui.label(theme.section(label));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(field).desired_width(560.0));
        if ui.button("Browse…").clicked() {
            let start = PathBuf::from(field.as_str());
            let dialog = if start.is_dir() {
                rfd::FileDialog::new().set_directory(&start)
            } else {
                rfd::FileDialog::new()
            };
            if let Some(picked) = dialog.pick_folder() {
                *field = picked.display().to_string();
            }
        }
    });
    if !candidates.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.weak("found here:");
            for candidate in candidates {
                let text = candidate.display().to_string();
                if ui.small_button(&text).clicked() {
                    *field = text;
                }
            }
        });
    }
}

pub fn verdict_line(ui: &mut Ui, problem: Option<&DirProblem>, ok: &str, theme: &Theme) {
    match problem {
        None => ui.colored_label(FITS, ok),
        Some(problem) => ui.colored_label(theme.palette.error, problem.to_string()),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ConfigDir {
        ConfigDir::new(PathBuf::from("/cfg/grim-vault"))
    }

    fn current() -> Settings {
        Settings {
            sync_reagents: ReagentSync::Off,
            ..Settings::for_dirs(PathBuf::from("/games/gd"), PathBuf::from("/saves/save"))
        }
    }

    #[test]
    fn the_draft_carries_the_seed_and_treats_the_default_store_path_as_no_override() {
        let dialog = SettingsDialog::open(&current(), &config());
        assert_eq!(dialog.store_field, "/cfg/grim-vault/vault-store.json");
        assert_eq!(dialog.draft(&config()), current());
        assert_eq!(
            Change::between(&current(), &dialog.draft(&config()), &config()),
            Change::Nothing
        );

        let mut moved = dialog.clone();
        moved.store_field = " /nas/vault-store.json ".into();
        let draft = moved.draft(&config());
        assert_eq!(
            draft.store_file,
            Some(PathBuf::from("/nas/vault-store.json"))
        );
        assert_eq!(draft.sync_reagents, ReagentSync::Off);
        assert_eq!(
            Change::between(&current(), &draft, &config()),
            Change::Store
        );

        let mut rules = dialog.clone();
        rules.sync = ReagentSync::On;
        rules.blueprints = BlueprintSync::Off;
        rules.duplicates = BulkDuplicates::Allow;
        let draft = rules.draft(&config());
        assert_eq!(draft.sync_reagents, ReagentSync::On);
        assert_eq!(draft.sync_blueprints, BlueprintSync::Off);
        assert_eq!(draft.bulk_duplicates, BulkDuplicates::Allow);
        assert_eq!(
            Change::between(&current(), &draft, &config()),
            Change::Rules
        );

        let mut world = moved;
        world.save_field = "/elsewhere/save".into();
        world.sync = ReagentSync::On;
        assert_eq!(
            Change::between(&current(), &world.draft(&config()), &config()),
            Change::World
        );
    }

    #[test]
    fn a_store_path_needs_an_existing_directory_and_must_not_be_one() {
        let scratch =
            std::env::temp_dir().join(format!("grimvault-settings-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        assert_eq!(
            store_file_verdict(Path::new("")),
            Err(StoreFileProblem::Empty)
        );
        assert_eq!(
            store_file_verdict(&scratch),
            Err(StoreFileProblem::IsDirectory(scratch.clone()))
        );
        assert_eq!(
            store_file_verdict(&scratch.join("vault-store.json")),
            Ok(StoreFileState::Absent)
        );
        std::fs::write(scratch.join("vault-store.json"), b"{}").unwrap();
        assert_eq!(
            store_file_verdict(&scratch.join("vault-store.json")),
            Ok(StoreFileState::Present)
        );
        assert_eq!(
            store_file_verdict(&scratch.join("unmounted").join("vault-store.json")),
            Err(StoreFileProblem::NoParent(scratch.join("unmounted")))
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
