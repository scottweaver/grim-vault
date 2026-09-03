//! Grim Vault desktop shell (egui/eframe): the transfer stash, the
//! vault store, and the characters of a Grim Dawn install, with the
//! item moves of `grimvault-core` behind drag-and-drop and autosave.
//!
//! `grimvault-gui --check <game dir> <save dir>` runs the load path
//! headless and prints what the window would show.

mod app;
mod autosave;
mod check;
mod documents;
mod drag;
mod facts;
mod grid;
mod icons;
mod loader;
mod panes;
mod settings;
mod setup;
mod theme;
mod watch;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use app::App;
use settings::ConfigDir;

const USAGE: &str = "usage: grimvault-gui [--check <game dir> <save dir>]";

enum Cli {
    Window,
    Check { game: PathBuf, save: PathBuf },
}

impl Cli {
    fn parse(args: &[OsString]) -> Result<Self, &'static str> {
        match args {
            [] => Ok(Self::Window),
            [flag, game, save] if flag == "--check" => Ok(Self::Check {
                game: PathBuf::from(game),
                save: PathBuf::from(save),
            }),
            _ => Err(USAGE),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match Cli::parse(&args) {
        Ok(Cli::Check { game, save }) => check::run(&game, &save),
        Ok(Cli::Window) => run_window(),
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::FAILURE
        }
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
            Ok(Cli::Check { game, save })
                if game.as_path() == Path::new("/game") && save.as_path() == Path::new("/save")
        ));
        let short: Vec<OsString> = ["--check", "/game"].map(OsString::from).to_vec();
        assert_eq!(Cli::parse(&short).err(), Some(USAGE));
    }
}
