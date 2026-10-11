#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{ffi::OsString, process::ExitCode};

fn main() -> ExitCode {
    // A compatibility probe must never initialize the GUI or contact Herdr.
    // Only the separate host CLI implements the orchestration command contract.
    if is_orchestration_cli_request(std::env::args_os().skip(1)) {
        eprintln!("Cockpit is the graphical client; use cockpit-cli for orchestration commands");
        return ExitCode::from(2);
    }
    cockpit_tauri::run();
    ExitCode::SUCCESS
}

fn is_orchestration_cli_request(mut args: impl Iterator<Item = OsString>) -> bool {
    args.next().is_some_and(|arg| arg == "orchestration")
}

#[cfg(test)]
mod tests {
    use super::is_orchestration_cli_request;

    #[test]
    fn protocol_probe_is_rejected_before_gui_startup() {
        assert!(is_orchestration_cli_request(
            ["orchestration", "protocol", "--json"]
                .into_iter()
                .map(Into::into)
        ));
    }

    #[test]
    fn malformed_orchestration_commands_do_not_launch_the_gui() {
        assert!(is_orchestration_cli_request(
            ["orchestration"].into_iter().map(Into::into)
        ));
        assert!(is_orchestration_cli_request(
            ["orchestration", "unknown"].into_iter().map(Into::into)
        ));
    }

    #[test]
    fn normal_gui_startup_does_not_claim_cli_support() {
        assert!(!is_orchestration_cli_request(std::iter::empty()));
    }
}
