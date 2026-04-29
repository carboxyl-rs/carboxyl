use carboxyl::{browser::AppResult, cli::Cli};
use clap::Parser;

fn main() -> AppResult<()> {
    // Install a global panic hook *before* anything else so that any panic
    // on any thread, including Servo's internal worker threads; restores
    // the terminal before printing the panic message.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore terminal first so the panic message is readable.
        ratatui::restore();
        crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture,).ok();
        default_hook(info);
    }));

    let cli = Cli::parse();
    carboxyl::browser::run(cli)
}
