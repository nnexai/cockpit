#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{ffi::OsString, process::ExitCode};

fn main() -> ExitCode {
    match classify_gui_args(std::env::args_os().skip(1)) {
        GuiArgs::Run => cockpit_tauri::run(),
        GuiArgs::Help => println!("Cockpit graphical app\n\nUsage: cockpit [--help | --version]\n\nFor commands use cockpit-cli, for example cockpit-cli service start or cockpit-cli serve."),
        GuiArgs::Version => println!("cockpit {}", env!("CARGO_PKG_VERSION")),
        GuiArgs::Reject => {
            eprintln!("cockpit is the graphical app; use cockpit-cli <command> (for example cockpit-cli service start or cockpit-cli serve)");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

#[derive(Debug, PartialEq, Eq)]
enum GuiArgs { Run, Help, Version, Reject }

fn classify_gui_args(mut args: impl Iterator<Item = OsString>) -> GuiArgs {
    let Some(first) = args.next() else { return GuiArgs::Run };
    if args.next().is_some() { return GuiArgs::Reject; }
    if first == "--help" || first == "-h" { GuiArgs::Help }
    else if first == "--version" || first == "-V" { GuiArgs::Version }
    else { GuiArgs::Reject }
}

#[cfg(test)]
mod tests {
    use super::{classify_gui_args, GuiArgs};

    #[test]
    fn cli_requests_never_start_gui() {
        for args in [
            vec!["orchestration", "protocol", "--json"],
            vec!["orchestration"],
            vec!["orchestration", "unknown"],
            vec!["serve"],
            vec!["--bogus"],
            vec!["--help", "extra"],
        ] {
            assert_eq!(classify_gui_args(args.into_iter().map(Into::into)), GuiArgs::Reject);
        }
    }

    #[test]
    fn gui_args_classify_without_starting_gui() {
        assert_eq!(classify_gui_args(std::iter::empty()), GuiArgs::Run);
        assert_eq!(classify_gui_args(["--help"].into_iter().map(Into::into)), GuiArgs::Help);
        assert_eq!(classify_gui_args(["--version"].into_iter().map(Into::into)), GuiArgs::Version);
    }
}
