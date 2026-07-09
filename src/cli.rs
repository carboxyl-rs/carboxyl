use clap::{ArgAction, Parser};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "carboxyl",
    version,
    about = "Carboxyl is a Servo-based browser for the terminal."
)]
pub struct Cli {
    /// URL to open
    pub url: Option<String>,

    /// Framerate. Maximum frames per second.
    /// Lower values reduce terminal rendering overhead and can make
    /// page interaction feel smoother under load.
    #[arg(short = 'f', long = "fps", default_value_t = 60)]
    pub fps: u16,

    /// Browser zoom percentage (100 = default).
    /// Higher values zoom in and make page content larger.
    /// Lower values zoom out and show more content at once.
    #[arg(short = 's', long = "scale", default_value_t = 100)]
    pub scale: u16,

    /// Increase log verbosity. Pass up to four times for progressively
    /// finer output: -v = warn, -vv = info, -vvv = debug, -vvvv = trace.
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbosity: u8,
}

impl Cli {
    /// Resolve the requested verbosity to a `log::LevelFilter`.
    pub fn log_level(&self) -> log::LevelFilter {
        match self.verbosity {
            0 => log::LevelFilter::Error,
            1 => log::LevelFilter::Warn,
            2 => log::LevelFilter::Info,
            3 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        }
    }
}
