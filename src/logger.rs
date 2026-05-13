use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use simplelog::{Config, WriteLogger};

use crate::cli::Cli;

pub fn init_logger(cli: &Cli) -> io::Result<Option<PathBuf>> {
    let level = cli.log_level();

    if cli.verbosity == 0 {
        // No verbosity flags: swallow everything above error into a sink.
        WriteLogger::init(level, Config::default(), io::sink()).ok();
        return Ok(None);
    }

    let path = std::env::temp_dir().join(format!("carboxyl-{}.log", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)?;

    WriteLogger::init(level, Config::default(), file).ok();
    Ok(Some(path))
}

pub fn print_log_path_if_nonempty(log_path: Option<PathBuf>) {
    if let Some(path) = log_path
        && std::fs::metadata(&path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    {
        eprintln!("carboxyl logs written to {}", path.display());
    }
}
