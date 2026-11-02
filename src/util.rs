use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use miette::IntoDiagnostic;
use ratatui::prelude::*;
use std::io;

/// Whether the terminal likely supports kitty's text sizing protocol (OSC 66).
/// Env-based; multiplexers are excluded since they swallow the sequence.
pub fn supports_text_sizing() -> bool {
    let in_multiplexer = std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some();
    !in_multiplexer
        && (std::env::var_os("KITTY_WINDOW_ID").is_some()
            || std::env::var("TERM").is_ok_and(|t| t == "xterm-kitty"))
}

pub fn setup_terminal() -> miette::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().into_diagnostic()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).into_diagnostic()?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).into_diagnostic()?;

    Ok(terminal)
}

pub fn restore_terminal() -> miette::Result<()> {
    // Restore terminal
    disable_raw_mode().into_diagnostic()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture).into_diagnostic()?;

    Ok(())
}
