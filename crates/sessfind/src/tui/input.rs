use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Focus, ResultsPane, ResumeOption};

pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Resume confirmation dialog intercepts all keys
    if app.confirm_resume.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.confirm_resume = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(state) = &mut app.confirm_resume {
                    state.selected = state.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(state) = &mut app.confirm_resume {
                    state.selected = (state.selected + 1).min(ResumeOption::ALL.len() - 1);
                }
            }
            KeyCode::Enter => {
                let option = app
                    .confirm_resume
                    .as_ref()
                    .map(|s| ResumeOption::ALL[s.selected])
                    .unwrap_or(ResumeOption::Cancel);
                app.confirm_resume_select(option);
            }
            _ => {}
        }
        return;
    }

    // Help popup intercepts all keys
    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => {
                app.show_help = false;
                app.help_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.help_scroll = app.help_scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
            KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10),
            KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if app.semantic_searching || app.llm_searching {
                app.cancel_pending_search();
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::BackTab if app.focus == Focus::Search => {
            app.toggle_mode();
        }
        KeyCode::Tab => {
            app.toggle_focus();
        }
        KeyCode::F(1) => {
            app.show_help = !app.show_help;
            if !app.show_help {
                app.help_scroll = 0;
            }
        }
        _ => match app.focus {
            Focus::Search => handle_search_key(app, key),
            Focus::Results => handle_results_key(app, key),
        },
    }
}

pub fn handle_paste(app: &mut App, text: &str) {
    app.input.insert_str(app.cursor_pos, text);
    app.cursor_pos += text.len();
    app.on_input_changed();
}

fn previous_word_boundary(input: &str, cursor: usize) -> usize {
    let before = &input[..cursor];
    let without_space = before.trim_end_matches(char::is_whitespace);
    without_space
        .rfind(char::is_whitespace)
        .map(|index| index + input[index..].chars().next().unwrap().len_utf8())
        .unwrap_or(0)
}

fn next_word_boundary(input: &str, cursor: usize) -> usize {
    let after = &input[cursor..];
    let without_space = after.trim_start_matches(char::is_whitespace);
    let word_start = input.len() - without_space.len();
    without_space
        .find(char::is_whitespace)
        .map(|index| word_start + index)
        .unwrap_or(input.len())
}

fn delete_previous_word(app: &mut App) {
    if app.cursor_pos == 0 {
        return;
    }
    let start = previous_word_boundary(&app.input, app.cursor_pos);
    app.input.drain(start..app.cursor_pos);
    app.cursor_pos = start;
    app.on_input_changed();
}

fn delete_next_word(app: &mut App) {
    if app.cursor_pos == app.input.len() {
        return;
    }
    let end = next_word_boundary(&app.input, app.cursor_pos);
    app.input.drain(app.cursor_pos..end);
    app.on_input_changed();
}

fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_sort();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.clear();
            app.cursor_pos = 0;
            app.on_input_changed();
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_pos = 0;
            app.on_cursor_moved();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_pos = app.input.len();
            app.on_cursor_moved();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            delete_previous_word(app);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
            delete_next_word(app);
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_pos = previous_word_boundary(&app.input, app.cursor_pos);
            app.on_cursor_moved();
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_pos = next_word_boundary(&app.input, app.cursor_pos);
            app.on_cursor_moved();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.cursor_pos < app.input.len() {
                app.input.truncate(app.cursor_pos);
                app.on_input_changed();
            }
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
        {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += c.len_utf8();
            app.on_input_changed();
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) && app.cursor_pos > 0 => {
            delete_previous_word(app);
        }
        KeyCode::Delete
            if key.modifiers.contains(KeyModifiers::ALT) && app.cursor_pos < app.input.len() =>
        {
            delete_next_word(app);
        }
        KeyCode::Backspace if app.cursor_pos > 0 => {
            // Find previous char boundary
            let prev = app.input[..app.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            app.input.remove(prev);
            app.cursor_pos = prev;
            app.on_input_changed();
        }
        KeyCode::Delete if app.cursor_pos < app.input.len() => {
            app.input.remove(app.cursor_pos);
            app.on_input_changed();
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) && app.cursor_pos > 0 => {
            app.cursor_pos = previous_word_boundary(&app.input, app.cursor_pos);
            app.on_cursor_moved();
        }
        KeyCode::Right
            if key.modifiers.contains(KeyModifiers::ALT) && app.cursor_pos < app.input.len() =>
        {
            app.cursor_pos = next_word_boundary(&app.input, app.cursor_pos);
            app.on_cursor_moved();
        }
        KeyCode::Home => {
            app.cursor_pos = 0;
            app.on_cursor_moved();
        }
        KeyCode::End => {
            app.cursor_pos = app.input.len();
            app.on_cursor_moved();
        }
        KeyCode::Left if app.cursor_pos > 0 => {
            app.cursor_pos = app.input[..app.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            app.on_cursor_moved();
        }
        KeyCode::Right if app.cursor_pos < app.input.len() => {
            app.cursor_pos = app.input[app.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| app.cursor_pos + i)
                .unwrap_or(app.input.len());
            app.on_cursor_moved();
        }
        KeyCode::Enter => {
            // Deferred modes: Enter triggers the search
            if app.search_mode().is_deferred() && !app.input.is_empty() {
                if app.search_mode().is_llm() {
                    app.request_llm_search();
                } else {
                    app.request_semantic_search();
                }
            } else {
                app.flush_debounced_search();
                if !app.results.is_empty() {
                    app.results_pane = ResultsPane::List;
                    app.focus = Focus::Results;
                }
            }
        }
        _ => {}
    }
}

fn handle_results_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Left => {
            app.results_pane = ResultsPane::List;
        }
        KeyCode::Right => {
            app.results_pane = ResultsPane::Preview;
        }
        KeyCode::Up | KeyCode::Char('k') => match app.results_pane {
            ResultsPane::List => app.select_prev(),
            ResultsPane::Preview => app.scroll_detail_up(),
        },
        KeyCode::Down | KeyCode::Char('j') => match app.results_pane {
            ResultsPane::List => app.select_next(),
            ResultsPane::Preview => app.scroll_detail_down(),
        },
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_sort();
        }
        KeyCode::Char('r') if app.results_pane == ResultsPane::Preview => {
            app.reindex_selected_source();
        }
        KeyCode::Enter => app.resume_selected(),
        KeyCode::PageUp => app.scroll_detail_page_up(),
        KeyCode::PageDown => app.scroll_detail_page_down(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_word_boundary_includes_preceding_whitespace() {
        let input = "find old query";

        assert_eq!(previous_word_boundary(input, input.len()), 9);
        assert_eq!(previous_word_boundary("find old   ", 11), 5);
        assert_eq!(previous_word_boundary("żółw test", "żółw".len()), 0);
    }

    #[test]
    fn next_word_boundary_includes_following_whitespace() {
        assert_eq!(next_word_boundary("find old query", 5), 8);
        assert_eq!(next_word_boundary("find   old", 4), 10);
        assert_eq!(next_word_boundary("żółw test", 0), "żółw".len());
    }
}
