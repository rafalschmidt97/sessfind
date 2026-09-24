mod app;
mod input;
mod ui;

use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::indexer::engine::IndexEngine;

pub use sessfind_common::CommandSpec as ResumeCommand;

fn handles_key_kind(kind: event::KeyEventKind) -> bool {
    matches!(
        kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    )
}

pub fn run(
    engine: &IndexEngine,
    initial_mode: Option<&str>,
    background_index: Option<mpsc::Receiver<Result<(), String>>>,
) -> Result<Option<ResumeCommand>> {
    // Validate catalog and initial mode before taking control of the terminal.
    let mut app = app::App::new(engine, initial_mode, background_index)?;

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // Main loop
    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        app.poll_debounced_search();
        app.poll_pending_search();
        app.poll_background_index();

        if let Ok(Some(ver)) = app.update_rx.try_recv()
            && ver != env!("CARGO_PKG_VERSION")
        {
            app.latest_version = Some(ver);
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key)
                    // Handle presses and held-key repeats, but ignore releases.
                    if handles_key_kind(key.kind) =>
                {
                    input::handle_key(&mut app, key);
                }
                Event::Paste(text) if app.focus == app::Focus::Search => {
                    input::handle_paste(&mut app, &text);
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(DisableBracketedPaste)?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(app.resume_command())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_pressed_and_held_keys_but_not_releases() {
        assert!(handles_key_kind(event::KeyEventKind::Press));
        assert!(handles_key_kind(event::KeyEventKind::Repeat));
        assert!(!handles_key_kind(event::KeyEventKind::Release));
    }
}
