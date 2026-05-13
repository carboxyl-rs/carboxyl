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
    // resize in runtime with alacritty ctrl +/-, not a _feature_ though
    pub scale: u16,

    /// Disable native terminal text rendering.
    /// By default, text is extracted from the page and rendered using the
    /// terminal's own glyph pipeline for crisp, resolution-independent output.
    /// Pass this flag to use the pixel-only renderer instead.
    #[arg(long = "no-native-text", action = ArgAction::SetTrue)]
    pub no_native_text: bool,

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
