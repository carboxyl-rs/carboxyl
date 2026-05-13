use carboxyl::browser::AppResult;
use carboxyl::cli::Cli;
use clap::Parser;

fn main() -> AppResult<()> {
    // Register fatal signal handlers before anything touches the terminal.
    // These restore the terminal on SIGSEGV/SIGBUS/SIGABRT/SIGILL so the
    // shell isn't left in raw mode after a crash.
    if let Err(e) = carboxyl::platform::signal::register() {
        eprintln!("warning: failed to register signal handlers: {e}");
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture,).ok();
        default_hook(info);
    }));

    let cli = Cli::parse();
    carboxyl::browser::run(cli)
}
