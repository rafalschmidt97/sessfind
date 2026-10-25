# How It Works

## Indexing pipeline

1. sessfind reads session files from each configured source (GitHub Copilot, Claude Code, OpenCode, Cursor, Codex).
2. Credentials are redacted from message text and titles (see below).
3. User/assistant messages are paired and split into chunks (~6000 chars each).
4. Chunks are indexed with [tantivy](https://github.com/quickwit-oss/tantivy) full-text search.

## Credential redaction

Session transcripts often contain pasted keys, tokens echoed by tools, and
`.env` contents. Before text reaches the index, sessfind replaces recognized
credentials with `[REDACTED]`, so search results, `show`, the TUI preview,
JSON output, and excerpts sent to LLM search or project summaries never repeat
them. Native previews read from the source files are redacted the same way.

Recognized formats: private key blocks, AWS access key IDs, GitHub tokens,
OpenAI/Anthropic `sk-` keys, Slack and Stripe live tokens, JWTs, `Bearer` and
`Authorization: Basic` values, passwords in connection URLs (scheme, user and
host are kept), and values assigned to names ending in `password`, `secret`,
`token`, `api_key`, `access_key`, `private_key`, or `account_key` when the
value mixes letters and digits.

Original session files are never modified, so redaction protects what sessfind
shows, not the files on disk. When the rules change, the next run rebuilds the
index. The semantic plugin keeps snippets for chunks it has already embedded;
delete its data directory to rebuild those with redacted text.

## Incremental updates

Each file's `mtime` and size are tracked in a local SQLite database (`state.db`). On subsequent runs, only new or modified session files are re-indexed — making updates fast even with large session histories.

## Search (FTS + Fuzzy)

- **FTS mode** queries the tantivy index with BM25 ranking for relevance-sorted results.
- **Fuzzy mode** does in-memory substring matching on pre-loaded chunks across content, project name, and title.

## Semantic search pipeline

1. The optional `sessfind-semantic` plugin generates vector embeddings for each chunk using the `multilingual-e5-small` model (384 dimensions, via ONNX Runtime).
2. Embeddings are stored in a local `sqlite-vec` database (`semantic.db`).
3. At query time, the input is embedded and compared against stored vectors via cosine similarity.

## LLM search pipeline

1. sessfind detects installed AI CLI tools (`claude`, `opencode`, `copilot`) on `PATH`.
2. The user's natural language query is sent to the selected LLM in headless mode (e.g., `claude -p`).
3. The LLM generates optimized FTS queries (synonyms, related terms, multiple languages).
4. sessfind executes each generated query against the tantivy index and merges the results.

## Resume mechanism

When you select a session and press `Enter`, sessfind replaces the current process (`exec()`) with the appropriate tool's resume command:

| Source | Resume Command |
|--------|----------------|
| GitHub Copilot | `copilot --resume=SESSION_ID` |
| Claude Code | `claude --resume SESSION_ID` |
| OpenCode | `opencode --session SESSION_ID` |
| Cursor | `cursor PROJECT_PATH` |
| Codex | `codex resume SESSION_ID` |

The `exec()` call means the terminal is handed off cleanly to the AI tool — sessfind's process is replaced, not kept running in the background.
