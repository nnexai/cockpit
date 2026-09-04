use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cockpit_protocol::typescript::{check, render_v1, write_atomic};

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
        "--write" => write_atomic(&path, render_v1().as_bytes())
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| format!("cannot write {}: {error}", path.display())),
        "--check" => match check(&path) {
            Ok(true) => Ok(ExitCode::SUCCESS),
            Ok(false) => Err(format!(
                "generated TypeScript is out of date: {}",
                path.display()
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
