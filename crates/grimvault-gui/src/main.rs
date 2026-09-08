//! Grim Vault desktop shell (egui/eframe): the transfer stash, the
//! vault store, and the characters of a Grim Dawn install, with the
//! item moves of `grimvault-core` behind drag-and-drop and autosave.
//!
//! `grimvault-gui --check [<game dir> <save dir>]` runs the load path
//! headless and prints what the window would show; without the two
//! directories it uses the saved settings, like the window does.

mod app;
mod automove;
mod autosave;
mod badges;
mod check;
mod crafting;
mod documents;
mod drag;
mod facts;
mod grid;
mod icons;
mod loader;
mod panes;
mod reference;
mod search;
mod settings;
mod settings_dialog;
mod setup;
mod sockets;
mod stat_lines;
mod theme;
mod ui_state;
mod watch;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use app::App;
use settings::{ConfigDir, Settings};

const USAGE: &str = "usage: grimvault-gui [--check [<game dir> <save dir>]]";

enum Cli {
    Window,
    Check(CheckPaths),
}

/// Where a headless check reads from: given on the command line, or
/// the saved settings.
enum CheckPaths {
    Explicit { game: PathBuf, save: PathBuf },
    Saved,
}

impl Cli {
    fn parse(args: &[OsString]) -> Result<Self, &'static str> {
        match args {
            [] => Ok(Self::Window),
            [flag] if flag == "--check" => Ok(Self::Check(CheckPaths::Saved)),
            [flag, game, save] if flag == "--check" => Ok(Self::Check(CheckPaths::Explicit {
                game: PathBuf::from(game),
                save: PathBuf::from(save),
            })),
            _ => Err(USAGE),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match Cli::parse(&args) {
        Ok(Cli::Check(paths)) => run_check(paths),
        Ok(Cli::Window) => run_window(),
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::FAILURE
        }
    }
}

/// Explicit directories carry no remembered campaign and no standing
/// orders: only the saved settings do, so only a check run from them
/// opens on the campaign and shows what the orders would do.
fn run_check(paths: CheckPaths) -> ExitCode {
    let (game, save, settings) = match paths {
        CheckPaths::Explicit { game, save } => (game, save, None),
        CheckPaths::Saved => match saved_settings() {
            Ok(settings) => (
                settings.game_dir.clone(),
                settings.save_dir.clone(),
                Some(settings),
            ),
            Err(error) => {
                eprintln!("grim-vault: {error}");
                return ExitCode::FAILURE;
            }
        },
    };
    check::run(&game, &save, settings.as_ref())
}

/// The saved settings, or why a check cannot run without directories.
fn saved_settings() -> Result<Settings, String> {
    let config = ConfigDir::resolve().map_err(|error| error.to_string())?;
    match settings::load(&config) {
        Ok(Some(settings)) => Ok(settings),
        Ok(None) => Err(format!(
            "no settings at {}: run the app once, or pass --check <game dir> <save dir>",
            config.settings_file().display()
        )),
        Err(error) => Err(error.to_string()),
    }
}

fn run_window() -> ExitCode {
    let config = match ConfigDir::resolve() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("grim-vault: {error}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Grim Vault")
            .with_app_id("grim-vault")
            .with_inner_size([1380.0, 920.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    let outcome = eframe::run_native(
        "Grim Vault",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, config)))),
    );
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("grim-vault: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_check_flag_takes_exactly_two_directories() {
        assert!(matches!(Cli::parse(&[]), Ok(Cli::Window)));
        let args: Vec<OsString> = ["--check", "/game", "/save"].map(OsString::from).to_vec();
        assert!(matches!(
            Cli::parse(&args),
            Ok(Cli::Check(CheckPaths::Explicit { game, save }))
                if game.as_path() == Path::new("/game") && save.as_path() == Path::new("/save")
        ));
        let short: Vec<OsString> = ["--check", "/game"].map(OsString::from).to_vec();
        assert_eq!(Cli::parse(&short).err(), Some(USAGE));
        let bare: Vec<OsString> = vec![OsString::from("--check")];
        assert!(matches!(
            Cli::parse(&bare),
            Ok(Cli::Check(CheckPaths::Saved))
        ));
    }
}
