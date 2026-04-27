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

    /// Maximum frames per second.
    /// Lower values reduce terminal rendering overhead and can make
    /// page interaction feel smoother under load. 24 fps is recommended.
    #[arg(short = 'f', long = "fps", default_value_t = 24)]
    pub fps: u32,

    /// Rendering resolution: pixels rendered per terminal cell.
    /// Each cell encodes a 2×4 sub-pixel grid; higher values give Servo
    /// more pixels to render into before downsampling. Default 200 = 4×8px/cell.
    /// Lower values make it easier to read in a standard terminal cell.
    #[arg(short = 'r', long = "resolution", default_value_t = 200)]
    pub resolution: u32,

    /// Enable debug logs
    #[arg(short = 'd', long = "debug", action = ArgAction::SetTrue)]
    pub debug: bool,
}
