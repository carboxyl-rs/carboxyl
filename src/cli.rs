use clap::{ArgAction, Parser};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "carbonyl",
    version,
    about = "Carboxyl is a Servo based browser for the terminal."
)]
pub struct Cli {
    /// URL to open
    pub url: Option<String>,

    /// Set the maximum number of frames per second
    #[arg(short = 'f', long = "fps", default_value_t = 60)]
    pub fps: u32,

    /// Set the zoom level in percent
    #[arg(short = 'z', long = "zoom", default_value_t = 100)]
    pub zoom: u32,

    /// Render text as bitmaps
    #[arg(short = 'b', long = "bitmap", action = ArgAction::SetTrue)]
    pub bitmap: bool,

    /// Enable debug logs
    #[arg(short = 'd', long = "debug", action = ArgAction::SetTrue)]
    pub debug: bool,
}
