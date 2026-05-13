use clap::Parser;
use color_eyre::eyre::Result;

use carboxyl::browser::run;
use carboxyl::browser::{BrowserConfig, BrowserRuntime};
use carboxyl::cli::Cli;
use carboxyl::output::restore_terminal;

fn main() -> Result<()> {
    color_eyre::install()?;

    if let Err(err) = carboxyl::platform::signal::register() {
        eprintln!("warning: failed to register signal handlers: {err}");
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        default_hook(panic_info);
    }));

    let cli = Cli::parse();
    let cfg = BrowserConfig::from_cli(&cli)?;
    let (runtime, channels) = BrowserRuntime::spawn(&cfg)?;

    run(channels, &cfg)?;

    // `runtime` drops here, joining the servo thread and restoring the terminal.
    drop(runtime);

    Ok(())
}
