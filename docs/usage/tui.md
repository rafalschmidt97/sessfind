# Interactive TUI

## Launching the TUI

```bash
sessfind            # launch TUI
sessfind --index    # index all sources first, then launch TUI
sessfind --index-in-background  # launch now and refresh while using the TUI
```

Background indexing starts a separate process, so it can finish after the TUI
closes. The status bar shows progress and reloads the session list when indexing
finishes while the TUI is open.

## Pane Layout

The TUI opens in full-screen mode with three areas:

- **Left pane** — search results list (source, project, date)
- **Right pane** — session details and conversation preview
- **Bottom** — search input with mode indicator

![sessfind interactive TUI — split-pane search and session preview](../assets/tui-fts.webp)

## Keybindings

| Key | Action |
|-----|--------|
| *Type / Backspace / Delete* | Filter sessions after a one-second pause in editing |
| `Tab` | Switch focus between search and results |
| `Shift+Tab` | Toggle search mode (FTS / Fuzzy / LLM / Semantic*) |
| `Ctrl+S` | Toggle sort order (Newest first / Best match) |
| `Up/Down`, `j/k` | Navigate results |
| `Enter` | Resume selected session (opens confirmation dialog) |
| `PgUp/PgDn` | Scroll session preview by one page |
| `r` | Re-index the selected session's source (preview pane) |
| `Ctrl+U` | Clear search input |
| `Alt+Backspace` / `Alt+Delete` | Delete the previous / next word in search |
| `Alt+Left` / `Alt+Right` | Move by one word in search |
| `Alt+B` / `Alt+F` / `Alt+D` | Move backward / forward or delete the next word |
| `Ctrl+A` / `Ctrl+E` | Move to the start / end of search |
| `Ctrl+W` / `Ctrl+K` | Delete the previous word / rest of search |
| `F1` | Show help popup |
| `Esc` | Cancel a pending search, close help, or quit |

!!! note
    `Semantic` mode is only available when the [`sessfind-semantic`](../plugins/semantic-search.md) plugin is installed.

## Sort Order

Press `Ctrl+S` (while in search focus) to toggle the sort order of results:

| Sort Mode | Description |
|-----------|-------------|
| **Newest first** *(default)* | Sessions sorted by time descending, then by relevance score |
| **Best match** | Sessions sorted by relevance score descending, then by time |

The current sort order is displayed at the bottom of the results list. The setting persists until the application is closed — switching between search and results does not reset it.

## Resume Confirmation

When you press `Enter` on a selected session, a confirmation dialog appears showing:

- **Session summary** — source, date (in local time), and title
- **Directory choice** — where to resume the session:
    - **Session directory** — the original project directory (if it no longer exists, it will be created)
    - **Current directory** — your current working directory
    - **Cancel** — go back to search

Use `↑/↓` to select an option and `Enter` to confirm, or `Esc` to cancel.

![sessfind resume confirmation dialog](../assets/tui-resume.webp)

All dates in the TUI are displayed in your computer's local timezone.

## Conversation preview

For Claude and OpenCode, the preview reads conversation text from the native
JSONL file or SQLite database when a session is selected. It does not reconstruct
the conversation from overlapping search chunks. Claude queued user prompts are
included and deduplicated by message identity; sidechain records are marked.
OpenCode text parts use message order with stable part ordering.

The coverage line identifies native text or an indexed fallback. Missing or
malformed native sources fall back to the index with a warning, so that content
may be incomplete or stale. Other sources continue to use indexed previews.
Tool output and reasoning are excluded; attachments appear as markers without
embedding image data. A native preview is a read at selection time, not a live
tail or a snapshot-checked transcript export. Selecting another session and
returning reloads it; background-index completion also refreshes the preview.

Search failures are shown separately from an empty result set and keep the
previous successful results visible. The status bar and preview also warn when
a source is stale or failed; press `r` in the preview pane to retry that source.

Full-text and fuzzy searches wait until input has been unchanged for one second,
so typing, cursor movement, and held Backspace remain responsive. Press `Enter`
to run a pending search immediately. Semantic and LLM searches remain
Enter-triggered.

On macOS, `Cmd+V` uses the terminal's normal bracketed-paste handling. Command
editing keys are terminal-dependent, so the TUI provides Ctrl and Alt bindings
that work consistently in terminal applications.
