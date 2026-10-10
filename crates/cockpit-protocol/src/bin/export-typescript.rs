use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cockpit_protocol::typescript::{check_generated, write_generated};

fn usage() -> &'static str {
    "usage: export-typescript (--write|--check) <path>"
}

fn resolve_path(path: &str) -> std::io::Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn run() -> Result<ExitCode, String> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let mode = arguments
        .next()
        .ok_or_else(|| usage().to_owned())?
        .to_string_lossy()
        .into_owned();
    let path = arguments
        .next()
        .ok_or_else(|| usage().to_owned())?
        .to_string_lossy()
        .into_owned();
    if arguments.next().is_some() || !matches!(mode.as_str(), "--write" | "--check") {
        return Err(usage().to_owned());
    }

    let path = resolve_path(&path).map_err(|error| format!("cannot resolve target: {error}"))?;
    match mode.as_str() {
        "--write" => write_generated(&path)
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| format!("cannot write {}: {error}", path.display())),
        "--check" => match check_generated(&path) {
            Ok(drifted) if drifted.is_empty() => Ok(ExitCode::SUCCESS),
            Ok(drifted) => Err(format!(
                "generated TypeScript is out of date:\n{}",
                drifted
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        },
        _ => unreachable!(),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
