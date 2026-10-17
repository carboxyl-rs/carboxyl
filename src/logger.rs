use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fd::OwnedFd;

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
    std::fs::File::create(&path)?;
    let file = OpenOptions::new().append(true).open(&path)?;

    WriteLogger::init(level, Config::default(), file).ok();
    Ok(Some(path))
}

/// Keep native graphics-driver diagnostics out of the terminal's cell buffer.
pub struct StderrGuard(OwnedFd);

impl StderrGuard {
    pub fn redirect(log_path: Option<&Path>) -> io::Result<Self> {
        let destination = OpenOptions::new()
            .write(true)
            .append(log_path.is_some())
            .open(log_path.unwrap_or(Path::new("/dev/null")))?;
        let original = rustix::io::dup(rustix::stdio::stderr())?;
        rustix::stdio::dup2_stderr(&destination)?;
        Ok(Self(original))
    }
}

impl Drop for StderrGuard {
    fn drop(&mut self) {
        let _ = rustix::stdio::dup2_stderr(&self.0);
    }
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
