use carboxyl::{browser::AppResult, cli::Cli};
use clap::Parser;

fn main() -> AppResult<()> {
    let cli = Cli::parse();
    carboxyl::browser::run(cli)
}
