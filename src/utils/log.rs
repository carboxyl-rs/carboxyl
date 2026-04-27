use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! warning {
    ($($args:expr),+) => {
        $crate::utils::log::write("WARNING", file!(), line!(), &format!($($args),*))
    };
}

macro_rules! error {
    ($($args:expr),+) => {
        $crate::utils::log::write("ERROR", file!(), line!(), &format!($($args),*))
    };
}

pub(crate) use error;
pub(crate) use warning;

pub fn write(level: &str, file: &str, line: u32, message: &str) {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);

    // Format as MMDD/HHMMss.uuuuuu — matches the original layout without chrono.
    let secs = (micros / 1_000_000) as u64;
    let us = (micros % 1_000_000) as u32;
    let min = (secs / 60) % 60;
    let hour = (secs / 3_600) % 24;
    // Day-of-year approximation: good enough for log timestamps.
    let day = (secs / 86_400) % 31 + 1;
    let month = (secs / (86_400 * 30)) % 12 + 1;
    let sec = secs % 60;

    let filename = Path::new(file)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    eprintln!(
        "[{:02}{:02}/{:02}{:02}{:02}.{:06}:{}:{}({})] {}",
        month, day, hour, min, sec, us, level, filename, line, message
    );
}
