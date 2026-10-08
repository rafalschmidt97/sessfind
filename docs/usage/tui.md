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
| `Enter` | Resume selected session in its recorded working directory |
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

## Resume in place

Press `Enter` on a selected result to resume immediately in the native session's
recorded working directory. There is no directory picker and sessfind never
creates a directory for resume or substitutes the current directory.

Claude's directory comes from the latest main-session `cwd` metadata, not the
encoded storage-folder name. This handles punctuation such as dots in usernames
without guessing paths. Other sources use their adapter's recorded directory.
If the source or directory is unavailable, the TUI stays open and shows an error.
Restore the original directory before retrying.

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
